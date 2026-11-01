//! 집계·bootstrap protocol과 평가 분석의 JSON codec.
use crate::learning::{array, assignment_json, fields, integer, number, read, text};
use bci_schema::evaluation::{Analysis, Bootstrap, Interval, Metrics, Protocol, Unit};
use serde_json::{Value, json};
use std::path::Path;

fn unit(s: &str) -> Result<Unit, String> {
    match s {
        "Subject" => Ok(Unit::Subject),
        "Session" => Ok(Unit::Session),
        "Trial" => Ok(Unit::Trial),
        _ => Err("E0703: 평가 단위는 Subject·Session·Trial을 요구한다.".into()),
    }
}
/// 고정 집계·난수·percentile 계약과 단위 순서를 읽는다.
pub fn load_protocol(path: &Path) -> Result<Protocol, String> {
    let v = read(path, &["bci-evaluation-protocol/0.1"])?;
    fields(
        &v,
        &["format", "aggregation", "weighting", "quality", "bootstrap"],
    )?;
    if v["weighting"] != "EqualChildMass"
        || v["quality"] != "RequireAll"
        || v.get("bootstrap").is_none()
    {
        return Err("E0703: 평가 protocol에 EqualChildMass·RequireAll과 bootstrap 객체 또는 null을 지정한다.".into());
    }
    let aggregation = array(&v, "aggregation", 3)?
        .iter()
        .map(|u| unit(u.as_str().ok_or("E0703: 집계 단위 문자열을 지정한다.")?))
        .collect::<Result<_, _>>()?;
    let bootstrap = if v["bootstrap"].is_null() {
        None
    } else {
        let b = &v["bootstrap"];
        fields(
            b,
            &[
                "unit",
                "iterations",
                "confidence",
                "seed",
                "rng",
                "interval",
                "undefined",
            ],
        )?;
        if b["rng"] != "SplitMix64/0.1"
            || b["interval"] != "PercentileLinear"
            || b["undefined"] != "Exclude"
        {
            return Err(
                "E0703: bootstrap 계약은 SplitMix64/0.1·PercentileLinear·Exclude를 요구한다."
                    .into(),
            );
        }
        Some(Bootstrap {
            unit: unit(text(b, "unit")?)?,
            iterations: usize::try_from(integer(b, "iterations")?).map_err(|e| e.to_string())?,
            confidence: number(b, "confidence")?,
            seed: integer(b, "seed")?,
        })
    };
    let protocol = Protocol {
        aggregation,
        bootstrap,
    };
    protocol.validate("trial")?;
    Ok(protocol)
}
/// 재계산에 필요한 모든 protocol 매개변수를 출력한다.
pub fn protocol_json(p: &Protocol) -> Value {
    json!({"format":"bci-evaluation-protocol/0.1","aggregation":p.aggregation.iter().map(|u|format!("{u:?}")).collect::<Vec<_>>(),
        "weighting":"EqualChildMass","quality":"RequireAll",
        "bootstrap":p.bootstrap.as_ref().map(|b|json!({"unit":format!("{:?}",b.unit),"iterations":b.iterations,"confidence":b.confidence,
            "seed":b.seed,"rng":"SplitMix64/0.1","interval":"PercentileLinear","undefined":"Exclude"}))})
}
fn metrics(m: &Metrics) -> Value {
    json!({"normalized_confusion_label_major":m.confusion,"class_mass":m.denominators,"accuracy":m.accuracy,"balanced_accuracy":m.balanced_accuracy,
        "missing_classes":m.denominators.iter().enumerate().filter_map(|(i,&d)|if d==0.0 {Some(i)} else {None}).collect::<Vec<_>>()})
}
fn interval(i: &Interval, attempts: usize) -> Value {
    json!({"lower":i.lower,"upper":i.upper,"defined":i.defined,"undefined":attempts-i.defined})
}
pub(crate) fn analysis_json(a: &Analysis) -> Value {
    json!({"protocol":protocol_json(&a.protocol),"protocol_id":a.protocol.digest(),"eval_digest":a.eval_digest,
        "catalog":a.catalog.iter().map(|r|json!({"assignment":assignment_json(&r.assignment),"hierarchy":r.hierarchy,"included":r.included})).collect::<Vec<_>>(),
        "groups":a.groups.iter().map(|g|json!({"unit":format!("{:?}",g.unit),"key":g.key,"included":g.included,"excluded":g.excluded,"children":g.children,"metrics":metrics(&g.metrics)})).collect::<Vec<_>>(),
        "estimate":metrics(&a.estimate),
        "bootstrap":a.resampling.as_ref().map(|b|json!({"clusters":b.clusters.iter().map(|c|json!({"key":c.key,"mass":c.mass})).collect::<Vec<_>>(),
            "accuracy_interval":interval(&b.accuracy_interval,b.accuracy.len()),"balanced_accuracy_interval":interval(&b.balanced_accuracy_interval,b.balanced_accuracy.len()),
            "replicates":b.accuracy.iter().zip(&b.balanced_accuracy).enumerate().map(|(i,(acc,balanced))|json!({"iteration":i,"accuracy":acc,"balanced_accuracy":balanced})).collect::<Vec<_>>()}))})
}
