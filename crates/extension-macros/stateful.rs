use proc_macro::TokenStream;
use quote::{ToTokens, quote};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};
use syn::{MetaNameValue, Token, parse::Parser, punctuated::Punctuated};

pub(crate) fn expand(attributes: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let implementation = syn::parse::<syn::ItemImpl>(item)?;
    let error = |message: &str| syn::Error::new_spanned(&implementation.self_ty, message);
    if implementation.trait_.as_ref().is_none_or(|(p, _)| {
        p.segments
            .last()
            .is_none_or(|s| s.ident != "StatefulOperator")
    }) || !implementation.generics.params.is_empty()
        || implementation.generics.where_clause.is_some()
    {
        return Err(error(
            "구체 타입의 impl StatefulOperator에 stateful 등록을 붙인다.",
        ));
    }
    let mut args = BTreeMap::new();
    for attr in Punctuated::<MetaNameValue, Token![,]>::parse_terminated.parse(attributes)? {
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
            return Err(error("속성 값은 문자열을 요구한다."));
        };
        if !["name", "version", "kind", "contract"].contains(&key.as_str())
            || args.insert(key, value.value()).is_some()
        {
            return Err(error("name·version·kind·contract를 각각 한 번 지정한다."));
        }
    }
    let arg = |key: &str| {
        args.get(key)
            .ok_or_else(|| error(&format!("{key}를 지정한다.")))
    };
    let name = arg("name")?;
    let version = arg("version")?;
    if arg("kind")? != "stateful" {
        return Err(error("상태 impl의 kind는 stateful을 요구한다."));
    }
    let base = std::env::var("CARGO_MANIFEST_DIR").map_err(|e| error(&e.to_string()))?;
    let path = Path::new(&base).join(arg("contract")?);
    let text = crate::read(&path).map_err(|e| error(&e))?;
    let table: toml::Table = text
        .parse()
        .map_err(|e| error(&format!("계약 TOML: {e}")))?;
    crate::keys(
        &table,
        &[
            "contract_version",
            "extends",
            "signature",
            "numeric",
            "resources",
            "state",
        ],
    )
    .map_err(|e| error(&e))?;
    let get = |table: &toml::Table, key: &str| -> syn::Result<String> {
        table
            .get(key)
            .and_then(toml::Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| error(&format!("{key} 문자열을 지정한다.")))
    };
    if get(&table, "contract_version")? != "bci-operator/0.1"
        || get(&table, "extends")? != "std.contracts.stateful_f64/0.1"
    {
        return Err(error("stateful_f64/0.1 계약 판본을 요구한다."));
    }
    let section = |key: &str| {
        table
            .get(key)
            .and_then(toml::Value::as_table)
            .ok_or_else(|| error(&format!("[{key}]를 지정한다.")))
    };
    let signature = section("signature")?;
    crate::keys(signature, &["quantity", "schema", "unit"]).map_err(|e| error(&e))?;
    let quantity = get(signature, "quantity")?;
    if get(signature, "schema")? != "preserve" || get(signature, "unit")? != "preserve" {
        return Err(error("stateful profile은 schema·unit preserve를 요구한다."));
    }
    let numeric = section("numeric")?;
    crate::keys(numeric, &["input_domain", "reference", "comparison"]).map_err(|e| error(&e))?;
    if get(numeric, "input_domain")? != "finite_f64" || get(numeric, "comparison")? != "exact_bits"
    {
        return Err(error("finite_f64·exact_bits 수치 계약을 요구한다."));
    }
    let reference_path = path
        .parent()
        .expect("parent")
        .join(get(numeric, "reference")?);
    let reference = crate::read(&reference_path).map_err(|e| error(&e))?;
    let pairs: Vec<[String; 2]> =
        serde_json::from_str(&reference).map_err(|e| error(&e.to_string()))?;
    if pairs.is_empty() || pairs.len() > 4096 {
        return Err(error("reference는 1..4096쌍을 요구한다."));
    }
    let mut bits = Vec::new();
    for pair in pairs {
        let parse = |s: &str| -> syn::Result<u64> {
            let bits = u64::from_str_radix(s, 16).map_err(|e| error(&e.to_string()))?;
            if s.len() != 16
                || !s.bytes().all(|c| c.is_ascii_hexdigit())
                || !f64::from_bits(bits).is_finite()
            {
                return Err(error("유한 f64의 16자리 hex를 요구한다."));
            }
            Ok(bits)
        };
        let a = parse(&pair[0])?;
        let b = parse(&pair[1])?;
        bits.push(quote! {(#a,#b)});
    }
    let resources = section("resources")?;
    crate::keys(
        resources,
        &[
            "fixed_bytes",
            "channel_bytes",
            "scratch_bytes",
            "checkpoint_bytes",
            "checkpoint_channel_bytes",
        ],
    )
    .map_err(|e| error(&e))?;
    let size = |key: &str| -> syn::Result<usize> {
        resources
            .get(key)
            .and_then(toml::Value::as_integer)
            .and_then(|v| usize::try_from(v).ok())
            .filter(|&v| v <= 64 * 1024 * 1024)
            .ok_or_else(|| error(&format!("{key}는 0..64 MiB 정수를 요구한다.")))
    };
    let fixed = size("fixed_bytes")?;
    let channel = size("channel_bytes")?;
    let scratch = size("scratch_bytes")?;
    let checkpoint = size("checkpoint_bytes")?;
    let checkpoint_channel = size("checkpoint_channel_bytes")?;
    let state = section("state")?;
    crate::keys(state, &["codec"]).map_err(|e| error(&e))?;
    let codec = get(state, "codec")?;
    let hash = |s: &str| format!("{:x}", Sha256::digest(s.as_bytes()));
    let contract_hash = hash(&text);
    let function_hash = hash(&implementation.to_token_stream().to_string());
    let reference_hash = hash(&reference);
    let path = path
        .to_str()
        .ok_or_else(|| error("계약 경로는 UTF-8을 요구한다."))?;
    let reference_path = reference_path
        .to_str()
        .ok_or_else(|| error("reference 경로는 UTF-8을 요구한다."))?;
    let ty = &implementation.self_ty;
    Ok(quote! {
        #implementation
        impl #ty {
            #[doc="BCi 상태 계약·factory·수치 참조 registration을 생성한다."]
            pub fn bci_registration() -> ::bci_extension::StatefulRegistration {
                const _: &str=include_str!(#path); const _: &str=include_str!(#reference_path);
                ::bci_extension::StatefulRegistration {
                    spec: ::bci_extension::OperatorSpec {
                        state: Some(::bci_extension::StateContract { codec:#codec.into(),fixed_bytes:#fixed,channel_bytes:#channel,scratch_bytes:#scratch,checkpoint_bytes:#checkpoint,checkpoint_channel_bytes:#checkpoint_channel }),
                        name:#name.into(),version:#version.into(),profile: ::bci_extension::STATEFUL_PROFILE.into(),quantity:#quantity.into(),
                        contract_digest:#contract_hash.into(),function_digest:#function_hash.into(),reference_digest:#reference_hash.into(),
                        rust_symbol:concat!(module_path!(),"::",stringify!(#ty)).into(),rust_source:format!("{}:{}:{}",file!(),line!(),column!()),
                    },
                    create: ::bci_extension::StatefulRegistration::create::<#ty>,
                    verify: |spec| ::bci_extension::StatefulRegistration::verify::<#ty>(spec,&[#(#bits),*]),
                }
            }
        }
    }.into())
}
