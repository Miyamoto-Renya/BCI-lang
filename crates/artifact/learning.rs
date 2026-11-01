//! 특징 catalog·분할·모델·평가의 판본별 JSON codec.
use bci_schema::{
    learning::{
        Ancestor, Assignment, Dataset, Evaluation, MAX_ROWS, Model, PROFILE, Role, Sample, Training,
    },
    selection::{Selection, metric},
    types::{Channel, ChannelSchema},
};
use serde_json::{Value, json};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

pub(crate) fn read(path: &Path, formats: &[&str]) -> Result<Value, String> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|e| format!("E0902: {}: {e}", path.display()))?
        .take(16_777_217)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 16_777_216 {
        return Err("E0703: 학습 artifact 상한은 16 MiB다.".into());
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|e| format!("E0703: {e}"))?;
    if !formats.iter().any(|format| value["format"] == *format) {
        return Err(format!(
            "E0703: artifact format은 {formats:?} 중 하나를 요구한다."
        ));
    }
    Ok(value)
}
pub(crate) fn fields(value: &Value, allowed: &[&str]) -> Result<(), String> {
    let map = value.as_object().ok_or("E0703: JSON 객체를 지정한다.")?;
    if let Some(key) = map.keys().find(|k| !allowed.contains(&k.as_str())) {
        return Err(format!(
            "E0703: 등록한 학습 artifact 필드를 사용한다: {key}"
        ));
    }
    Ok(())
}
pub(crate) fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value[key]
        .as_str()
        .ok_or_else(|| format!("E0703: {key} 문자열을 지정한다."))
}
pub(crate) fn array<'a>(value: &'a Value, key: &str, max: usize) -> Result<&'a [Value], String> {
    value[key]
        .as_array()
        .filter(|a| a.len() <= max)
        .map(Vec::as_slice)
        .ok_or_else(|| format!("E0703: {key} 배열 상한은 {max}개다."))
}
fn strings(value: &Value, key: &str, max: usize) -> Result<Vec<String>, String> {
    array(value, key, max)?
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("E0703: {key} 문자열 배열을 지정한다."))
        })
        .collect()
}
pub(crate) fn number(value: &Value, key: &str) -> Result<f64, String> {
    value[key]
        .as_f64()
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("E0703: {key} 유한 수치를 지정한다."))
}
pub(crate) fn integer(value: &Value, key: &str) -> Result<u64, String> {
    value[key]
        .as_u64()
        .ok_or_else(|| format!("E0703: {key} 0 이상의 정수를 지정한다."))
}
fn training(value: &Value) -> Result<Training, String> {
    fields(value, &["iterations", "learning_rate", "l2"])?;
    let training = Training {
        iterations: usize::try_from(integer(value, "iterations")?).map_err(|e| e.to_string())?,
        learning_rate: number(value, "learning_rate")?,
        l2: number(value, "l2")?,
    };
    training.validate()?;
    Ok(training)
}
fn role(value: &Value) -> Result<Role, String> {
    match text(value, "role")? {
        "Fit" => Ok(Role::Fit),
        "Eval" => Ok(Role::Eval),
        "Tune" => Ok(Role::Tune),
        _ => Err("E0702: batch 학습 profile은 Fit·Tune·Eval 역할을 요구한다.".into()),
    }
}
fn ancestors(value: &Value) -> Result<Vec<Ancestor>, String> {
    array(value, "ancestors", 64)?
        .iter()
        .map(|v| {
            fields(v, &["source_id", "epoch", "first", "last"])?;
            Ok(Ancestor {
                source: text(v, "source_id")?.into(),
                epoch: integer(v, "epoch")?,
                first: integer(v, "first")?,
                last: integer(v, "last")?,
            })
        })
        .collect()
}
fn assignment(value: &Value) -> Result<Assignment, String> {
    fields(value, &["id", "group", "role", "ancestors"])?;
    Ok(Assignment {
        id: text(value, "id")?.into(),
        group: strings(value, "group", 3)?,
        role: role(value)?,
        ancestors: ancestors(value)?,
    })
}
pub(crate) fn assignment_json(value: &Assignment) -> Value {
    json!({"id":value.id,"group":value.group,"role":format!("{:?}",value.role),"ancestors":value.ancestors.iter().map(|a|json!({"source_id":a.source,"epoch":a.epoch,"first":a.first,"last":a.last})).collect::<Vec<_>>()})
}
/// 상대 schema 경로를 dataset 위치에서 해석하고 행을 고유 ID 순서로 정렬한다.
pub fn load_dataset(path: &Path) -> Result<Dataset, String> {
    let v = read(path, &["bci-dataset/0.1"])?;
    fields(
        &v,
        &[
            "format",
            "dataset_id",
            "schema",
            "classes",
            "group_by",
            "training",
            "rows",
        ],
    )?;
    let features = crate::descriptor::load_schema(
        &path
            .parent()
            .unwrap_or(Path::new("."))
            .join(text(&v, "schema")?),
    )?;
    let classes = strings(&v, "classes", 32)?;
    let group_by = text(&v, "group_by")?.to_string();
    let depth = match group_by.as_str() {
        "subject" => 1,
        "session" => 2,
        "trial" => 3,
        _ => return Err("E0701: group_by는 subject·session·trial을 요구한다.".into()),
    };
    let mut rows = Vec::new();
    for r in array(&v, "rows", MAX_ROWS)? {
        fields(
            r,
            &[
                "id",
                "subject",
                "session",
                "trial",
                "role",
                "ancestors",
                "label",
                "values",
                "valid",
            ],
        )?;
        let label = text(r, "label")?;
        let label = classes
            .iter()
            .position(|s| s == label)
            .ok_or_else(|| format!("E0703: 클래스 {label}을 classes 순서에 등록한다."))?;
        let values = array(r, "values", 256)?
            .iter()
            .map(|n| {
                n.as_f64()
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| "E0702: 특징은 유한 f64를 요구한다.".to_string())
            })
            .collect::<Result<_, _>>()?;
        let valid = array(r, "valid", 256)?
            .iter()
            .map(|n| {
                n.as_u64()
                    .filter(|&n| n <= 1)
                    .map(|v| v as u8)
                    .ok_or_else(|| "E0702: valid는 0 또는 1을 요구한다.".to_string())
            })
            .collect::<Result<_, _>>()?;
        let group: Vec<String> = ["subject", "session", "trial"]
            .iter()
            .take(depth)
            .map(|key| text(r, key).map(str::to_string))
            .collect::<Result<_, _>>()?;
        let mut hierarchy = group.clone();
        for key in ["subject", "session", "trial"].iter().skip(depth) {
            if r.get(*key).is_none() {
                break;
            }
            hierarchy.push(text(r, key)?.into());
        }
        rows.push(Sample {
            hierarchy,
            assignment: Assignment {
                id: text(r, "id")?.into(),
                group,
                role: role(r)?,
                ancestors: ancestors(r)?,
            },
            label,
            values,
            valid,
        });
    }
    rows.sort_by(|a, b| a.assignment.id.cmp(&b.assignment.id));
    let data = Dataset {
        id: text(&v, "dataset_id")?.into(),
        features,
        classes,
        group_by,
        training: training(&v["training"])?,
        rows,
    };
    data.validate()?;
    Ok(data)
}
fn features_json(schema: &ChannelSchema) -> Value {
    json!({"name":schema.name,"channels":schema.channels.iter().map(|c|json!({"id":c.id,"quantity":c.quantity,"reference":c.reference,"unit":c.unit.to_string(),"unit_dimension":c.unit.dimension,"unit_scale":c.unit.scale.to_string()})).collect::<Vec<_>>()})
}
fn features(value: &Value) -> Result<ChannelSchema, String> {
    fields(value, &["name", "channels"])?;
    let channels = array(value, "channels", 256)?
        .iter()
        .map(|c| {
            fields(
                c,
                &[
                    "id",
                    "quantity",
                    "reference",
                    "unit",
                    "unit_dimension",
                    "unit_scale",
                ],
            )?;
            Ok(Channel {
                id: text(c, "id")?.into(),
                quantity: text(c, "quantity")?.into(),
                reference: text(c, "reference")?.into(),
                unit: crate::stream_codec::decode_unit(c)?,
            })
        })
        .collect::<Result<_, String>>()?;
    Ok(ChannelSchema {
        name: text(value, "name")?.into(),
        channels,
    })
}
fn bits(values: &[f64]) -> Vec<String> {
    values
        .iter()
        .map(|v| format!("{:016x}", v.to_bits()))
        .collect()
}
fn decode_bits(value: &Value, key: &str, max: usize) -> Result<Vec<f64>, String> {
    strings(value, key, max)?
        .iter()
        .map(|s| {
            if s.len() != 16 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("E0703: f64 bit 표기는 16자리 hex를 요구한다.".into());
            }
            let n = f64::from_bits(u64::from_str_radix(s, 16).map_err(|e| e.to_string())?);
            if !n.is_finite() {
                return Err("E0703: 모델 수치는 유한 f64를 요구한다.".into());
            }
            Ok(n)
        })
        .collect()
}
/// 모델의 수치 bit·입력·클래스·학습 계보를 함께 직렬화한다.
pub fn model_json(model: &Model) -> Value {
    json!({"format":"bci-model/0.2","selection":model.selection.as_deref().map(selection_json),"profile":PROFILE,"model_id":model.digest(),"features":features_json(&model.features),"classes":model.classes,
        "mean_bits":bits(&model.mean),"scale_bits":bits(&model.scale),"weight_bits":bits(&model.weights),"bias_bits":bits(&model.bias),
        "training":{"iterations":model.training.iterations,"learning_rate_bits":format!("{:016x}",model.training.learning_rate.to_bits()),"l2_bits":format!("{:016x}",model.training.l2.to_bits())},
        "dataset_id":model.dataset,"group_by":model.group_by,"split":model.split.iter().map(assignment_json).collect::<Vec<_>>(),"fit_rows":model.fit_rows,"fit_digest":model.fit_digest,
        "numeric_contract":{"dtype":"f64","normalization":"FitPopulationStdConstantOne","probability":"Softmax","calibration":"Uncalibrated","quality":"RequireAll","initial_weights":"Zero","rng":"None"}})
}
/// 내용 digest와 역할 계보를 검증한 immutable 모델을 읽는다.
pub fn load_model(path: &Path) -> Result<Model, String> {
    let v = read(path, &["bci-model/0.1", "bci-model/0.2"])?;
    decode_model(&v, true)
}
fn decode_model(v: &Value, allow_selection: bool) -> Result<Model, String> {
    if v["format"] != "bci-model/0.1" && v["format"] != "bci-model/0.2" {
        return Err("E0703: 모델 format은 bci-model/0.1 또는 bci-model/0.2를 요구한다.".into());
    }
    if (v["format"] == "bci-model/0.1" && v.get("selection").is_some())
        || (v["format"] == "bci-model/0.2" && v.get("selection").is_none())
        || (!allow_selection && !v["selection"].is_null())
    {
        return Err("E0703: 0.2의 selection에는 null 또는 단일 선택 증거를 지정하고 후보에는 직접 학습 모델을 지정한다.".into());
    }
    fields(
        v,
        &[
            "format",
            "profile",
            "model_id",
            "features",
            "classes",
            "mean_bits",
            "scale_bits",
            "weight_bits",
            "bias_bits",
            "training",
            "dataset_id",
            "group_by",
            "split",
            "fit_rows",
            "fit_digest",
            "numeric_contract",
            "selection",
        ],
    )?;
    if v["profile"] != PROFILE {
        return Err("E0703: 지원하는 softmax profile 판본을 지정한다.".into());
    }
    fields(
        &v["training"],
        &["iterations", "learning_rate_bits", "l2_bits"],
    )?;
    let solver_number = |key: &str| -> Result<f64, String> {
        let encoded = text(&v["training"], key)?;
        let wrapper = json!({"bits":[encoded]});
        Ok(decode_bits(&wrapper, "bits", 1)?[0])
    };
    let model = Model {
        features: features(&v["features"])?,
        classes: strings(v, "classes", 32)?,
        mean: decode_bits(v, "mean_bits", 256)?,
        scale: decode_bits(v, "scale_bits", 256)?,
        weights: decode_bits(v, "weight_bits", 8192)?,
        bias: decode_bits(v, "bias_bits", 32)?,
        training: Training {
            iterations: usize::try_from(integer(&v["training"], "iterations")?)
                .map_err(|e| e.to_string())?,
            learning_rate: solver_number("learning_rate_bits")?,
            l2: solver_number("l2_bits")?,
        },
        dataset: text(v, "dataset_id")?.into(),
        group_by: text(v, "group_by")?.into(),
        split: array(v, "split", MAX_ROWS)?
            .iter()
            .map(assignment)
            .collect::<Result<_, _>>()?,
        fit_rows: strings(v, "fit_rows", MAX_ROWS)?,
        fit_digest: text(v, "fit_digest")?.into(),
        selection: if v["selection"].is_null() {
            None
        } else {
            Some(Box::new(selection(&v["selection"])?))
        },
    };
    model.validate()?;
    if v["model_id"] != model.digest()
        || v["numeric_contract"] != model_json(&model)["numeric_contract"]
    {
        return Err("E0703: 모델 digest·수치 계약을 저장한 내용과 맞춘다.".into());
    }
    Ok(model)
}
/// 전체 후보 모델과 Tune 선택 근거를 직렬화한다.
pub fn selection_json(s: &Selection) -> Value {
    json!({"format":"bci-selection/0.1","metric":"BalancedAccuracy","tie_break":"FirstCandidate",
        "quality":"RequireAll","aggregation":"Sample","dataset_id":s.dataset,
        "tune_digest":s.tune_digest,"tune_rows":s.tune_rows,
        "tune":s.tune.iter().map(assignment_json).collect::<Vec<_>>(),
        "candidates":s.candidates.iter().map(model_json).collect::<Vec<_>>(),
        "confusion_label_major":s.confusion,"winner":s.winner,
        "balanced_accuracy":s.confusion.iter().map(|c|metric(c,s.candidates[0].classes.len()).map(|(_,m)|m).unwrap_or(f64::NAN)).collect::<Vec<_>>()})
}
fn selection(v: &Value) -> Result<Selection, String> {
    fields(
        v,
        &[
            "format",
            "metric",
            "tie_break",
            "quality",
            "aggregation",
            "dataset_id",
            "tune_digest",
            "tune_rows",
            "tune",
            "candidates",
            "confusion_label_major",
            "winner",
            "balanced_accuracy",
        ],
    )?;
    if v["format"] != "bci-selection/0.1"
        || v["metric"] != "BalancedAccuracy"
        || v["tie_break"] != "FirstCandidate"
        || v["quality"] != "RequireAll"
        || v["aggregation"] != "Sample"
    {
        return Err(
            "E0703: 선택 계약은 Sample·RequireAll·BalancedAccuracy·FirstCandidate를 요구한다."
                .into(),
        );
    }
    let candidates = array(v, "candidates", 16)?
        .iter()
        .map(|m| decode_model(m, false))
        .collect::<Result<Vec<_>, _>>()?;
    if candidates.len() < 2 {
        return Err("E0703: 선택 후보는 2..16개를 요구한다.".into());
    }
    let confusion = array(v, "confusion_label_major", 16)?
        .iter()
        .map(|c| {
            c.as_array()
                .filter(|a| a.len() <= 1024)
                .ok_or("E0703: confusion은 최대 1024개 정수 배열을 요구한다.")?
                .iter()
                .map(|n| {
                    n.as_u64()
                        .ok_or_else(|| "E0703: confusion count는 0 이상 정수를 요구한다.".into())
                })
                .collect::<Result<Vec<_>, String>>()
        })
        .collect::<Result<Vec<_>, _>>()?;
    let result = Selection {
        dataset: text(v, "dataset_id")?.into(),
        tune_digest: text(v, "tune_digest")?.into(),
        tune_rows: strings(v, "tune_rows", MAX_ROWS)?,
        tune: array(v, "tune", MAX_ROWS)?
            .iter()
            .map(assignment)
            .collect::<Result<_, _>>()?,
        candidates,
        confusion,
        winner: usize::try_from(integer(v, "winner")?).map_err(|e| e.to_string())?,
    };
    let expected = result
        .confusion
        .iter()
        .map(|c| metric(c, result.candidates[0].classes.len()).map(|(_, score)| score))
        .collect::<Result<Vec<_>, _>>()?;
    if v["balanced_accuracy"] != json!(expected) {
        return Err("E0703: balanced accuracy를 confusion 분모와 맞춘다.".into());
    }
    Ok(result)
}

