use bci_schema::{
    input::exact_time,
    types::{Channel, ChannelSchema, DType, Occurrence, StreamType, ValueType},
    unit::Unit,
};
use serde_json::{Value, json};

pub(crate) fn encode(stream: &StreamType) -> Value {
    let kind = match stream.value {
        ValueType::Probability(_) => "Probability",
        ValueType::Decision(_) => "Decision",
        ValueType::Vector(..) => "Vec",
        ValueType::Epoch(..) => "Epoch",
        ValueType::Spectrum(..) => "Spectrum",
        ValueType::Covariance(_, _, false) => "Cov",
        ValueType::Covariance(_, _, true) => "SPD",
    };
    let (shape, internal_rate) = match stream.value {
        ValueType::Probability(ref classes) => (vec![classes.len()], None),
        ValueType::Decision(_) => (vec![2], None),
        ValueType::Vector(_, c) => (vec![c], None),
        ValueType::Spectrum(_, c, n, r) => (vec![c, n / 2 + 1], Some(r.to_string())),
        ValueType::Epoch(_, c, n, r) => (vec![c, n], Some(r.to_string())),
        ValueType::Covariance(_, c, _) => (vec![c, c], None),
    };
    let occurrence = match &stream.occurrence {
        Occurrence::Uniform { rate, phase } => {
            json!({"rate_hz":rate.to_string(),"phase_seconds":phase.to_string()})
        }
        Occurrence::PendingUniform { rate, phase_key } => {
            json!({"rate_hz":rate.to_string(),"phase_key":phase_key})
        }
        Occurrence::Events => json!({"events":true}),
    };
    let spectrum = match stream.value {
        ValueType::Spectrum(_, _, n, r) => {
            json!({"samples":n,"bin_width_hz":r.divide(bci_schema::number::Rational::integer(n as i64)).expect("verified spectrum").to_string(),"normalization":"OneSidedDensity"})
        }
        _ => Value::Null,
    };
    let classes = match &stream.value {
        ValueType::Probability(c) | ValueType::Decision(c) => Some(c),
        _ => None,
    };
    json!({"classes":classes,"spectrum":spectrum,"value_type":stream.value.to_string(),"value_kind":kind,"dtype":"f64","shape":shape,"internal_rate_hz":internal_rate,"schema_name":stream.schema.name,
        "clock":stream.clock,"occurrence":occurrence,"quality":stream.quality,
        "channels":stream.schema.channels.iter().map(|c|json!({"id":c.id,"quantity":c.quantity,"unit":c.unit.to_string(),"unit_dimension":c.unit.dimension,"unit_scale":c.unit.scale.to_string(),"reference":c.reference})).collect::<Vec<_>>()})
}

