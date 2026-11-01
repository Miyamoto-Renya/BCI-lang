//! Rust 연산자 등록 속성과 wrapper 생성을 담당하는 proc-macro 패키지다.
//!
//! 생성 코드는 bci-extension의 SDK 경로를 참조한다.
//! 등록 속성의 계약과 생성 범위는 상세 설계 13에 정의한다.

use proc_macro::TokenStream;
use quote::{ToTokens, format_ident, quote};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io::Read, path::Path};
use syn::{ItemFn, LitStr, MetaNameValue, Token, parse::Parser, punctuated::Punctuated};
mod stateful;

/// f64 → f64 함수에 계약·reference와 `<함수>_registration` 생성자를 연결한다.
#[proc_macro_attribute]
pub fn operator(attributes: TokenStream, item: TokenStream) -> TokenStream {
    let result = if syn::parse::<syn::ItemImpl>(item.clone()).is_ok() {
        stateful::expand(attributes, item)
    } else {
        expand(attributes, item)
    };
    match result {
        Ok(output) => output,
        Err(error) => error.to_compile_error().into(),
    }
}

fn expand(attributes: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let attrs = Punctuated::<MetaNameValue, Token![,]>::parse_terminated.parse(attributes)?;
    let function = syn::parse::<ItemFn>(item)?;
    let error = |message: &str| syn::Error::new_spanned(&function.sig, message);
    let mut args = BTreeMap::new();
    for attr in attrs {
        let key = attr
            .path
            .get_ident()
            .ok_or_else(|| error("속성 이름을 지정한다."))?
            .to_string();
        let syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(value),
            ..
        }) = attr.value
        else {
            return Err(error("operator 속성 값은 문자열로 지정한다."));
        };
        if !["name", "version", "kind", "contract"].contains(&key.as_str())
            || args.insert(key, value).is_some()
        {
            return Err(error(
                "operator는 name·version·kind·contract를 각각 한 번 요구한다.",
            ));
        }
    }
    let arg = |name: &str| {
        args.get(name)
            .map(LitStr::value)
            .ok_or_else(|| error(&format!("operator의 {name}을 지정한다.")))
    };
    let name = arg("name")?;
    let version = arg("version")?;
    if arg("kind")? != "pointwise" {
        return Err(error("현재 매크로의 kind는 pointwise를 요구한다."));
    }
    let sig = &function.sig;
    let f64_type = |ty: &syn::Type| matches!(ty, syn::Type::Path(p) if p.qself.is_none() && p.path.is_ident("f64"));
    if sig.asyncness.is_some()
        || !matches!(sig.safety, syn::Safety::Default)
        || sig.abi.is_some()
        || sig.variadic.is_some()
        || !sig.generics.params.is_empty()
        || sig.generics.where_clause.is_some()
        || sig.inputs.len() != 1
        || !matches!(sig.inputs.first(), Some(syn::FnArg::Typed(p)) if f64_type(&p.ty))
        || !matches!(&sig.output, syn::ReturnType::Type(_, ty) if f64_type(ty))
    {
        return Err(error(
            "pointwise 함수 서명은 fn(value: f64) -> f64를 요구한다.",
        ));
    }
    let base = std::env::var("CARGO_MANIFEST_DIR").map_err(|e| error(&e.to_string()))?;
    let contract_path = Path::new(&base).join(arg("contract")?);
    let contract = read(&contract_path).map_err(|e| error(&e))?;
    let table: toml::Table = contract
        .parse()
        .map_err(|e| error(&format!("계약 TOML: {e}")))?;
    keys(
        &table,
        &[
            "contract_version",
            "extends",
            "signature",
            "numeric",
            "resources",
        ],
    )
    .map_err(|e| error(&e))?;
    let expect = |table: &toml::Table, key: &str, value: &str| -> syn::Result<()> {
        if table.get(key).and_then(toml::Value::as_str) == Some(value) {
            Ok(())
        } else {
            Err(error(&format!("계약의 {key}는 {value}를 요구한다.")))
        }
    };
    expect(&table, "contract_version", "bci-operator/0.1")?;
    expect(&table, "extends", "std.contracts.pointwise_f64/0.1")?;
    let section = |key: &str| {
        table
            .get(key)
            .and_then(toml::Value::as_table)
            .ok_or_else(|| error(&format!("계약의 [{key}]를 지정한다.")))
    };
    let signature = section("signature")?;
    keys(signature, &["quantity", "schema", "unit"]).map_err(|e| error(&e))?;
    expect(signature, "schema", "preserve")?;
    expect(signature, "unit", "preserve")?;
    let quantity = signature
        .get("quantity")
        .and_then(toml::Value::as_str)
        .ok_or_else(|| error("signature.quantity 문자열을 지정한다."))?;
    let numeric = section("numeric")?;
    keys(numeric, &["input_domain", "reference", "comparison"]).map_err(|e| error(&e))?;
    expect(numeric, "input_domain", "finite_f64")?;
    expect(numeric, "comparison", "exact_bits")?;
    let reference_path = numeric
        .get("reference")
        .and_then(toml::Value::as_str)
        .ok_or_else(|| error("numeric.reference 파일을 지정한다."))?;
    let reference_path = contract_path
        .parent()
        .expect("contract parent")
        .join(reference_path);
    let reference = read(&reference_path).map_err(|e| error(&e))?;
    let pairs: Vec<[String; 2]> =
        serde_json::from_str(&reference).map_err(|e| error(&format!("reference JSON: {e}")))?;
    if pairs.is_empty() || pairs.len() > 4096 {
        return Err(error("reference는 1..4096개 f64 bit 쌍을 요구한다."));
    }
    let mut bits = Vec::new();
    for pair in pairs {
        let parse = |s: &str| -> syn::Result<u64> {
            let value = u64::from_str_radix(s, 16).map_err(|e| error(&e.to_string()))?;
            if s.len() != 16 || !f64::from_bits(value).is_finite() {
                return Err(error("reference bit는 유한 f64의 16자리 hex로 지정한다."));
            }
            Ok(value)
        };
        let input = parse(&pair[0])?;
        let output = parse(&pair[1])?;
        bits.push(quote! { (#input, #output) });
    }
    let resources = section("resources")?;
    keys(resources, &["state_bytes", "scratch_bytes"]).map_err(|e| error(&e))?;
    for key in ["state_bytes", "scratch_bytes"] {
        if resources.get(key).and_then(toml::Value::as_integer) != Some(0) {
            return Err(error(
                "pointwise의 state_bytes·scratch_bytes는 0을 요구한다.",
            ));
        }
    }
    let hash = |s: &str| format!("{:x}", Sha256::digest(s.as_bytes()));
    let contract_digest = hash(&contract);
    let reference_digest = hash(&reference);
    let function_digest = hash(&function.to_token_stream().to_string());
    let path = contract_path
        .to_str()
        .ok_or_else(|| error("계약 경로에 UTF-8을 요구한다."))?;
    let reference_path = reference_path
        .to_str()
        .ok_or_else(|| error("reference 경로에 UTF-8을 요구한다."))?;
    let ident = &sig.ident;
    let registration = format_ident!("{}_registration", ident);
    let visibility = &function.vis;
    let cfg = function.attrs.iter().filter(|a| a.path().is_ident("cfg"));
    Ok(quote! {
        #function
        #(#cfg)*
        #[doc = "BCi pointwise 계약·함수·exact-bit reference 등록을 생성한다."]
        #visibility fn #registration() -> ::bci_extension::PointwiseRegistration {
            const _: &str = include_str!(#path);
            const _: &str = include_str!(#reference_path);
            ::bci_extension::PointwiseRegistration {
                spec: ::bci_extension::OperatorSpec {
                    state: None,
                    name: #name.into(), version: #version.into(),
                    profile: ::bci_extension::POINTWISE_PROFILE.into(), quantity: #quantity.into(),
                    contract_digest: #contract_digest.into(), function_digest: #function_digest.into(),
                    reference_digest: #reference_digest.into(),
                    rust_symbol: concat!(module_path!(), "::", stringify!(#ident)).into(),
                    rust_source: format!("{}:{}:{}", file!(), line!(), column!()),
                },
                function: #ident,
                reference: &[#(#bits),*],
            }
        }
    }.into())
}

fn keys(table: &toml::Table, allowed: &[&str]) -> Result<(), String> {
    if table.len() != allowed.len() || table.keys().any(|k| !allowed.contains(&k.as_str())) {
        return Err(format!(
            "계약 필드는 {}를 정확히 지정한다.",
            allowed.join(", ")
        ));
    }
    Ok(())
}

fn read(path: &Path) -> Result<String, String> {
    let mut value = String::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(1_048_577).read_to_string(&mut value))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if value.len() > 1_048_576 {
        return Err(format!(
            "{}: 계약·reference 파일 상한은 1 MiB다.",
            path.display()
        ));
    }
    Ok(value)
}