/// 평가 지표·분모·제외 목록과 예측/라벨 연결을 구성한다.
pub fn evaluation_json(e: &Evaluation) -> Value {
    let mut value = json!({"format":"bci-evaluation/0.1","model_id":e.model_digest,"dataset_id":e.dataset,"classes":e.classes,"quality":"RequireAll","aggregation":"Sample",
        "confusion_label_major":e.confusion,"accuracy":e.accuracy,"balanced_accuracy":e.balanced_accuracy,"included":e.predictions.len(),"excluded":e.excluded,
        "predictions":e.predictions.iter().map(|p|json!({"id":p.id,"label":p.label,"predicted":p.predicted,"probabilities":p.probabilities})).collect::<Vec<_>>(),
        "lineage":e.lineage.iter().map(assignment_json).collect::<Vec<_>>()});
    if let Some(analysis) = &e.analysis {
        value["format"] = json!("bci-evaluation/0.2");
        value["analysis"] = crate::evaluation::analysis_json(analysis);
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        hash.update(b"bci-evaluation-report/0.2\0");
        hash.update(serde_json::to_vec(&value).expect("finite evaluation JSON"));
        value["report_id"] = json!(format!("{:x}", hash.finalize()));
    }
    value
}
/// 새 경로에 완전한 JSON을 원자적으로 게시하고 sync한다.
pub fn write_new(path: &Path, value: &Value) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|e| format!("E0902: {e}"))?;
    bytes.push(b'\n');
    if bytes.len() > 16_777_216 {
        return Err("E0703: 학습 artifact 상한은 16 MiB다.".into());
    }
    use std::sync::atomic::{AtomicU64, Ordering};
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let pending = parent.join(format!(
        ".bci-learning-{}-{}.tmp",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&pending)
        .map_err(|e| format!("E0902: {}: {e}", pending.display()))?;
    let result = (|| -> Result<(), String> {
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| format!("E0902: {e}"))?;
        std::fs::hard_link(&pending, path).map_err(|e| {
            format!(
                "E0902: {}: 새 artifact 경로를 지정한다: {e}",
                path.display()
            )
        })?;
        #[cfg(unix)]
        File::open(parent)
            .and_then(|f| f.sync_all())
            .map_err(|e| format!("E0902: {e}"))?;
        Ok(())
    })();
    drop(file);
    let _ = std::fs::remove_file(pending);
    result
}

/// 각 JSON의 크기를 확인하고 새 결과 디렉터리에 학습 artifact를 게시한다.
pub fn write_results(output: &Path, files: &[(&str, Value)]) -> Result<(), String> {
    for (_, value) in files {
        let bytes = serde_json::to_vec_pretty(value).map_err(|e| format!("E0703: {e}"))?;
        if bytes.len() >= 16_777_216 {
            return Err("E0703: 학습 artifact 상한은 16 MiB다.".into());
        }
    }
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| format!("E0902: {e}"))?;
    }
    std::fs::create_dir(output).map_err(|e| {
        format!(
            "E0902: {}: 새 학습 결과 디렉터리를 지정한다: {e}",
            output.display()
        )
    })?;
    for (name, value) in files {
        write_new(&output.join(name), value)?;
    }
    Ok(())
}
