//! 소스 검사가 소비하는 스키마·계수·시계 매핑 기술자.

use bci_schema::{
    number::Rational,
    types::{Channel, ChannelSchema},
};
use serde_json::Value;
use std::{collections::BTreeSet, io::Read, path::Path};

fn read(path: &Path, format: &str) -> Result<Value, String> {
    let file = std::fs::File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(1_048_577)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > 1_048_576 {
        return Err("기술자 크기는 1 MiB 이하여야 한다.".into());
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    if value.get("format").and_then(Value::as_str) != Some(format) {
        return Err(format!("기술자 format은 {format}을 요구한다."));
    }
    Ok(value)
}

fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("문자열 필드 {key}를 지정한다."))
}

fn fields(value: &Value, allowed: &[&str]) -> Result<(), String> {
    let object = value.as_object().ok_or("기술자는 JSON 객체를 요구한다.")?;
    if let Some(name) = object.keys().find(|name| !allowed.contains(&name.as_str())) {
        return Err(format!(
            "현재 기술자 판본의 필드를 사용한다: {name}. 허용 필드: {}",
            allowed.join(", ")
        ));
    }
    Ok(())
}

/// 순서 있는 채널과 명목 측정량·단위·기준을 읽는다.
pub fn load_schema(path: &Path) -> Result<ChannelSchema, String> {
    let value = read(path, "bci-schema/0.1")?;
    fields(&value, &["format", "name", "channels"])?;
    let name = text(&value, "name")?.to_string();
    let items = value
        .get("channels")
        .and_then(Value::as_array)
        .ok_or("channels 배열을 지정한다.")?;
    if items.is_empty() || items.len() > 65_536 {
        return Err("채널 수는 1..65536 범위를 요구한다.".into());
    }
    let mut ids = BTreeSet::new();
    let mut channels = Vec::new();
    for item in items {
        fields(item, &["id", "quantity", "unit", "reference"])?;
        let id = text(item, "id")?.to_string();
        if id.is_empty() || !ids.insert(id.clone()) {
            return Err("채널 id는 고유한 비어 있지 않은 문자열을 요구한다.".into());
        }
        channels.push(Channel {
            id,
            quantity: text(item, "quantity")?.into(),
            unit: text(item, "unit")?.parse()?,
            reference: text(item, "reference")?.into(),
        });
    }
    Ok(ChannelSchema { name, channels })
}

/// 정확한 십진 문자열로 작성한 무차원 FIR 계수를 읽는다.
pub fn load_coefficients(path: &Path) -> Result<Vec<Rational>, String> {
    let value = read(path, "bci-coefficients/0.1")?;
    fields(&value, &["format", "values"])?;
    let items = value
        .get("values")
        .and_then(Value::as_array)
        .ok_or("values 배열을 지정한다.")?;
    if items.is_empty() || items.len() > 65_536 {
        return Err("계수 수는 1..65536 범위를 요구한다.".into());
    }
    items
        .iter()
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| "계수는 정확한 십진 문자열로 작성한다.".to_string())?
                .parse()
        })
        .collect()
}

/// 매핑 양 끝의 명목 clock ID를 읽는다. 시간 증거 검사는 bind 의무에 속한다.
pub fn load_mapping(path: &Path) -> Result<MappingDescriptor, String> {
    let value = read(path, "bci-clock-mapping/0.1")?;
    fields(&value, &["format", "from", "to", "slope", "offset_seconds"])?;
    let slope: Rational = text(&value, "slope")?.parse()?;
    if !slope.is_positive() {
        return Err("mapping slope는 양수를 요구한다.".into());
    }
    Ok(MappingDescriptor {
        from: text(&value, "from")?.into(),
        to: text(&value, "to")?.into(),
        slope,
        offset: text(&value, "offset_seconds")?.parse()?,
    })
}

/// 양쪽 시계와 명목 affine 변환의 구성 기술자.
#[derive(Debug, Clone)]
pub struct MappingDescriptor {
    /// 입력 clock ID.
    pub from: String,
    /// 출력 clock ID.
    pub to: String,
    /// 출력 초 / 입력 초 배율.
    pub slope: Rational,
    /// 출력 초 단위 offset.
    pub offset: Rational,
}

/// 정확한 십진 문자열의 SOS를 a0=1로 정규화하고 안정성을 검사한다.
pub fn load_sos(path: &Path) -> Result<bci_schema::iir::Realization, String> {
    let value = read(path, "bci-sos/0.1")?;
    fields(
        &value,
        &["format", "sample_rate_hz", "warmup_samples", "sections"],
    )?;
    let rate = text(&value, "sample_rate_hz")?.parse()?;
    let warmup = value["warmup_samples"]
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or("warmup_samples는 정수를 요구한다.")?;
    let rows = value["sections"]
        .as_array()
        .ok_or("SOS sections 배열을 지정한다.")?;
    if !(1..=64).contains(&rows.len()) {
        return Err("SOS 절 수는 1..64 범위를 요구한다.".into());
    }
    let mut sections = Vec::new();
    for row in rows {
        let values = row
            .as_array()
            .filter(|v| v.len() == 6)
            .ok_or("SOS 각 절은 b0,b1,b2,a0,a1,a2의 6개 계수를 요구한다.")?;
        let numbers = values
            .iter()
            .map(|v| {
                v.as_str()
                    .ok_or_else(|| "SOS 계수는 십진 문자열을 요구한다.".to_string())?
                    .parse::<Rational>()
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut section = [Rational::ZERO; 5];
        for (to, from) in section.iter_mut().zip([0, 1, 2, 4, 5]) {
            *to = numbers[from].divide(numbers[3])?;
        }
        sections.push(section);
    }
    let result = bci_schema::iir::Realization {
        rate,
        warmup,
        sections,
    };
    result.validate()?;
    Ok(result)
}