pub(crate) fn decode(value: &Value) -> Result<StreamType, String> {
    let items = value["channels"]
        .as_array()
        .ok_or("channels 배열을 요구한다.")?;
    if items.is_empty() || items.len() > 65_536 {
        return Err("채널 수는 1..65536 범위를 요구한다.".into());
    }
    let mut ids = std::collections::BTreeSet::new();
    let mut channels = Vec::new();
    for item in items {
        let id = text(item, "id")?.to_string();
        if id.is_empty() || !ids.insert(id.clone()) {
            return Err("기록 채널은 고유한 ID를 요구한다.".into());
        }
        channels.push(Channel {
            id,
            quantity: text(item, "quantity")?.into(),
            unit: decode_unit(item)?,
            reference: text(item, "reference")?.into(),
        });
    }
    let shape = value["shape"]
        .as_array()
        .ok_or("shape 배열을 요구한다.")?
        .iter()
        .map(|v| {
            v.as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .filter(|&n| (1..=65_536).contains(&n))
                .ok_or_else(|| "shape의 크기는 1..65536 범위를 요구한다.".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if shape.first().copied() != Some(channels.len()) {
        return Err("shape와 채널 수를 맞춘다.".into());
    }
    let name = text(value, "value_type")?;
    let tag = value["value_kind"].as_str();
    if tag.is_some() && value["dtype"] != "f64" {
        return Err("기록 dtype은 f64를 요구한다.".into());
    }
    let kind = match shape.as_slice() {
        [n] if tag == Some("Probability") || tag == Some("Decision") => {
            let classes = value["classes"]
                .as_array()
                .ok_or("클래스 순서를 지정한다.")?
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_string)
                        .ok_or("클래스 문자열을 지정한다.".to_string())
                })
                .collect::<Result<Vec<_>, _>>()?;
            if !bci_schema::learning::classes_valid(&classes)
                || *n
                    != if tag == Some("Decision") {
                        2
                    } else {
                        classes.len()
                    }
            {
                return Err("클래스 순서와 확률·판정 layout을 맞춘다.".into());
            }
            if tag == Some("Decision") {
                ValueType::Decision(classes)
            } else {
                ValueType::Probability(classes)
            }
        }
        [c] if tag == Some("Vec") || (tag.is_none() && name == format!("Vec<f64,{c}>")) => {
            ValueType::Vector(DType::F64, *c)
        }
        [c, bins] if tag == Some("Spectrum") => {
            let n = value["spectrum"]["samples"]
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .filter(|n| (1..=bci_schema::spectral::MAX_DFT_SAMPLES).contains(n))
                .ok_or("PSD의 DFT 표본 수를 확인한다.")?;
            let rate = exact_time(text(value, "internal_rate_hz")?)?;
            if !rate.is_positive()
                || *bins != n / 2 + 1
                || value["spectrum"]["normalization"] != "OneSidedDensity"
                || exact_time(text(&value["spectrum"], "bin_width_hz")?)?
                    != rate.divide(bci_schema::number::Rational::integer(n as i64))?
            {
                return Err("PSD bin 수·폭·내부 격자·정규화를 맞춘다.".into());
            }
            ValueType::Spectrum(DType::F64, *c, n, rate)
        }
        [c, n]
            if tag == Some("Epoch")
                || (tag.is_none() && name.starts_with(&format!("Epoch<f64,{c},{n}; grid="))) =>
        {
            let rate = value["internal_rate_hz"]
                .as_str()
                .or_else(|| {
                    name.split_once("; grid=")
                        .and_then(|(_, s)| s.strip_suffix("Hz>"))
                })
                .ok_or("Epoch 내부 격자를 요구한다.")?;
            let rate = exact_time(rate)?;
            if !rate.is_positive() {
                return Err("내부 표본률은 양수를 요구한다.".into());
            }
            ValueType::Epoch(DType::F64, *c, *n, rate)
        }
        [c, n]
            if c == n
                && (tag == Some("Cov") || (tag.is_none() && name == format!("Cov<f64,{c}>"))) =>
        {
            ValueType::Covariance(DType::F64, *c, false)
        }
        [c, n]
            if c == n
                && (tag == Some("SPD") || (tag.is_none() && name == format!("SPD<f64,{c}>"))) =>
        {
            ValueType::Covariance(DType::F64, *c, true)
        }
        _ => return Err("기록의 f64 값 타입과 shape를 맞춘다.".into()),
    };
    let rate = exact_time(text(&value["occurrence"], "rate_hz")?)?;
    if !rate.is_positive() {
        return Err("발생률은 양수를 요구한다.".into());
    }
    let occurrence = if let Some(phase) = value["occurrence"]["phase_seconds"].as_str() {
        Occurrence::Uniform {
            rate,
            phase: exact_time(phase)?,
        }
    } else {
        Occurrence::PendingUniform {
            rate,
            phase_key: text(&value["occurrence"], "phase_key")?.into(),
        }
    };
    let quality = text(value, "quality")?;
    if quality != "StandardQuality" {
        return Err("기록은 StandardQuality 어휘를 요구한다.".into());
    }
    let stream = StreamType {
        value: kind,
        schema: ChannelSchema {
            name: value["schema_name"].as_str().unwrap_or("recorded").into(),
            channels,
        },
        clock: text(value, "clock")?.into(),
        occurrence,
        quality: quality.into(),
    };
    bci_schema::learning::typed_stream(&stream)?;
    Ok(stream)
}

pub(crate) fn decode_unit(value: &Value) -> Result<Unit, String> {
    if let Some(dimensions) = value["unit_dimension"].as_array() {
        if dimensions.len() != 8 {
            return Err("단위 차원은 8개 지수를 요구한다.".into());
        }
        let mut dimension = [0; 8];
        for (to, from) in dimension.iter_mut().zip(dimensions) {
            *to = from
                .as_i64()
                .and_then(|n| i16::try_from(n).ok())
                .ok_or("단위 지수는 i16 범위를 요구한다.")?;
        }
        let scale = exact_time(text(value, "unit_scale")?)?;
        if !scale.is_positive() {
            return Err("단위 배율은 양수를 요구한다.".into());
        }
        return Ok(Unit { dimension, scale });
    }
    let unit = text(value, "unit")?;
    match unit.parse() {
        Ok(unit) => Ok(unit),
        Err(error) => {
            let Some((factor, rest)) = unit.split_once('*') else {
                return Err(error);
            };
            let scale = exact_time(factor)?;
            let mut unit: Unit = rest.parse()?;
            unit.scale = unit.scale.multiply(scale)?;
            if !unit.scale.is_positive() {
                return Err("단위 배율은 양수를 요구한다.".into());
            }
            Ok(unit)
        }
    }
}

pub(crate) fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value[key]
        .as_str()
        .ok_or_else(|| format!("{key} 문자열을 요구한다."))
}
