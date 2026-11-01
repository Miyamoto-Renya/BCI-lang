//! 공통 예제의 실제 기록을 수학적 참조와 비교하고 실행 경계 실패를 확인한다.

use bci_runtime::{ExecutionPlan, Frame, Recorder, RunReport, SessionState};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

const EXAMPLE: &str = include_str!("../../examples/typed.bci");
static SERIAL: AtomicU64 = AtomicU64::new(0);
mod lab_signal;
mod lab_state;
struct Temporary(PathBuf);
impl Temporary {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "bci-runtime-test-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn examples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}
fn manifest(directory: &Path) -> Value {
    serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap()
}
fn single_run(root: &Path) -> PathBuf {
    let files: Vec<_> = fs::read_dir(root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(files.len(), 1);
    files[0].clone()
}
fn source_at(root: &Path, source: &str) -> PathBuf {
    let mut source = source.to_string();
    for name in ["eeg.schema.json", "smooth.coefficients.json", "samples.csv"] {
        source = source.replace(name, examples().join(name).to_str().unwrap());
    }
    let path = root.join("main.bci");
    fs::write(&path, source).unwrap();
    path
}

fn cli_eof(source: &Path, output: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_bci"))
        .arg("run")
        .arg(source)
        .arg("--output")
        .arg(output)
        .output()
        .unwrap()
}
fn data(directory: &Path) -> Vec<Value> {
    let path = fs::read_dir(directory)
        .unwrap()
        .map(|p| p.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .unwrap();
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .skip(1)
        .map(|s| serde_json::from_str(s).unwrap())
        .collect()
}

struct FileRecorder(bci_artifact::recording::RunWriter);
impl Recorder for FileRecorder {
    fn prepare(&mut self, plan: &ExecutionPlan) -> Result<(), String> {
        self.0.prepare(plan)
    }
    fn write(&mut self, sink: usize, port: usize, frame: &Frame) -> Result<(), String> {
        self.0.write(sink, port, frame)
    }
    fn finish(&mut self, report: &RunReport) -> Result<(), String> {
        self.0.finish(report)
    }
}

#[test]
fn grouped_evaluation_bootstrap_protocol_and_cli() {
    use bci_artifact::{
        evaluation::{load_protocol, protocol_json},
        learning::{evaluation_json, load_dataset, load_model, model_json, write_new},
    };
    use bci_operators::evaluation::evaluate;
    use bci_schema::{
        evaluation::{Protocol, Unit},
        learning::Role,
    };
    let temp = Temporary::new();
    let model = load_model(&examples().join("learning.model.json")).unwrap();
    let dataset = load_dataset(&examples().join("evaluation.dataset.json")).unwrap();
    let protocol = load_protocol(&examples().join("evaluation.protocol.json")).unwrap();
    let report = evaluate(&model, &dataset, &protocol).unwrap();
    assert_eq!(report.confusion, [3, 3, 3, 3]);
    assert_eq!((report.predictions.len(), report.excluded.len()), (12, 2));
    assert_eq!(report.balanced_accuracy, 0.5);
    let analysis = report.analysis.as_ref().unwrap();
    // A는 세션 (정답률 1, 0)을 동일 가중하고 B의 정답률은 0이다.
    assert_eq!(analysis.estimate.confusion, [0.125, 0.375, 0.375, 0.125]);
    assert_eq!(analysis.estimate.balanced_accuracy, Some(0.25));
    let subjects: Vec<_> = analysis
        .groups
        .iter()
        .filter(|g| g.unit == Unit::Subject)
        .collect();
    assert_eq!((subjects[0].included, subjects[0].children), (10, 2));
    assert_eq!(
        (
            subjects[2].included,
            subjects[2].excluded,
            subjects[2].children
        ),
        (0, 2, 0)
    );
    assert_eq!(subjects[2].metrics.balanced_accuracy, None);
    let bootstrap = analysis.resampling.as_ref().unwrap();
    assert_eq!(bootstrap.clusters.len(), 2);
    assert_eq!(
        (
            bootstrap.balanced_accuracy_interval.lower,
            bootstrap.balanced_accuracy_interval.upper
        ),
        (0.0, 0.5)
    );
    // 두 피험자를 두 번 복원 추출하면 AA, AB, BA, BB의 세 지표 값이 나온다.
    assert!(
        bootstrap
            .balanced_accuracy
            .iter()
            .all(|v| [Some(0.0), Some(0.25), Some(0.5)].contains(v))
    );
    assert_eq!(bootstrap.balanced_accuracy_interval.defined, 1000);
    assert_eq!(
        evaluation_json(&report),
        evaluation_json(&evaluate(&model, &dataset, &protocol).unwrap())
    );
    let mut different = protocol.clone();
    different.bootstrap.as_mut().unwrap().seed = 99;
    assert_ne!(
        bootstrap.balanced_accuracy,
        evaluate(&model, &dataset, &different)
            .unwrap()
            .analysis
            .unwrap()
            .resampling
            .unwrap()
            .balanced_accuracy
    );
    let direct = Protocol {
        aggregation: vec![Unit::Subject],
        bootstrap: None,
    };
    let direct_score = evaluate(&model, &dataset, &direct)
        .unwrap()
        .analysis
        .unwrap()
        .estimate
        .balanced_accuracy
        .unwrap();
    assert!((direct_score - 0.3).abs() < 1e-14);
    let sample = evaluate(&model, &dataset, &Protocol::default()).unwrap();
    assert_eq!(
        sample.analysis.unwrap().estimate.balanced_accuracy,
        Some(report.balanced_accuracy)
    );

    // label이 하나인 cluster를 재표집한 반복의 결측 클래스와 지표 분모를 기록한다.
    let mut incomplete = dataset.clone();
    incomplete.rows.truncate(2);
    for (i, row) in incomplete.rows.iter_mut().enumerate() {
        row.assignment.group = vec![format!("class-{i}")];
        row.hierarchy[0] = row.assignment.group[0].clone();
        row.values = vec![-2.0, -0.4];
    }
    let partial = evaluate(&model, &incomplete, &protocol)
        .unwrap()
        .analysis
        .unwrap();
    assert_eq!(partial.estimate.balanced_accuracy, Some(0.5));
    let resampling = partial.resampling.unwrap();
    let defined = resampling.balanced_accuracy.iter().flatten().count();
    assert!(defined > 2 && defined < 1000);
    assert_eq!(resampling.balanced_accuracy_interval.defined, defined);
    assert_eq!(resampling.accuracy_interval.defined, 1000);
    assert_eq!(
        (
            resampling.balanced_accuracy_interval.lower,
            resampling.balanced_accuracy_interval.upper
        ),
        (0.5, 0.5)
    );
    let mut one_cluster = dataset.clone();
    one_cluster
        .rows
        .retain(|r| r.assignment.group[0] == "subject-a");
    assert!(
        evaluate(&model, &one_cluster, &protocol)
            .unwrap_err()
            .contains("cluster 2개")
    );
    let mut missing = dataset.clone();
    missing.rows[0].hierarchy.truncate(1);
    assert!(
        evaluate(&model, &missing, &protocol)
            .unwrap_err()
            .contains("계층 key")
    );
    let mut bad = protocol.clone();
    bad.aggregation = vec![Unit::Subject, Unit::Trial];
    assert!(evaluate(&model, &dataset, &bad).is_err());
    bad = protocol.clone();
    bad.bootstrap.as_mut().unwrap().unit = Unit::Trial;
    assert!(evaluate(&model, &dataset, &bad).is_err());
    let mut leakage = dataset.clone();
    leakage.rows[0].assignment.ancestors = model.split[0].ancestors.clone();
    assert!(
        evaluate(&model, &leakage, &protocol)
            .unwrap_err()
            .contains("SplitOverlap")
    );
    let protocol_path = temp.0.join("protocol.json");
    write_new(&protocol_path, &protocol_json(&protocol)).unwrap();
    assert_eq!(load_protocol(&protocol_path).unwrap(), protocol);
    let mut damaged = protocol_json(&protocol);
    damaged["bootstrap"]["rng"] = serde_json::json!("Unknown");
    let bad_path = temp.0.join("bad-protocol.json");
    write_new(&bad_path, &damaged).unwrap();
    assert!(load_protocol(&bad_path).is_err());

    let command = |args: &[&str]| {
        let result = Command::new(env!("CARGO_BIN_EXE_bci"))
            .args(args)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    };
    let output = temp.0.join("evaluation.json");
    command(&[
        "learn",
        "evaluate",
        examples().join("learning.model.json").to_str().unwrap(),
        examples().join("evaluation.dataset.json").to_str().unwrap(),
        "--protocol",
        protocol_path.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
    ]);
    let actual: Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    assert_eq!(actual, evaluation_json(&report));
    assert_eq!(actual["format"], "bci-evaluation/0.2");
    assert_eq!(actual["report_id"].as_str().unwrap().len(), 64);
    let denied = temp.0.join("denied.json");
    let error = Command::new(env!("CARGO_BIN_EXE_bci"))
        .args([
            "learn",
            "evaluate",
            examples().join("learning.model.json").to_str().unwrap(),
            examples().join("evaluation.dataset.json").to_str().unwrap(),
            "--output",
            denied.to_str().unwrap(),
            "--protocol",
            bad_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!error.status.success());
    assert!(!denied.exists());

    let mut combined: Value =
        serde_json::from_str(include_str!("../../examples/learning.dataset.json")).unwrap();
    let eval_data: Value =
        serde_json::from_str(include_str!("../../examples/evaluation.dataset.json")).unwrap();
    combined["schema"] = serde_json::json!(examples().join("learning.schema.json"));
    combined["rows"]
        .as_array_mut()
        .unwrap()
        .retain(|r| r["role"] == "Fit");
    combined["rows"]
        .as_array_mut()
        .unwrap()
        .extend(eval_data["rows"].as_array().unwrap().clone());
    let combined_path = temp.0.join("combined.json");
    write_new(&combined_path, &combined).unwrap();
    let fitted = temp.0.join("fit");
    command(&[
        "learn",
        "fit",
        combined_path.to_str().unwrap(),
        "--output",
        fitted.to_str().unwrap(),
        "--protocol",
        protocol_path.to_str().unwrap(),
    ]);
    let fitted_model = load_model(&fitted.join("model.json")).unwrap();
    let mut tuning: Value =
        serde_json::from_str(include_str!("../../examples/selection.dataset.json")).unwrap();
    tuning["rows"]
        .as_array_mut()
        .unwrap()
        .retain(|r| r["role"] == "Tune");
    combined["rows"]
        .as_array_mut()
        .unwrap()
        .extend(tuning["rows"].as_array().unwrap().clone());
    let selection_data_path = temp.0.join("selection-data.json");
    write_new(&selection_data_path, &combined).unwrap();
    let chosen = temp.0.join("chosen");
    command(&[
        "learn",
        "select",
        selection_data_path.to_str().unwrap(),
        "--candidate",
        examples().join("learning.model.json").to_str().unwrap(),
        "--candidate",
        fitted.join("model.json").to_str().unwrap(),
        "--protocol",
        protocol_path.to_str().unwrap(),
        "--output",
        chosen.to_str().unwrap(),
    ]);
    let selected = load_model(&chosen.join("model.json")).unwrap();
    let selection_data = load_dataset(&selection_data_path).unwrap();
    assert_eq!(
        selected.digest(),
        bci_operators::learning::select(&[model.clone(), fitted_model], &selection_data)
            .unwrap()
            .digest()
    );
    let advanced: Value =
        serde_json::from_slice(&fs::read(chosen.join("evaluation.json")).unwrap()).unwrap();
    assert_eq!(advanced["analysis"]["estimate"]["balanced_accuracy"], 0.25);
    assert_eq!(model_json(&model)["model_id"], model.digest());
    assert!(
        selection_data
            .rows
            .iter()
            .any(|r| r.assignment.role == Role::Tune)
    );
}

#[test]
fn tune_selection_lineage_artifacts_and_execution() {
    use bci_artifact::learning::{load_dataset, load_model, model_json, write_new};
    use bci_operators::learning::{evaluate, fit, select};
    use bci_schema::learning::Role;
    let temp = Temporary::new();
    let dataset = load_dataset(&examples().join("selection.dataset.json")).unwrap();
    let early =
        fit(&load_dataset(&examples().join("selection-early.dataset.json")).unwrap()).unwrap();
    let trained = fit(&dataset).unwrap();
    let candidates = [early.clone(), trained.clone()];
    let selected = select(&candidates, &dataset).unwrap();
    let evidence = selected.selection.as_ref().unwrap();
    assert_eq!(evidence.winner, 1);
    assert_eq!(evidence.confusion, [vec![0, 2, 2, 0], vec![2, 0, 0, 2]]);
    assert_eq!((evidence.tune.len(), evidence.tune_rows.len()), (5, 4));
    assert_eq!(
        evaluate(&selected, &dataset).unwrap().confusion,
        [2, 0, 0, 2]
    );

    let mut changed = dataset.clone();
    for row in changed
        .rows
        .iter_mut()
        .filter(|r| r.assignment.role == Role::Eval)
    {
        row.values.fill(100.0);
        row.label = 1 - row.label;
    }
    assert_eq!(
        select(&candidates, &changed).unwrap().digest(),
        selected.digest()
    );
    assert_eq!(fit(&changed).unwrap().digest(), trained.digest());
    for row in changed
        .rows
        .iter_mut()
        .filter(|r| r.assignment.role == Role::Tune)
    {
        row.label = 1 - row.label;
    }
    let reversed = select(&candidates, &changed).unwrap();
    assert_eq!(reversed.selection.as_ref().unwrap().winner, 0);
    assert_ne!(reversed.digest(), selected.digest());
    for row in changed
        .rows
        .iter_mut()
        .filter(|r| r.assignment.role == Role::Tune && r.label == 1)
    {
        row.valid.fill(0);
    }
    assert!(
        select(&candidates, &changed)
            .unwrap_err()
            .contains("MetricUndefined")
    );

    let mut tied = trained.clone();
    tied.training.iterations = 399;
    for pair in [[trained.clone(), tied.clone()], [tied, trained.clone()]] {
        let choice = select(&pair, &dataset).unwrap();
        assert_eq!(choice.selection.as_ref().unwrap().winner, 0);
        assert_eq!(choice.weights, pair[0].weights);
    }
    assert!(select(&[trained.clone(), trained.clone()], &dataset).is_err());
    assert!(select(&[selected.clone(), early], &dataset).is_err());

    // 탈락 후보만 읽은 Fit 원천도 선택 모델의 Eval 경계에 포함한다.
    let mut loser_data = dataset.clone();
    for row in loser_data
        .rows
        .iter_mut()
        .filter(|r| r.assignment.role == Role::Fit)
    {
        row.assignment.group = vec!["loser-fit".into()];
        row.hierarchy[0] = "loser-fit".into();
        row.assignment.ancestors[0].source = "loser-fit".into();
    }
    loser_data.training.iterations = 1;
    let loser = fit(&loser_data).unwrap();
    let closure = select(&[loser.clone(), trained.clone()], &dataset).unwrap();
    let mut leaked = dataset.clone();
    leaked.rows.retain(|r| r.assignment.role == Role::Eval);
    for (i, row) in leaked.rows.iter_mut().enumerate() {
        row.assignment.group = loser.split[i].group.clone();
        row.hierarchy[0] = row.assignment.group[0].clone();
        row.assignment.ancestors = loser.split[i].ancestors.clone();
    }
    evaluate(&trained, &leaked).unwrap();
    assert!(
        evaluate(&closure, &leaked)
            .unwrap_err()
            .contains("SplitOverlap")
    );
    // Tune의 원천 또는 그룹을 Eval에 재사용하면 분할 검사에서 충돌을 보고한다.
    for by_group in [false, true] {
        let mut leaked = dataset.clone();
        leaked.rows.retain(|r| r.assignment.role == Role::Eval);
        for row in &mut leaked.rows {
            if by_group {
                row.assignment.group = evidence.tune[0].group.clone();
                row.hierarchy[0] = row.assignment.group[0].clone();
            } else {
                row.assignment.ancestors = evidence.tune[0].ancestors.clone();
            }
        }
        assert!(
            evaluate(&selected, &leaked)
                .unwrap_err()
                .contains("SplitOverlap")
        );
    }
    let mut retagged = dataset.clone();
    for row in &mut retagged.rows {
        if row.assignment.role == Role::Eval {
            row.assignment.role = Role::Tune;
        }
    }
    assert!(
        select(&candidates, &retagged)
            .unwrap_err()
            .contains("SplitOverlap")
    );

    let selected_path = temp.0.join("selected.json");
    write_new(&selected_path, &model_json(&selected)).unwrap();
    assert_eq!(load_model(&selected_path).unwrap(), selected);
    for case in 0..5 {
        let mut damaged = model_json(&selected);
        match case {
            0 => damaged["selection"]["winner"] = serde_json::json!(0),
            1 => damaged["selection"]["confusion_label_major"][1][0] = serde_json::json!(3),
            2 => damaged["selection"]["candidates"][0] = model_json(&selected),
            3 => damaged["selection"]["tune_digest"] = serde_json::json!("0".repeat(64)),
            _ => {
                damaged["selection"]["candidates"][0]["weight_bits"][0] =
                    serde_json::json!("0000000000000000")
            }
        }
        let path = temp.0.join(format!("damaged-{case}.json"));
        write_new(&path, &damaged).unwrap();
        assert!(load_model(&path).is_err(), "case {case}");
    }

    let command = |args: &[&str]| {
        let result = Command::new(env!("CARGO_BIN_EXE_bci"))
            .args(args)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    };
    for name in ["selection-early", "selection"] {
        let output = temp.0.join(name);
        command(&[
            "learn",
            "fit",
            examples()
                .join(format!("{name}.dataset.json"))
                .to_str()
                .unwrap(),
            "--output",
            output.to_str().unwrap(),
        ]);
        assert!(output.join("model.json").exists());
        assert!(!output.join("evaluation.json").exists());
    }
    let results = temp.0.join("results");
    command(&[
        "learn",
        "select",
        examples().join("selection.dataset.json").to_str().unwrap(),
        "--candidate",
        temp.0.join("selection-early/model.json").to_str().unwrap(),
        "--candidate",
        temp.0.join("selection/model.json").to_str().unwrap(),
        "--output",
        results.to_str().unwrap(),
    ]);
    assert_eq!(load_model(&results.join("model.json")).unwrap(), selected);
    let report: Value =
        serde_json::from_slice(&fs::read(results.join("selection.json")).unwrap()).unwrap();
    assert_eq!(report["balanced_accuracy"], serde_json::json!([0.0, 1.0]));
    command(&[
        "learn",
        "evaluate",
        results.join("model.json").to_str().unwrap(),
        examples().join("selection.dataset.json").to_str().unwrap(),
        "--output",
        temp.0.join("evaluation.json").to_str().unwrap(),
    ]);
    assert_eq!(
        fs::read(results.join("evaluation.json")).unwrap(),
        fs::read(temp.0.join("evaluation.json")).unwrap()
    );

    let source = include_str!("../../examples/learning.bci")
        .replace("learning.model.json", selected_path.to_str().unwrap())
        .replace(
            "learning.schema.json",
            examples().join("learning.schema.json").to_str().unwrap(),
        )
        .replace(
            "learning.csv",
            examples().join("learning.csv").to_str().unwrap(),
        );
    let source_path = temp.0.join("main.bci");
    fs::write(&source_path, source).unwrap();
    let full = temp.0.join("full");
    command(&[
        "run",
        source_path.to_str().unwrap(),
        "--output",
        full.to_str().unwrap(),
    ]);
    let full_dir = single_run(&full);
    let expected = data(&full_dir);
    assert_eq!(expected.len(), 30);
    let manifest_text = fs::read_to_string(full_dir.join("manifest.json")).unwrap();
    assert!(manifest_text.contains(&selected.digest()));
    assert!(manifest_text.contains(&candidates[0].digest()));
    let checkpoint = temp.0.join("checkpoint.json");
    let head = temp.0.join("head");
    let tail = temp.0.join("tail");
    command(&[
        "run",
        source_path.to_str().unwrap(),
        "--samples",
        "4",
        "--checkpoint-out",
        checkpoint.to_str().unwrap(),
        "--output",
        head.to_str().unwrap(),
    ]);
    command(&[
        "run",
        source_path.to_str().unwrap(),
        "--resume",
        checkpoint.to_str().unwrap(),
        "--output",
        tail.to_str().unwrap(),
    ]);
    let mut joined = data(&single_run(&head));
    joined.extend(data(&single_run(&tail)));
    assert_eq!(joined, expected);
    // 선택 순서까지 포함한 모델 정체성을 checkpoint 계획과 대조한다.
    let other = select(&[trained, candidates[0].clone()], &dataset).unwrap();
    fs::write(
        &selected_path,
        serde_json::to_vec(&model_json(&other)).unwrap(),
    )
    .unwrap();
    let rejected = Command::new(env!("CARGO_BIN_EXE_bci"))
        .args([
            "run",
            source_path.to_str().unwrap(),
            "--resume",
            checkpoint.to_str().unwrap(),
            "--output",
            temp.0.join("wrong").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!rejected.status.success());
}

#[test]
fn learning_roles_model_codec_predictions_decisions_and_recovery() {
    use bci_runtime::{Checkpoint, CheckpointOptions, run_with_checkpoint};
    use bci_schema::learning::Role;
    let temp = Temporary::new();
    let dataset =
        bci_artifact::learning::load_dataset(&examples().join("learning.dataset.json")).unwrap();
    let model = bci_operators::learning::fit(&dataset).unwrap();
    let report = bci_operators::learning::evaluate(&model, &dataset).unwrap();
    assert_eq!(report.confusion, [2, 0, 0, 2]);
    assert_eq!(report.balanced_accuracy, 1.0);
    let mut one_step = dataset.clone();
    one_step.training.iterations = 1;
    let single = bci_operators::learning::fit(&one_step).unwrap();
    for j in 0..2 {
        let rows: Vec<_> = dataset
            .rows
            .iter()
            .filter(|r| r.assignment.role == Role::Fit)
            .collect();
        let mean = rows.iter().map(|r| r.values[j]).sum::<f64>() / rows.len() as f64;
        let scale = (rows
            .iter()
            .map(|r| (r.values[j] - mean).powi(2))
            .sum::<f64>()
            / rows.len() as f64)
            .sqrt();
        assert!((single.mean[j] - mean).abs() < 1e-14);
        assert!((single.scale[j] - scale).abs() < 1e-14);
        for class in 0..2 {
            let gradient = rows
                .iter()
                .map(|r| (0.5 - f64::from(r.label == class)) * (r.values[j] - mean) / scale)
                .sum::<f64>()
                / rows.len() as f64;
            assert!(
                (single.weights[class * 2 + j] + dataset.training.learning_rate * gradient).abs()
                    < 1e-14
            );
        }
    }
    let mut changed = dataset.clone();
    for row in changed
        .rows
        .iter_mut()
        .filter(|r| r.assignment.role == Role::Eval)
    {
        row.values.fill(1e6);
        row.label = 1 - row.label;
    }
    assert_eq!(
        bci_operators::learning::fit(&changed).unwrap().digest(),
        model.digest()
    );
    for case in 0..3 {
        let mut bad = dataset.clone();
        if case == 0 {
            bad.rows[4].assignment.group = bad.rows[0].assignment.group.clone();
        }
        if case == 1 {
            bad.rows[4].assignment.ancestors = bad.rows[0].assignment.ancestors.clone();
        }
        if case == 2 {
            for row in bad
                .rows
                .iter_mut()
                .filter(|r| r.assignment.role == Role::Fit && r.label == 1)
            {
                row.valid.fill(0);
            }
        }
        assert!(bci_operators::learning::fit(&bad).is_err());
    }
    let mut bad_eval = dataset.clone();
    for row in bad_eval
        .rows
        .iter_mut()
        .filter(|r| r.assignment.role == Role::Eval && r.label == 1)
    {
        row.valid.fill(0);
    }
    assert!(
        bci_operators::learning::evaluate(&model, &bad_eval)
            .unwrap_err()
            .contains("MetricUndefined")
    );
    let mut leakage = dataset.clone();
    leakage.rows.retain(|r| r.assignment.role == Role::Eval);
    leakage.rows[0].assignment.group = model.split[0].group.clone();
    leakage.rows[0].hierarchy[0] = model.split[0].group[0].clone();
    assert!(
        bci_operators::learning::evaluate(&model, &leakage)
            .unwrap_err()
            .contains("SplitOverlap")
    );
    let trained = temp.0.join("trained");
    let command = Command::new(env!("CARGO_BIN_EXE_bci"))
        .args(["learn", "fit"])
        .arg(examples().join("learning.dataset.json"))
        .arg("--output")
        .arg(&trained)
        .output()
        .unwrap();
    assert!(
        command.status.success(),
        "{}",
        String::from_utf8_lossy(&command.stderr)
    );
    let loaded = bci_artifact::learning::load_model(&trained.join("model.json")).unwrap();
    assert_eq!(model, loaded);
    assert_eq!(
        bci_artifact::learning::load_model(&examples().join("learning.model.json")).unwrap(),
        loaded
    );
    let mut damaged = bci_artifact::learning::model_json(&loaded);
    damaged["weight_bits"][0] = serde_json::json!("0000000000000000");
    let damaged_path = temp.0.join("damaged.json");
    bci_artifact::learning::write_new(&damaged_path, &damaged).unwrap();
    assert!(bci_artifact::learning::load_model(&damaged_path).is_err());
    assert!(bci_artifact::learning::write_new(&damaged_path, &damaged).is_err());
    let source = include_str!("../../examples/learning.bci").replace(
        "learning.model.json",
        trained.join("model.json").to_str().unwrap(),
    );
    let compile = |text: &str| {
        bci_compiler::compile(text, &examples())
            .unwrap()
            .execution_plan(None, None)
            .unwrap()
    };
    let plan = compile(&source);
    let run = |plan: &ExecutionPlan, name: &str, saved: Option<&Checkpoint>, capture| {
        let root = temp.0.join(name);
        let mut writer = FileRecorder(bci_artifact::recording::RunWriter::new(
            &examples(),
            Some(&root),
        ));
        let (report, snapshot) = run_with_checkpoint(
            plan,
            &bci_operators::StandardKernels,
            &bci_artifact::input::FileSources,
            &mut writer,
            &AtomicBool::new(false),
            CheckpointOptions {
                resume: saved,
                save: capture,
            },
        );
        assert!(report.failure.is_none(), "{:?}", report.failure);
        (single_run(&root), snapshot)
    };
    let (directory, _) = run(&plan, "full", None, false);
    let full = data(&directory);
    assert_eq!(full.len(), 30);
    for raw in full.iter().filter(|r| r["port"] == 0) {
        let probability = full
            .iter()
            .find(|r| r["port"] == 1 && r["sequence"] == raw["sequence"])
            .unwrap();
        let decision = full
            .iter()
            .find(|r| r["port"] == 2 && r["sequence"] == raw["sequence"])
            .unwrap();
        if raw["valid"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!(0))
        {
            assert_eq!(probability["values"], serde_json::json!([0.0, 0.0]));
            assert_eq!(decision["values"], serde_json::json!([-1.0, 0.0]));
            assert_eq!(decision["valid"], serde_json::json!([0, 0]));
            continue;
        }
        let logits: Vec<_> = (0..2)
            .map(|k| {
                model.bias[k]
                    + (0..2)
                        .map(|j| {
                            model.weights[k * 2 + j]
                                * (raw["values"][j].as_f64().unwrap() - model.mean[j])
                                / model.scale[j]
                        })
                        .sum::<f64>()
            })
            .collect();
        let denominator = logits.iter().map(|v| v.exp()).sum::<f64>();
        for (k, logit) in logits.iter().enumerate() {
            assert!(
                (probability["values"][k].as_f64().unwrap() - logit.exp() / denominator).abs()
                    < 1e-12
            );
        }
        let selected = usize::from(logits[1] > logits[0]);
        let confidence = logits[selected].exp() / denominator;
        assert_eq!(
            decision["values"][0].as_f64().unwrap(),
            if confidence >= 0.8 {
                selected as f64
            } else {
                -1.0
            }
        );
    }
    for split in [3, 8] {
        let mut head = plan.clone();
        head.samples = Some(split);
        let (head_dir, saved) = run(&head, &format!("head-{split}"), None, true);
        let (tail_dir, _) = run(&plan, &format!("tail-{split}"), saved.as_ref(), false);
        let mut joined = data(&head_dir);
        joined.extend(data(&tail_dir));
        assert_eq!(joined, full);
    }
    for port in [1, 2] {
        let replay = format!(
            r#"language "bci/0.1"; module replay; use std.source as source; use std.recording as recording; run Main {{let x=source.replay(path:{:?},port:{port});recording.write(x,to:"unused");}}"#,
            directory.join("sink-3.jsonl").to_str().unwrap()
        );
        let (replayed, _) = run(&compile(&replay), &format!("replay-{port}"), None, false);
        let mut expected: Vec<_> = full.iter().filter(|r| r["port"] == port).cloned().collect();
        for row in &mut expected {
            row["port"] = serde_json::json!(0);
        }
        assert_eq!(data(&replayed), expected);
    }
    let (tie, _) = run(
        &compile(&source.replace("minimum: 0.8", "minimum: 0.5")),
        "tie",
        None,
        false,
    );
    assert_eq!(
        data(&tie)
            .iter()
            .find(|r| r["port"] == 2 && r["sequence"] == 3)
            .unwrap()["values"][0],
        0.0
    );
    assert!(
        bci_compiler::compile(&source.replace("minimum: 0.8", "minimum: 1.1"), &examples())
            .is_err()
    );
}

#[test]
fn rational_resampling_queues_reference_replay_and_recovery() {
    use bci_runtime::{Checkpoint, CheckpointOptions, run_with_checkpoint};
    use bci_schema::{input::exact_time, number::Rational};
    const SOURCE: &str = include_str!("../../examples/resample.bci");
    struct Sources;
    impl bci_schema::input::SourceFactory for Sources {
        fn open(
            &self,
            spec: &bci_schema::input::SourceSpec,
        ) -> Result<Box<dyn bci_schema::input::InputSource>, bci_schema::input::InputError>
        {
            if spec.kind == bci_schema::input::SourceKind::Synthetic {
                bci_operators::SyntheticSources.open(spec)
            } else {
                bci_artifact::input::FileSources.open(spec)
            }
        }
    }
    let temp = Temporary::new();
    let compile = |source: &str, samples| {
        bci_compiler::compile(source, &examples())
            .unwrap()
            .execution_plan(None, samples)
            .unwrap()
    };
    let plan = compile(SOURCE, Some(9));
    let run = |plan: &ExecutionPlan, name: &str, saved: Option<&Checkpoint>, capture: bool| {
        let root = temp.0.join(name);
        let mut writer = FileRecorder(bci_artifact::recording::RunWriter::new(
            &examples(),
            Some(&root),
        ));
        let (report, snapshot) = run_with_checkpoint(
            plan,
            &bci_operators::StandardKernels,
            &Sources,
            &mut writer,
            &AtomicBool::new(false),
            CheckpointOptions {
                resume: saved,
                save: capture,
            },
        );
        assert!(report.failure.is_none(), "{:?}", report.failure);
        (single_run(&root), snapshot)
    };
    let (directory, _) = run(&plan, "full", None, false);
    let full = data(&directory);
    assert_eq!(full.len(), 40);
    assert_eq!(manifest(&directory)["report"]["pending_records"], 0);
    // 배치·polyphase 구현과 독립적인 zero-insertion 합성곱 참조.
    let reference = |rows: &[Value],
                     source: u64,
                     target: u64,
                     p: u64,
                     q: u64,
                     offset: u64,
                     h: &[f64],
                     rate: i64| {
        for actual in rows.iter().filter(|r| r["port"] == target) {
            let m = actual["sequence"].as_u64().unwrap();
            assert_eq!(
                exact_time(actual["time_seconds"].as_str().unwrap()).unwrap(),
                Rational::new((m * q + offset) as i128, (rate as u64 * p) as i128).unwrap()
            );
            for c in 0..2 {
                let mut expected = 0.0;
                let mut valid = 1;
                for (k, &weight) in h.iter().enumerate().filter(|(_, w)| **w != 0.0) {
                    let high = m * q + offset;
                    if high < k as u64 {
                        if (k as u64 - high).is_multiple_of(p) {
                            valid = 0;
                        }
                        continue;
                    }
                    if !(high - k as u64).is_multiple_of(p) {
                        continue;
                    }
                    let n = (high - k as u64) / p;
                    if let Some(input) = rows.iter().find(|r| {
                        r["port"] == source
                            && r["sequence"] == n
                            && r["generation"] == actual["generation"]
                    }) {
                        expected += weight * input["values"][c].as_f64().unwrap();
                        valid &= input["valid"][c].as_u64().unwrap();
                    } else {
                        valid = 0;
                    }
                }
                assert!(
                    (actual["values"][c].as_f64().unwrap() - expected).abs() < 1e-12,
                    "{actual}"
                );
                assert_eq!(actual["valid"][c], valid, "{actual}");
            }
        }
    };
    reference(
        &full,
        0,
        1,
        3,
        2,
        0,
        &[0.25, 0.5, 0.75, 0.75, 0.5, 0.25],
        250,
    );
    reference(&full, 1, 2, 2, 3, 0, &[0.5; 4], 375);
    for row in full.iter().filter(|r| r["port"] == 3) {
        for c in 0..2 {
            let input = |port| {
                full.iter()
                    .find(|r| r["port"] == port && r["sequence"] == row["sequence"])
                    .unwrap()
            };
            let expected = if row["valid"][c] == 1 {
                0.5 * (input(0)["values"][c].as_f64().unwrap()
                    + input(2)["values"][c].as_f64().unwrap())
            } else {
                0.0
            };
            assert_eq!(row["values"][c].as_f64().unwrap(), expected);
        }
    }
    for split in [1, 2, 4, 7] {
        let mut head = plan.clone();
        head.samples = Some(split);
        let (head_dir, saved) = run(&head, &format!("head-{split}"), None, true);
        if split == 2 {
            assert_eq!(manifest(&head_dir)["report"]["pending_records"], 1);
        }
        let checkpoint = temp.0.join(format!("at-{split}.json"));
        bci_artifact::checkpoint::save(&checkpoint, &saved.unwrap()).unwrap();
        let saved = bci_artifact::checkpoint::load(&checkpoint).unwrap();
        let mut tail = plan.clone();
        tail.samples = Some(9 - split);
        let (tail_dir, _) = run(&tail, &format!("tail-{split}"), Some(&saved), false);
        let mut combined = data(&head_dir);
        combined.extend(data(&tail_dir));
        assert_eq!(combined, full);
    }
    let replay = format!(
        r#"language "bci/0.1"; module replay;
        use std.source as source; use std.recording as recording;
        run Main {{ let x=source.replay(path:{:?},port:1); recording.write(x,to:"unused"); }}"#,
        directory.join("sink-4.jsonl").to_str().unwrap()
    );
    let replay_plan = compile(&replay, None);
    let mut expected: Vec<_> = full.iter().filter(|r| r["port"] == 1).cloned().collect();
    for row in &mut expected {
        row["port"] = serde_json::json!(0);
    }
    assert_eq!(data(&run(&replay_plan, "replay", None, false).0), expected);
    // 비영 위상·감산·무효 mask·Gap의 경계를 같은 수학적 참조로 확인한다.
    let prefix=SOURCE.split("    let fast =").next().unwrap().replace("source.synthetic(schema: EEG, clock: Session, rate: 250[Hz])","source.csv(path: \"samples.csv\", schema: EEG, clock: Session, rate: 250[Hz], gap: Gap.Reset, invalid: Invalid.Mark)");
    let csv = format!(
        "{prefix} let slow=raw |> signal.resample(up:2,down:3,filter:[0.25,0.5,0.75,0.5],phase:2[ms]); recording.write((raw,slow),to:\"unused\"); }}"
    );
    let csv_plan = compile(&csv, None);
    let (csv_dir, _) = run(&csv_plan, "csv", None, false);
    let csv_full = data(&csv_dir);
    reference(&csv_full, 0, 1, 2, 3, 1, &[0.25, 0.5, 0.75, 0.5], 250);
    let replay_csv = replay.replace(
        directory.join("sink-4.jsonl").to_str().unwrap(),
        csv_dir.join("sink-2.jsonl").to_str().unwrap(),
    );
    let mut csv_expected: Vec<_> = csv_full
        .iter()
        .filter(|r| r["port"] == 1)
        .cloned()
        .collect();
    for row in &mut csv_expected {
        row["port"] = serde_json::json!(0);
    }
    assert_eq!(
        data(&run(&compile(&replay_csv, None), "csv-replay", None, false).0),
        csv_expected
    );

    assert!(
        csv_full
            .iter()
            .any(|r| r["port"] == 1 && r["gap"].is_array())
    );
    let mut head = csv_plan.clone();
    head.samples = Some(12);
    let (head_dir, saved) = run(&head, "csv-head", None, true);
    let (tail_dir, _) = run(&csv_plan, "csv-tail", saved.as_ref(), false);
    let mut joined = data(&head_dir);
    joined.extend(data(&tail_dir));
    assert_eq!(joined, csv_full);
    for (from, to) in [
        ("up: 3, down: 2", "up: 6, down: 4"),
        ("up: 3, down: 2", "up: 0, down: 2"),
        ("phase: 0[s]", "phase: 1[s]"),
        ("phase: 0[s]", "phase: 1[ms]"),
    ] {
        assert!(bci_compiler::compile(&SOURCE.replace(from, to), &examples()).is_err());
    }
    let multiple = include_str!("../../examples/multirate.bci");
    let multiple_plan = compile(multiple, Some(17));
    let (multiple_dir, _) = run(&multiple_plan, "multi-full", None, false);
    let multiple_full = data(&multiple_dir);
    assert_eq!(multiple_full.len(), 48);
    assert_eq!(manifest(&multiple_dir)["input_policy"], "UniformTimeMerge");
    assert_eq!(manifest(&multiple_dir)["report"]["nodes"][0]["records"], 9);
    assert_eq!(manifest(&multiple_dir)["report"]["nodes"][1]["records"], 13);
    let mut head = multiple_plan.clone();
    head.samples = Some(2);
    let (head_dir, saved) = run(&head, "multi-head", None, true);
    assert_eq!(manifest(&head_dir)["report"]["pending_records"], 1);
    let mut tail = multiple_plan.clone();
    tail.samples = Some(15);
    let (tail_dir, _) = run(&tail, "multi-tail", saved.as_ref(), false);
    let mut joined = data(&head_dir);
    joined.extend(data(&tail_dir));
    assert_eq!(joined, multiple_full);
    let mut files = multiple.to_string();
    for (rate, count) in [(250, 9), (375, 13)] {
        let path = temp.0.join(format!("input-{rate}.csv"));
        let rows = (0..count)
            .map(|i| format!("{i}/{rate},{i},{}\n", i * 2))
            .collect::<String>();
        fs::write(&path, format!("time_s,C3[uV],C4[uV]\n{rows}")).unwrap();
        files=files.replace(&format!("source.synthetic(schema: EEG, clock: Session, rate: {rate}[Hz])"),&format!("source.csv(path:{:?},schema: EEG,clock: Session,rate:{rate}[Hz],gap:Gap.Reset,invalid:Invalid.Mark)",path.to_str().unwrap()));
    }
    let file_plan = compile(&files, None);
    let (file_dir, _) = run(&file_plan, "multi-files", None, false);
    assert_eq!(manifest(&file_dir)["report"]["end_of_input"], true);
    assert_eq!(data(&file_dir).len(), 48);
    let mut broken = plan.clone();
    broken.queues.capacities[3][0] += 1;
    struct Discard;
    impl Recorder for Discard {
        fn prepare(&mut self, _: &ExecutionPlan) -> Result<(), String> {
            Ok(())
        }
        fn write(&mut self, _: usize, _: usize, _: &Frame) -> Result<(), String> {
            Ok(())
        }
        fn finish(&mut self, _: &RunReport) -> Result<(), String> {
            Ok(())
        }
    }
    let report = bci_runtime::run(
        &broken,
        &bci_operators::StandardKernels,
        &Sources,
        &mut Discard,
        &AtomicBool::new(false),
    );
    assert_eq!(report.failure.unwrap().code, "E0801");
}

#[test]
fn sos_iir_matches_difference_equations_and_restores_channel_state() {
    use bci_runtime::{Checkpoint, CheckpointOptions, run_with_checkpoint};
    use bci_schema::kernel::KernelSpec;
    const SOURCE: &str = include_str!("../../examples/iir.bci");
    let temp = Temporary::new();
    let plan = bci_compiler::compile(SOURCE, &examples())
        .unwrap()
        .execution_plan(None, None)
        .unwrap();
    let run = |plan: &ExecutionPlan, name: &str, saved: Option<&Checkpoint>, capture: bool| {
        let root = temp.0.join(name);
        let mut writer = FileRecorder(bci_artifact::recording::RunWriter::new(
            &examples(),
            Some(&root),
        ));
        let (report, snapshot) = run_with_checkpoint(
            plan,
            &bci_operators::StandardKernels,
            &bci_artifact::input::FileSources,
            &mut writer,
            &AtomicBool::new(false),
            CheckpointOptions {
                resume: saved,
                save: capture,
            },
        );
        assert!(report.failure.is_none(), "{:?}", report.failure);
        (single_run(&root), snapshot)
    };
    let (full_dir, _) = run(&plan, "full", None, false);
    let full = data(&full_dir);
    assert_eq!(full.len(), 51);
    let mut state = [[[0.0; 4]; 2]; 2];
    let mut counts = [0; 2];
    let mut generation = None;
    let mut first = 0;
    let coefficients = [
        [0.375, 0.0, 0.0, -0.75, 0.125],
        [0.75, -0.75, 0.0, -0.5, 0.0],
    ];
    for raw in full.iter().filter(|r| r["port"] == 0) {
        if generation != raw["generation"].as_u64() {
            generation = raw["generation"].as_u64();
            state = [[[0.0; 4]; 2]; 2];
            counts = [0; 2];
            first = raw["sequence"].as_u64().unwrap();
        }
        let actual = full
            .iter()
            .find(|r| r["port"] == 2 && r["sequence"] == raw["sequence"])
            .unwrap();
        let mut expected = raw.clone();
        expected["port"] = serde_json::json!(2);
        expected["source_range"][0] = serde_json::json!(first);
        let mut flags = raw["flags"].as_u64().unwrap();
        for c in 0..2 {
            let mut value = 0.0;
            if raw["valid"][c] == 0 {
                state[c] = [[0.0; 4]; 2];
                counts[c] = 0;
            } else {
                value = raw["values"][c].as_f64().unwrap() * 1e-6;
                for (section, coef) in state[c].iter_mut().zip(coefficients) {
                    let [x1, x2, y1, y2] = *section;
                    let y =
                        coef[0] * value + coef[1] * x1 + coef[2] * x2 - coef[3] * y1 - coef[4] * y2;
                    *section = [value, x1, y, y1];
                    value = y;
                }
                if counts[c] < 2 {
                    expected["valid"][c] = serde_json::json!(0);
                    flags |= 1;
                    counts[c] += 1;
                }
            }
            assert!((actual["values"][c].as_f64().unwrap() - value).abs() < 1e-18);
        }
        expected["flags"] = serde_json::json!(flags);
        expected["values"] = actual["values"].clone();
        assert_eq!(*actual, expected);
    }
    let mut saved_at_gap = None;
    for split in [1, 7, 12, 17] {
        let mut head_plan = plan.clone();
        head_plan.samples = Some(split);
        let (head, saved) = run(&head_plan, &format!("head-{split}"), None, true);
        let saved = saved.unwrap();
        let (tail, _) = run(&plan, &format!("tail-{split}"), Some(&saved), false);
        let mut combined = data(&head);
        combined.extend(data(&tail));
        assert_eq!(combined, full);
        if split == 12 {
            saved_at_gap = Some(saved);
        }
    }
    let begin = SOURCE.find("source.csv(").unwrap();
    let end = begin + SOURCE[begin..].find(");").unwrap() + 1;
    let replay = SOURCE.replace(
        &SOURCE[begin..end],
        &format!(
            "source.replay(path:{:?},port:0)",
            full_dir.join("sink-6.jsonl").to_str().unwrap()
        ),
    );
    let replay_plan = bci_compiler::compile(&replay, &examples())
        .unwrap()
        .execution_plan(None, None)
        .unwrap();
    assert_eq!(data(&run(&replay_plan, "replay", None, false).0), full);
    let filter_path = temp.0.join("filter.json");
    let original: Value =
        serde_json::from_str(include_str!("../../examples/smooth.sos.json")).unwrap();
    for (index, coefficient, value) in [
        (0, 3, "0"),
        (0, 4, "-2"),
        (0, 5, "1"),
        (0, 5, "-1"),
        (0, 4, "-0.99999999999999"),
    ] {
        let mut filter = original.clone();
        if value == "-0.99999999999999" {
            filter["sections"][0][5] = serde_json::json!("0");
        }
        filter["sections"][index][coefficient] = serde_json::json!(value);
        fs::write(&filter_path, serde_json::to_vec(&filter).unwrap()).unwrap();
        assert!(
            bci_compiler::compile(
                &SOURCE.replace("smooth.sos.json", filter_path.to_str().unwrap()),
                &examples()
            )
            .is_err()
        );
    }
    let mut equivalent = original.clone();
    equivalent["sections"][0] = serde_json::json!(["0.75", "0", "0", "2", "-1.5", "0.25"]);
    fs::write(&filter_path, serde_json::to_vec(&equivalent).unwrap()).unwrap();
    let normalized = bci_artifact::descriptor::load_sos(&filter_path).unwrap();
    let KernelSpec::Iir { realization, .. } = &plan.nodes[2].kernel else {
        unreachable!()
    };
    assert_eq!(normalized.digest(), realization.digest());
    assert!(
        bci_compiler::compile(
            &SOURCE.replace("rate: 250[Hz]", "rate: 125[Hz]"),
            &examples()
        )
        .is_err()
    );
    for mutation in 0..3 {
        let mut broken = plan.clone();
        let mut snapshot = saved_at_gap.clone().unwrap();
        match mutation {
            0 => {
                let KernelSpec::Iir { realization, .. } = &mut broken.nodes[2].kernel else {
                    unreachable!()
                };
                realization.sections[0][4] = bci_schema::number::Rational::ONE;
            }
            1 => {
                snapshot.kernels[2].truncate(10);
            }
            _ => {
                let bytes = &mut snapshot.kernels[2];
                let end = bytes.len();
                bytes[end - 8..].copy_from_slice(&f64::NAN.to_bits().to_le_bytes());
            }
        }
        let stop = AtomicBool::new(false);
        let mut probe = Probe {
            prepared: false,
            written: 0,
            fail_at: None,
            stop_after: None,
            stop: &stop,
            finished: false,
        };
        let (report, saved) = run_with_checkpoint(
            &broken,
            &bci_operators::StandardKernels,
            &bci_artifact::input::FileSources,
            &mut probe,
            &stop,
            CheckpointOptions {
                resume: Some(&snapshot),
                save: true,
            },
        );
        assert_eq!(
            report.failure.unwrap().code,
            if mutation == 0 { "E0801" } else { "E0909" }
        );
        assert!(!probe.prepared && saved.is_none());
    }
}

#[test]
fn spectral_power_normalization_types_and_recovery() {
    use bci_extension::{KernelFactory, Tick};
    use bci_runtime::{Checkpoint, CheckpointOptions, run_with_checkpoint};
    use bci_schema::{
        kernel::KernelSpec,
        number::Rational,
        spectral::{Band, Taper},
    };
    // 닫힌 식의 DC·Nyquist·단일 주파수·Hann 에너지를 대조한다.
    for (samples, taper, expected) in [
        (vec![2.0; 4], Taper::Rectangular, vec![4.0, 0.0, 0.0]),
        (
            vec![3.0, -3.0, 3.0, -3.0],
            Taper::Rectangular,
            vec![0.0, 0.0, 9.0],
        ),
        (
            vec![1.0, 0.0, -1.0, 0.0],
            Taper::Rectangular,
            vec![0.0, 0.5, 0.0],
        ),
        (
            (0..5)
                .map(|n| (std::f64::consts::TAU * 2.0 * n as f64 / 5.0).cos())
                .collect(),
            Taper::Rectangular,
            vec![0.0, 0.0, 0.5],
        ),
        (vec![2.0; 4], Taper::Hann, vec![8.0 / 3.0, 4.0 / 3.0, 0.0]),
        (vec![2.0], Taper::Rectangular, vec![4.0]),
    ] {
        let n = samples.len();
        let rate = Rational::integer(n as i64);
        let mut input = Frame::allocate(n, 1).unwrap();
        input.values = samples;
        input.valid[0] = 1;
        let mut output = Frame::allocate(n / 2 + 1, 1).unwrap();
        let mut kernel = bci_operators::StandardKernels
            .create(&KernelSpec::Psd {
                channels: 1,
                samples: n,
                rate,
                taper,
            })
            .unwrap();
        kernel
            .process(Some(&input), Tick { index: 0 }, &mut output)
            .unwrap();
        for (a, e) in output.values.iter().zip(&expected) {
            assert!((a - e).abs() < 1e-12, "{a} != {e}");
        }
        for include_high in [false, true] {
            let band = Band {
                low: Rational::ZERO,
                high: rate.divide(Rational::integer(2)).unwrap(),
                include_high,
            };
            let mut power = Frame::allocate(1, 1).unwrap();
            let mut kernel = bci_operators::StandardKernels
                .create(&KernelSpec::BandPower {
                    channels: 1,
                    samples: n,
                    rate,
                    band,
                })
                .unwrap();
            kernel
                .process(Some(&output), Tick { index: 0 }, &mut power)
                .unwrap();
            let wanted: f64 = expected
                [..expected.len() - usize::from(!include_high && n.is_multiple_of(2))]
                .iter()
                .sum();
            assert!((power.values[0] - wanted).abs() < 1e-12);
            let mut feature = Frame::allocate(1, 1).unwrap();
            let mut kernel = bci_operators::StandardKernels
                .create(&KernelSpec::LogPower {
                    channels: 1,
                    floor: Rational::ONE,
                    reference: Rational::integer(2),
                    unit: "uV^2".parse().unwrap(),
                })
                .unwrap();
            kernel
                .process(Some(&power), Tick { index: 0 }, &mut feature)
                .unwrap();
            assert!((feature.values[0] - ((wanted + 1.0) / 2.0).ln()).abs() < 1e-12);
        }
    }
    const EXAMPLE: &str = include_str!("../../examples/spectral.bci");
    let temp = Temporary::new();
    let source=EXAMPLE.replace("source.synthetic(schema: EEG, clock: Session, rate: 250[Hz])",
        "source.csv(path: \"samples.csv\", schema: EEG, clock: Session, rate: 250[Hz], gap: Gap.Reset, invalid: Invalid.Mark)")
        .replace("let referenced = raw |> signal.rereference(weights: [0.5, 0.5]);","let referenced = raw;")
        .replace("size: 250, hop: 125","size: 8, hop: 4")
        .replace("band: #{low: 8[Hz], high: 13[Hz]}","band: #{low: 0[Hz], high: 125[Hz]}, include_high: true");
    let compile = |s: &str| {
        bci_compiler::compile(s, &examples())
            .unwrap()
            .execution_plan(None, None)
            .unwrap()
    };
    let plan = compile(&source);
    let run = |plan: &ExecutionPlan, name: &str, saved: Option<&Checkpoint>, capture: bool| {
        let root = temp.0.join(name);
        let mut writer = FileRecorder(bci_artifact::recording::RunWriter::new(
            &examples(),
            Some(&root),
        ));
        let (report, snapshot) = run_with_checkpoint(
            plan,
            &bci_operators::StandardKernels,
            &bci_artifact::input::FileSources,
            &mut writer,
            &AtomicBool::new(false),
            CheckpointOptions {
                resume: saved,
                save: capture,
            },
        );
        assert!(report.failure.is_none(), "{:?}", report.failure);
        (single_run(&root), snapshot)
    };
    let (full_dir, _) = run(&plan, "full", None, false);
    let full = data(&full_dir);
    assert_eq!(full.len(), 31);
    assert_eq!(
        manifest(&full_dir)["nodes"][2]["output"]["shape"],
        serde_json::json!([2, 5])
    );
    assert_eq!(
        manifest(&full_dir)["nodes"][2]["output"]["spectrum"]["bin_width_hz"],
        "125/4"
    );
    assert_eq!(
        plan.nodes[2].output.as_ref().unwrap().schema.channels[0].unit,
        "uV^2/Hz".parse().unwrap()
    );
    assert_eq!(
        plan.nodes[3].output.as_ref().unwrap().schema.channels[0].unit,
        "uV^2".parse().unwrap()
    );
    assert_eq!(
        plan.nodes[4].output.as_ref().unwrap().schema.channels[0].unit,
        bci_schema::unit::Unit::scalar()
    );
    for power in full.iter().filter(|r| r["port"] == 3) {
        let end = power["sequence"].as_u64().unwrap();
        let raw: Vec<_> = full
            .iter()
            .filter(|r| {
                r["port"] == 0
                    && r["sequence"].as_u64().unwrap() >= end - 7
                    && r["sequence"].as_u64().unwrap() <= end
            })
            .collect();
        assert_eq!(raw.len(), 8);
        let spectrum = full
            .iter()
            .find(|r| r["port"] == 2 && r["sequence"] == power["sequence"])
            .unwrap();
        let log = full
            .iter()
            .find(|r| r["port"] == 4 && r["sequence"] == power["sequence"])
            .unwrap();
        for c in 0..2 {
            let valid = raw.iter().all(|r| r["valid"][c] == 1);
            assert_eq!(power["valid"][c], u8::from(valid));
            if valid {
                let (mut energy, mut norm) = (0.0, 0.0);
                for (i, row) in raw.iter().enumerate() {
                    let w = (1.0 - (std::f64::consts::TAU * i as f64 / 8.0).cos()) / 2.0;
                    energy += (w * row["values"][c].as_f64().unwrap()).powi(2);
                    norm += w * w;
                }
                let actual = power["values"][c].as_f64().unwrap();
                assert!((actual - energy / norm).abs() < 1e-10);
                assert!((log["values"][c].as_f64().unwrap() - (actual + 1e-12).ln()).abs() < 1e-12);
            } else {
                for v in &spectrum["values"].as_array().unwrap()[c * 5..(c + 1) * 5] {
                    assert_eq!(v, 0.0);
                }
                assert_eq!(power["values"][c], 0.0);
                assert_eq!(log["values"][c], 0.0);
            }
        }
    }
    let mut head_plan = plan.clone();
    head_plan.samples = Some(5);
    let (head, snapshot) = run(&head_plan, "head", None, true);
    let (tail, _) = run(&plan, "tail", snapshot.as_ref(), false);
    let mut combined = data(&head);
    combined.extend(data(&tail));
    assert_eq!(combined, full);
    let replay_source = format!(
        r#"language "bci/0.1"; module spectral.replay;
        use std.source as source; use std.signal as signal; use std.recording as recording;
        run Main {{ let spectrum=source.replay(path:{:?},port:2);
        let power=spectrum |> signal.bandpower(band:#{{low:0[Hz],high:125[Hz]}},include_high:true);
        let features=power |> signal.log_power(floor:1e-12[uV^2],reference:1[uV^2]);
        recording.write((spectrum,power,features),to:"unused"); }}"#,
        full_dir.join("sink-5.jsonl").to_str().unwrap()
    );
    let replay_plan = compile(&replay_source);
    assert_eq!(replay_plan.nodes[0].output, plan.nodes[2].output);
    let mut expected: Vec<_> = full.iter().filter(|r| r["port"] != 0).cloned().collect();
    for row in &mut expected {
        row["port"] = serde_json::json!(row["port"].as_u64().unwrap() - 2);
    }
    assert_eq!(data(&run(&replay_plan, "replay", None, false).0), expected);
    for (from, to) in [
        ("Taper.Hann", "Warmup.Full"),
        ("size: 8", "size: 4097"),
        ("high: 125[Hz]", "high: 126[Hz]"),
        ("low: 0[Hz], high: 125[Hz]", "low: 1[Hz], high: 2[Hz]"),
        ("floor: 1e-12[uV^2]", "floor: 0[uV^2]"),
        ("reference: 1[uV^2]", "reference: 1[V]"),
        ("include_high: true", "include_high: 1"),
    ] {
        assert!(
            bci_compiler::compile(&source.replace(from, to), &examples()).is_err(),
            "{from}"
        );
    }
    let mut broken = plan.clone();
    let KernelSpec::Psd { rate, .. } = &mut broken.nodes[2].kernel else {
        unreachable!()
    };
    *rate = Rational::integer(500);
    let stop = AtomicBool::new(false);
    let mut probe = Probe {
        prepared: false,
        written: 0,
        fail_at: None,
        stop_after: None,
        stop: &stop,
        finished: false,
    };
    let report = bci_runtime::run(
        &broken,
        &bci_operators::StandardKernels,
        &bci_artifact::input::FileSources,
        &mut probe,
        &stop,
    );
    assert_eq!(report.failure.unwrap().code, "E0801");
    assert!(!probe.prepared);
}

#[test]
fn channel_transforms_preserve_schema_quality_and_recovery() {
    use bci_runtime::{Checkpoint, CheckpointOptions, run_with_checkpoint};
    use bci_schema::kernel::KernelSpec;
    const SOURCE: &str = include_str!("../../examples/channels.bci");
    let temp = Temporary::new();
    let compile = |source: &str| {
        bci_compiler::compile(source, &examples())
            .unwrap()
            .execution_plan(None, None)
            .unwrap()
    };
    let run = |plan: &ExecutionPlan, name: &str, saved: Option<&Checkpoint>, capture: bool| {
        let root = temp.0.join(name);
        let mut writer = FileRecorder(bci_artifact::recording::RunWriter::new(
            &examples(),
            Some(&root),
        ));
        let (report, snapshot) = run_with_checkpoint(
            plan,
            &bci_operators::StandardKernels,
            &bci_artifact::input::FileSources,
            &mut writer,
            &AtomicBool::new(false),
            CheckpointOptions {
                resume: saved,
                save: capture,
            },
        );
        assert!(report.failure.is_none(), "{:?}", report.failure);
        (single_run(&root), snapshot)
    };
    let plan = compile(SOURCE);
    let (full_dir, _) = run(&plan, "full", None, false);
    let full = data(&full_dir);
    assert_eq!(full.len(), 95);
    for raw in full.iter().filter(|r| r["port"] == 0) {
        let at = |port| {
            full.iter()
                .find(|r| r["port"] == port && r["sequence"] == raw["sequence"])
                .unwrap()
        };
        let mut expected = raw.clone();
        expected["port"] = serde_json::json!(1);
        expected["values"] = serde_json::json!([raw["values"][1], raw["values"][0]]);
        expected["valid"] = serde_json::json!([raw["valid"][1], raw["valid"][0]]);
        assert_eq!(at(1), &expected);
        expected["port"] = serde_json::json!(2);
        let valid = u8::from(raw["valid"][0] == 1 && raw["valid"][1] == 1);
        let value = if valid == 1 {
            (raw["values"][1].as_f64().unwrap() - raw["values"][0].as_f64().unwrap()) / 2.0
        } else {
            0.0
        };
        expected["values"] = serde_json::json!([value, if valid == 1 { -value } else { 0.0 }]);
        expected["valid"] = serde_json::json!([valid, valid]);
        assert_eq!(at(2), &expected);
        expected["port"] = serde_json::json!(3);
        expected["values"] = serde_json::json!([expected["values"][1]]);
        expected["valid"] = serde_json::json!([valid]);
        assert_eq!(at(3), &expected);
    }
    let reference = &plan.nodes[2].output.as_ref().unwrap().schema.channels[0].reference;
    assert!(reference.starts_with("rereference:sha256:"));
    assert_eq!(
        plan.nodes[3].output.as_ref().unwrap().schema.channels[0].id,
        "C3"
    );
    assert_eq!(
        manifest(&full_dir)["nodes"][2]["kernel"]["weights"],
        serde_json::json!(["1/2", "1/2"])
    );
    let equivalent = compile(&SOURCE.replace("[0.5, 0.5]", "[1/2, 1/2]"));
    assert_eq!(
        &equivalent.nodes[2].output.as_ref().unwrap().schema.channels[0].reference,
        reference
    );
    let weighted = compile(&SOURCE.replace("[0.5, 0.5]", "[0.25, 0.75]"));
    let reordered = compile(
        &SOURCE
            .replace("[0.5, 0.5]", "[0.75, 0.25]")
            .replace("[\"C4\", \"C3\"]", "[\"C3\", \"C4\"]"),
    );
    assert_eq!(
        weighted.nodes[2].output.as_ref().unwrap().schema.channels[0].reference,
        reordered.nodes[2].output.as_ref().unwrap().schema.channels[0].reference
    );
    assert_ne!(
        &weighted.nodes[2].output.as_ref().unwrap().schema.channels[0].reference,
        reference
    );
    let mut head_plan = plan.clone();
    head_plan.samples = Some(12);
    let (head, snapshot) = run(&head_plan, "head", None, true);
    let snapshot = snapshot.unwrap();
    assert!(snapshot.kernels[1..4].iter().all(Vec::is_empty));
    let (tail, _) = run(&plan, "tail", Some(&snapshot), false);
    let mut combined = data(&head);
    combined.extend(data(&tail));
    assert_eq!(combined, full);
    let begin = SOURCE.find("source.csv(").unwrap();
    let end = begin + SOURCE[begin..].find(");").unwrap() + 1;
    let replay = SOURCE.replace(
        &SOURCE[begin..end],
        &format!(
            "source.replay(path:{:?},port:0)",
            full_dir.join("sink-9.jsonl").to_str().unwrap()
        ),
    );
    assert_eq!(data(&run(&compile(&replay), "replay", None, false).0), full);
    let selected_replay = format!(
        "language \"bci/0.1\"; module replay.channels; use std.source as source; use std.recording as recording; run Main {{ let x=source.replay(path:{:?},port:3); recording.write(x,to:\"unused\"); }}",
        full_dir.join("sink-9.jsonl").to_str().unwrap()
    );
    let replay_plan = compile(&selected_replay);
    assert_eq!(replay_plan.nodes[0].output, plan.nodes[3].output);
    let mut expected: Vec<_> = full.iter().filter(|r| r["port"] == 3).cloned().collect();
    for row in &mut expected {
        row["port"] = serde_json::json!(0);
    }
    assert_eq!(
        data(&run(&replay_plan, "selected-replay", None, false).0),
        expected
    );

    // 기준 전극 자체의 0 행은 무효 원천에도 0·valid를 출력한다.
    let basis = compile(
        &SOURCE
            .replace("[0.5, 0.5]", "[1, 0]")
            .replace("channels: [\"C3\"]", "channels: [\"C4\"]"),
    );
    for row in data(&run(&basis, "basis", None, false).0)
        .iter()
        .filter(|r| r["port"] == 3)
    {
        assert_eq!(row["values"], serde_json::json!([0.0]));
        assert_eq!(row["valid"], serde_json::json!([1]));
        assert_eq!(row["flags"].as_u64().unwrap() & 2, 0);
    }
    let prefix = SOURCE.split("    let ordered").next().unwrap();
    let epoch_source = format!(
        r#"{prefix}
        let epochs = raw |> signal.window(size:4,hop:2,start:0);
        let ordered = epochs |> signal.reorder(channels:["C4","C3"]);
        let selected = ordered |> signal.select(channels:["C4"]);
        let referenced = ordered |> signal.rereference(weights:[0.5,0.5]);
        recording.write((epochs,ordered,selected,referenced),to:"unused"); }}"#
    );
    let epochs = data(&run(&compile(&epoch_source), "epoch", None, false).0);
    assert_eq!(epochs.len(), 36);
    for rows in epochs.chunks_exact(4) {
        let values = rows[0]["values"].as_array().unwrap();
        assert_eq!(
            rows[1]["values"],
            serde_json::json!([&values[4..], &values[..4]].concat())
        );
        assert_eq!(rows[2]["values"], serde_json::json!(values[4..]));
        assert_eq!(rows[2]["valid"], serde_json::json!([rows[0]["valid"][1]]));
        let valid = u8::from(rows[0]["valid"][0] == 1 && rows[0]["valid"][1] == 1);
        let mut expected = vec![0.0; 8];
        if valid == 1 {
            for i in 0..4 {
                expected[i] = (values[4 + i].as_f64().unwrap() - values[i].as_f64().unwrap()) / 2.0;
                expected[4 + i] = -expected[i];
            }
        }
        assert_eq!(rows[3]["values"], serde_json::json!(expected));
        assert_eq!(rows[3]["valid"], serde_json::json!([valid, valid]));
        assert_eq!(
            rows[2]["flags"].as_u64().unwrap() & 2,
            if rows[2]["valid"][0] == 1 { 0 } else { 2 }
        );
        assert_eq!(rows[0]["source_range"], rows[3]["source_range"]);
    }
    for (before, after) in [
        ("[0.5, 0.5]", "[0.2, 0.2]"),
        ("[0.5, 0.5]", "[1]"),
        ("[0.5, 0.5]", "[0.5[V], 0.5[V]]"),
        ("[\"C4\", \"C3\"]", "[\"C3\", \"C3\"]"),
        ("[\"C4\", \"C3\"]", "[\"C3\"]"),
        ("channels: [\"C3\"]", "channels: []"),
        ("channels: [\"C3\"]", "channels: [\"unknown\"]"),
    ] {
        assert!(bci_compiler::compile(&SOURCE.replace(before, after), &examples()).is_err());
    }
    for (field, value) in [
        ("reference", "FCz"),
        ("unit", "V"),
        ("quantity", "optical.intensity"),
    ] {
        let mut schema: Value =
            serde_json::from_str(include_str!("../../examples/eeg.schema.json")).unwrap();
        schema["channels"][1][field] = serde_json::json!(value);
        let path = temp.0.join("schema.json");
        fs::write(&path, serde_json::to_vec(&schema).unwrap()).unwrap();
        let source = SOURCE
            .replace("eeg.schema.json", path.to_str().unwrap())
            .replace(
                &SOURCE[begin..end],
                "source.synthetic(schema: EEG, clock: Session, rate: 250[Hz])",
            );
        assert!(bci_compiler::compile(&source, &examples()).is_err());
    }
    for mutation in 0..4 {
        let mut broken = plan.clone();
        match mutation {
            0 => {
                let KernelSpec::ChannelMap { indices, .. } = &mut broken.nodes[1].kernel else {
                    unreachable!()
                };
                indices[0] = 99;
            }
            1 => {
                let KernelSpec::Rereference { weights, .. } = &mut broken.nodes[2].kernel else {
                    unreachable!()
                };
                weights[0] = bci_schema::number::Rational::ONE;
            }
            2 => {
                broken.nodes[2].output.as_mut().unwrap().schema.channels[0].reference = "CPz".into()
            }
            _ => {
                let KernelSpec::ChannelMap { width, .. } = &mut broken.nodes[3].kernel else {
                    unreachable!()
                };
                *width = 2;
            }
        }
        let stop = AtomicBool::new(false);
        let mut probe = Probe {
            prepared: false,
            written: 0,
            fail_at: None,
            stop_after: None,
            stop: &stop,
            finished: false,
        };
        let report = bci_runtime::run(
            &broken,
            &bci_operators::StandardKernels,
            &bci_artifact::input::FileSources,
            &mut probe,
            &stop,
        );
        assert_eq!(report.failure.unwrap().code, "E0801");
        assert!(!probe.prepared);
    }
}

#[test]
fn synchronous_mix_preserves_quality_replay_and_all_source_checkpoints() {
    use bci_runtime::{Checkpoint, CheckpointOptions, run_with_checkpoint};
    let temp = Temporary::new();
    let paired = include_str!("../../examples/paired.csv");
    let right = temp.0.join("paired.csv");
    fs::write(&right, paired).unwrap();
    let source =
        include_str!("../../examples/multi.bci").replace("paired.csv", right.to_str().unwrap());
    let compile = |s: &str| {
        bci_compiler::compile(s, &examples())
            .unwrap()
            .execution_plan(None, None)
            .unwrap()
    };
    let plan = compile(&source);
    assert_eq!(plan.sources, [0, 1]);
    let run = |plan: &ExecutionPlan, name: &str, saved: Option<&Checkpoint>, capture: bool| {
        let root = temp.0.join(name);
        let mut writer = FileRecorder(bci_artifact::recording::RunWriter::new(
            &examples(),
            Some(&root),
        ));
        let (report, snapshot) = run_with_checkpoint(
            plan,
            &bci_operators::StandardKernels,
            &bci_artifact::input::FileSources,
            &mut writer,
            &AtomicBool::new(false),
            CheckpointOptions {
                resume: saved,
                save: capture,
            },
        );
        assert!(report.failure.is_none(), "{:?}", report.failure);
        (single_run(&root), snapshot)
    };
    let (full_dir, _) = run(&plan, "full", None, false);
    let full = data(&full_dir);
    assert_eq!(full.len(), 73);
    let metadata = manifest(&full_dir);
    assert_eq!(metadata["sources"], serde_json::json!([0, 1]));
    assert_eq!(
        metadata["nodes"][7]["source_ports"],
        serde_json::json!([0, 1])
    );
    assert_eq!(metadata["report"]["completed_samples"], 22);
    assert_eq!(metadata["report"]["gaps"], 1);
    assert_eq!(metadata["report"]["missing_samples"], 2);
    for mixed in full.iter().filter(|r| r["port"] == 2) {
        let input = |port| {
            full.iter()
                .find(|r| r["port"] == port && r["sequence"] == mixed["sequence"])
                .unwrap()
        };
        let (left, right) = (input(0), input(1));
        let mut expected = left.clone();
        expected["port"] = serde_json::json!(2);
        expected["flags"] =
            serde_json::json!(left["flags"].as_u64().unwrap() | right["flags"].as_u64().unwrap());
        for c in 0..2 {
            let valid = left["valid"][c] == 1 && right["valid"][c] == 1;
            expected["valid"][c] = serde_json::json!(u8::from(valid));
            expected["values"][c] = serde_json::json!(if valid {
                left["values"][c].as_f64().unwrap() * 0.25
                    + right["values"][c].as_f64().unwrap() * 0.75
            } else {
                0.0
            });
        }
        assert_eq!(*mixed, expected);
    }
    let mut head_plan = plan.clone();
    head_plan.samples = Some(12);
    let (head_dir, saved) = run(&head_plan, "head", None, true);
    let saved = saved.unwrap();
    assert_eq!(saved.position, 12);
    let (tail_dir, _) = run(&plan, "tail", Some(&saved), false);
    let mut combined = data(&head_dir);
    combined.extend(data(&tail_dir));
    assert_eq!(combined, full);
    assert_eq!(manifest(&tail_dir)["report"]["input_offset"], 12);

    let mut replay = source.clone();
    for port in 0..2 {
        let begin = replay.find("source.csv(").unwrap();
        let end = begin + replay[begin..].find(");").unwrap() + 1;
        replay.replace_range(
            begin..end,
            &format!(
                "source.replay(path:{:?},port:{port})",
                full_dir.join("sink-8.jsonl").to_str().unwrap()
            ),
        );
    }
    let replay_plan = compile(&replay);
    assert_eq!(data(&run(&replay_plan, "replay", None, false).0), full);

    // 첫 입력의 유래가 바뀌면 FIR·창의 기여 구간까지 Mixed를 유지한다.
    let path = full_dir.join("sink-8.jsonl");
    let text = fs::read_to_string(&path).unwrap();
    let changed = text.replacen("\"origin\":\"Measurement\"", "\"origin\":\"Synthetic\"", 1);
    assert_ne!(changed, text);
    fs::write(&path, changed).unwrap();
    let (origins_dir, _) = run(&replay_plan, "origins", None, false);
    let origins = data(&origins_dir);
    let origin = |port, sequence| {
        origins
            .iter()
            .find(|r| r["port"] == port && r["sequence"] == sequence)
            .unwrap()["origin"]
            .as_str()
            .unwrap()
    };
    assert_eq!(origin(2, 0), "Mixed");
    assert_eq!(origin(2, 1), "Measurement");
    assert_eq!(origin(7, 5), "Mixed");
    assert_eq!(origin(7, 7), "Measurement");
    let mut origin_head = replay_plan.clone();
    origin_head.samples = Some(2);
    let (head, snapshot) = run(&origin_head, "origin-head", None, true);
    let (tail, _) = run(&replay_plan, "origin-tail", snapshot.as_ref(), false);
    let mut combined = data(&head);
    combined.extend(data(&tail));
    assert_eq!(combined, origins);

    for (case, csv) in [
        paired.replace("0.000,1,5,1,1\n", ""),
        paired.lines().take(22).collect::<Vec<_>>().join("\n") + "\n",
        paired.replace("0.056,5,7,1,1", "0.052,5,7,1,1"),
        paired.replace("0.004,3,3,1,1", "0.004,9,3,1,1"),
    ]
    .into_iter()
    .enumerate()
    {
        fs::write(&right, csv).unwrap();
        let stop = AtomicBool::new(false);
        let mut probe = Probe {
            prepared: false,
            written: 0,
            fail_at: None,
            stop_after: None,
            stop: &stop,
            finished: false,
        };
        let (report, snapshot) = run_with_checkpoint(
            &plan,
            &bci_operators::StandardKernels,
            &bci_artifact::input::FileSources,
            &mut probe,
            &stop,
            CheckpointOptions {
                resume: (case == 3).then_some(&saved),
                save: true,
            },
        );
        assert_eq!(
            report.failure.unwrap().code,
            if case == 3 { "E0909" } else { "E0904" }
        );
        assert!(snapshot.is_none());
        if case == 0 || case == 3 {
            assert_eq!(probe.written, 0);
        }
        if case == 3 {
            assert!(!probe.prepared);
        }
    }
    fs::write(&right, paired).unwrap();
    for case in 0..2 {
        let mut broken = plan.clone();
        if case == 0 {
            broken.sources.reverse();
        } else {
            let bci_schema::kernel::KernelSpec::Mix { weights, .. } = &mut broken.nodes[2].kernel
            else {
                unreachable!()
            };
            weights.pop();
        }
        let stop = AtomicBool::new(false);
        let mut probe = Probe {
            prepared: false,
            written: 0,
            fail_at: None,
            stop_after: None,
            stop: &stop,
            finished: false,
        };
        let report = bci_runtime::run(
            &broken,
            &bci_operators::StandardKernels,
            &bci_artifact::input::FileSources,
            &mut probe,
            &stop,
        );
        assert_eq!(report.failure.unwrap().code, "E0801");
        assert!(!probe.prepared);
    }
    for weights in ["[1]", "[1[V], 1[V]]"] {
        assert!(
            bci_compiler::compile(&source.replace("[0.25, 0.75]", weights), &examples()).is_err()
        );
    }
    let independent = format!(
        "{}recording.write((left,right),to:\"unused\"); }}",
        source.split("    let mixed").next().unwrap()
    );
    for text in [
        independent.replacen("rate: 250[Hz]", "rate: 125[Hz]", 1),
        independent.replacen("rate: 250[Hz]", "phase: 0.004[s], rate: 250[Hz]", 1),
    ] {
        let plan = bci_compiler::compile(&text, &examples())
            .unwrap()
            .execution_plan(None, None)
            .unwrap();
        assert!(!plan.synchronous_inputs());
    }
    let text = independent
        .replace(
            "run Main",
            "clock Other = time.session_clock(name: \"other\"); run Main",
        )
        .replacen("clock: Session", "clock: Other", 1);
    assert_eq!(
        bci_compiler::compile(&text, &examples())
            .unwrap()
            .execution_plan(None, None)
            .unwrap_err()
            .code,
        "E0401"
    );
    let unequal = source.replacen("rate: 250[Hz]", "rate: 125[Hz]", 1);
    assert!(bci_compiler::compile(&unequal, &examples()).is_err());
}

#[test]
fn stateful_checkpoint_preserves_full_pipeline_and_rejects_mismatches() {
    use bci_extension::Extensions;
    use bci_runtime::{Checkpoint, CheckpointOptions, run_with_checkpoint};
    let extensions = Extensions::empty()
        .with_stateful(vec![lab_state::RunningMean::bci_registration()])
        .unwrap();
    assert_eq!(extensions.verify_references().unwrap(), 4);
    let temp = Temporary::new();
    let csv = include_str!("../../examples/samples.csv");
    let input = temp.0.join("samples.csv");
    fs::write(&input, csv).unwrap();
    let source =
        include_str!("../../examples/stateful.bci").replace("samples.csv", input.to_str().unwrap());
    let compilation =
        bci_compiler::compile_with_extensions(&source, &examples(), extensions.specifications())
            .unwrap();
    let plan = compilation.execution_plan(None, None).unwrap();
    let run = |plan: &ExecutionPlan, name: &str, saved: Option<&Checkpoint>, capture: bool| {
        let root = temp.0.join(name);
        let mut writer = FileRecorder(bci_artifact::recording::RunWriter::new(
            &examples(),
            Some(&root),
        ));
        let (report, snapshot) = run_with_checkpoint(
            plan,
            &extensions.kernels(&bci_operators::StandardKernels),
            &bci_artifact::input::FileSources,
            &mut writer,
            &AtomicBool::new(false),
            CheckpointOptions {
                resume: saved,
                save: capture,
            },
        );
        assert!(report.failure.is_none(), "{:?}", report.failure);
        (
            data(&single_run(&root)),
            snapshot,
            manifest(&single_run(&root)),
        )
    };
    let (full, _, _) = run(&plan, "full", None, false);
    assert_eq!(full.len(), 51);
    let mut generation = None;
    let mut first = 0;
    let mut sums = [0.0; 2];
    let mut counts = [0u64; 2];
    for raw in full.iter().filter(|v| v["port"] == 0) {
        if generation != raw["generation"].as_u64() {
            generation = raw["generation"].as_u64();
            first = raw["sequence"].as_u64().unwrap();
            sums.fill(0.0);
            counts.fill(0);
        }
        let mut expected = raw.clone();
        expected["port"] = serde_json::json!(1);
        expected["source_range"][0] = serde_json::json!(first);
        for c in 0..2 {
            if raw["valid"][c] == 1 {
                sums[c] += raw["values"][c].as_f64().unwrap();
                counts[c] += 1;
                expected["values"][c] = serde_json::json!(sums[c] / counts[c] as f64);
            }
        }
        assert_eq!(
            *full
                .iter()
                .find(|v| v["port"] == 1 && v["sequence"] == raw["sequence"])
                .unwrap(),
            expected
        );
    }
    let mut boundary = None;
    for split in [2, 12, 17] {
        let mut head_plan = plan.clone();
        head_plan.samples = Some(split);
        let (mut head, saved, _) = run(&head_plan, &format!("head-{split}"), None, true);
        let saved = saved.unwrap();
        assert_eq!(saved.position, split);
        let path = temp.0.join(format!("state-{split}.json"));
        bci_artifact::checkpoint::save(&path, &saved).unwrap();
        let saved = bci_artifact::checkpoint::load(&path).unwrap();
        let (tail, finished, manifest) = run(&plan, &format!("tail-{split}"), Some(&saved), true);
        assert_eq!(manifest["report"]["input_offset"], split);
        assert_eq!(finished.unwrap().position, 22);
        head.extend(tail);
        assert_eq!(head, full, "split={split}");
        if split == 12 {
            boundary = Some(saved);
        }
        let original = fs::read(&path).unwrap();
        assert!(
            bci_artifact::checkpoint::save(
                &path,
                boundary
                    .as_ref()
                    .unwrap_or(&bci_artifact::checkpoint::load(&path).unwrap())
            )
            .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), original);
    }
    let boundary = boundary.unwrap();
    for case in 0..5 {
        let mut altered = boundary.clone();
        let mut altered_plan = plan.clone();
        match case {
            0 => altered_plan.nodes[1].operator = "lab.signal.other".into(),
            1 => altered.kernels[1].truncate(1),
            2 => altered.grids[3].last_index = Some(u64::MAX),
            3 => {
                fs::write(&input, csv.replace("0.004,1,1,1,1", "0.004,8,1,1,1")).unwrap();
            }
            _ => altered.position = 100,
        }
        let stop = AtomicBool::new(false);
        let mut probe = Probe {
            prepared: false,
            written: 0,
            fail_at: None,
            stop_after: None,
            stop: &stop,
            finished: false,
        };
        let (report, snapshot) = run_with_checkpoint(
            &altered_plan,
            &extensions.kernels(&bci_operators::StandardKernels),
            &bci_artifact::input::FileSources,
            &mut probe,
            &stop,
            CheckpointOptions {
                resume: Some(&altered),
                save: true,
            },
        );
        assert!(
            matches!(report.failure.as_ref().unwrap().code, "E0801" | "E0909"),
            "case={case}, {:?}",
            report.failure
        );
        assert!(!probe.prepared && snapshot.is_none());
        fs::write(&input, csv).unwrap();
    }
    let path = temp.0.join("state-12.json");
    let original = fs::read(&path).unwrap();
    for field in ["payload_sha256", "executable_sha256"] {
        let mut json: Value = serde_json::from_slice(&original).unwrap();
        json[field] = serde_json::json!("0".repeat(64));
        let broken = temp.0.join(format!("broken-{field}.json"));
        fs::write(&broken, serde_json::to_vec(&json).unwrap()).unwrap();
        assert!(bci_artifact::checkpoint::load(&broken).is_err());
    }
    let stop = AtomicBool::new(false);
    let mut probe = Probe {
        prepared: false,
        written: 0,
        fail_at: Some(3),
        stop_after: None,
        stop: &stop,
        finished: false,
    };
    let (report, snapshot) = run_with_checkpoint(
        &plan,
        &extensions.kernels(&bci_operators::StandardKernels),
        &bci_artifact::input::FileSources,
        &mut probe,
        &stop,
        CheckpointOptions {
            resume: None,
            save: true,
        },
    );
    assert_eq!(report.failure.unwrap().code, "E0902");
    assert!(snapshot.is_none());
}

#[test]
fn rust_extension_contract_quality_replay_and_faults() {
    use bci_extension::Extensions;
    use bci_schema::kernel::KernelSpec;
    let extensions = Extensions::new(vec![lab_signal::rectify_registration()]).unwrap();
    assert_eq!(extensions.verify_references().unwrap(), 8);
    assert!(
        Extensions::new(vec![
            lab_signal::rectify_registration(),
            lab_signal::rectify_registration()
        ])
        .is_err()
    );
    let temp = Temporary::new();
    fs::copy(
        examples().join("eeg.schema.json"),
        temp.0.join("eeg.schema.json"),
    )
    .unwrap();
    let csv = include_str!("../../examples/samples.csv")
        .replace("0.000,0,2,1,1", "0.000,-0,-2,1,1")
        .replace("0.080,4,3,1,0", "0.080,4,-3,1,0");
    fs::write(temp.0.join("samples.csv"), csv).unwrap();
    let source = r#"language "bci/0.1"; module extension.check;
use std.schema as schema; use std.time as time; use std.source as source;
use std.signal as signal; use std.recording as recording; use lab.signal as lab;
schema EEG = schema.load("eeg.schema.json"); clock Session = time.session_clock(name:"file");
run Main {
    let raw=source.csv(path:"samples.csv",schema:EEG,clock:Session,rate:250[Hz],gap:Gap.Reset,invalid:Invalid.Mark);
    let rectified=raw |> lab.rectify();
    recording.write((raw,rectified),to:"unused");
}"#;
    let compile = |text: &str| {
        bci_compiler::compile_with_extensions(text, &temp.0, extensions.specifications())
    };
    let plan = compile(source).unwrap().execution_plan(None, None).unwrap();
    assert_eq!(plan.nodes[0].output, plan.nodes[1].output);
    assert_eq!(
        bci_compiler::compile(source, &temp.0).unwrap_err()[0].code,
        "E0103"
    );
    let mut wrong_quantity = extensions.specifications().to_vec();
    wrong_quantity[0].quantity = "optical.intensity".into();
    assert_eq!(
        bci_compiler::compile_with_extensions(source, &temp.0, &wrong_quantity).unwrap_err()[0]
            .code,
        "E0204"
    );
    assert_eq!(
        compile(&source.replace(
            "raw |> lab.rectify()",
            "raw |> signal.window(size:4,hop:2) |> lab.rectify()"
        ))
        .unwrap_err()[0]
            .code,
        "E0201"
    );
    assert_eq!(
        compile(&source.replace("run Main", "const Bad = lab.rectify(1); run Main")).unwrap_err()
            [0]
        .code,
        "E0211"
    );
    let run = |plan: &ExecutionPlan, name: &str| {
        let output = temp.0.join(name);
        let mut writer = FileRecorder(bci_artifact::recording::RunWriter::new(
            &temp.0,
            Some(&output),
        ));
        let report = bci_runtime::run(
            plan,
            &extensions.kernels(&bci_operators::StandardKernels),
            &bci_artifact::input::FileSources,
            &mut writer,
            &AtomicBool::new(false),
        );
        assert!(report.failure.is_none(), "{:?}", report.failure);
        single_run(&output)
    };
    let first = run(&plan, "first");
    let records = data(&first);
    assert_eq!(records.len(), 44);
    for pair in records.chunks_exact(2) {
        let mut expected = pair[0].clone();
        expected["port"] = serde_json::json!(1);
        for c in 0..2 {
            let value = expected["values"][c].as_f64().unwrap();
            if expected["valid"][c] == 1 && value < 0.0 {
                expected["values"][c] = serde_json::json!(0.0);
            }
        }
        assert_eq!(pair[1], expected);
    }
    assert_eq!(
        records[1]["values"][0].as_f64().unwrap().to_bits(),
        (-0.0f64).to_bits()
    );
    let first_manifest = manifest(&first);
    assert_eq!(first_manifest["report"]["gaps"], 1);
    assert_eq!(first_manifest["report"]["missing_samples"], 2);
    assert_eq!(
        first_manifest["nodes"][1]["kernel"]["contract_digest"],
        extensions.specifications()[0].contract_digest
    );
    assert_eq!(
        first_manifest["executable_sha256"].as_str().unwrap().len(),
        64
    );
    let start = source.find("source.csv(").unwrap();
    let end = start + source[start..].find(");").unwrap() + 1;
    let replay = source.replace(
        &source[start..end],
        &format!(
            "source.replay(path:{:?},port:0)",
            first.join("sink-2.jsonl").to_str().unwrap()
        ),
    );
    let replay = compile(&replay)
        .unwrap()
        .execution_plan(None, None)
        .unwrap();
    assert_eq!(records, data(&run(&replay, "replay")));
    for case in 0..5 {
        let mut plan = plan.clone();
        let mut registration = lab_signal::rectify_registration();
        let (expected, prepared) = match case {
            0 => {
                let KernelSpec::Extension { operator, .. } = &mut plan.nodes[1].kernel else {
                    unreachable!()
                };
                operator.function_digest = "0".repeat(64);
                ("E0510", false)
            }
            1 => {
                plan.nodes[1].output.as_mut().unwrap().clock = "other".into();
                ("E0801", false)
            }
            2 => {
                registration.reference = &[(0, 1)];
                ("E0510", false)
            }
            3 => {
                registration.reference = &[(0, 0)];
                registration.function = |value| {
                    assert!(value <= 2.0, "주입한 함수 panic");
                    value
                };
                ("E0908", true)
            }
            _ => {
                registration.reference = &[(0, 0)];
                registration.function = |value| if value > 2.0 { f64::INFINITY } else { value };
                ("E0903", true)
            }
        };
        let broken = Extensions::new(vec![registration]).unwrap();
        let stop = AtomicBool::new(false);
        let mut probe = Probe {
            prepared: false,
            written: 0,
            fail_at: None,
            stop_after: None,
            stop: &stop,
            finished: false,
        };
        let report = bci_runtime::run(
            &plan,
            &broken.kernels(&bci_operators::StandardKernels),
            &bci_artifact::input::FileSources,
            &mut probe,
            &stop,
        );
        assert_eq!(
            report.failure.as_ref().unwrap().code,
            expected,
            "case {case}"
        );
        assert_eq!(probe.prepared, prepared);
        assert_eq!(probe.finished, prepared);
        assert_eq!(report.completed_samples, if prepared { 2 } else { 0 });
        assert_eq!(probe.written, if prepared { 4 } else { 0 });
        if prepared {
            assert_eq!(report.nodes[1].records, 2);
        }
    }
}

#[test]
fn csv_and_replay_preserve_values_time_quality_and_gap_boundaries() {
    const FILE: &str = include_str!("../../examples/file.bci");
    const CSV: &str = include_str!("../../examples/samples.csv");
    let temp = Temporary::new();
    let output = temp.0.join("csv");
    let result = cli_eof(&examples().join("file.bci"), &output);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let run = single_run(&output);
    let report = manifest(&run);
    assert_eq!(report["report"]["completed_samples"], 22);
    assert_eq!(report["report"]["end_of_input"], true);
    assert_eq!(report["report"]["gaps"], 1);
    assert_eq!(report["report"]["missing_samples"], 2);
    assert_eq!(report["report"]["nodes"][5]["records"], 7);
    assert_eq!(report["report"]["nodes"][5]["valid_records"], 2);
    assert_eq!(report["report"]["nodes"][3]["generations"], 2);
    assert_eq!(
        report["report"]["nodes"][3]["generation_first_time_seconds"],
        "19/250"
    );
    let recorded = data(&run);
    let at_gap = recorded
        .iter()
        .find(|v| v["port"] == 0 && v["sequence"] == 14)
        .unwrap();
    assert_eq!(at_gap["generation"], 1);
    assert_eq!(at_gap["gap"], serde_json::json!([12, 13]));
    assert_eq!(at_gap["origin"], "Measurement");
    let invalid = recorded
        .iter()
        .find(|v| v["port"] == 0 && v["sequence"] == 6)
        .unwrap();
    assert_eq!(invalid["values"][0], 0.0);
    assert_eq!(invalid["valid"][0], 0);

    let begin = FILE.find("source.csv(").unwrap();
    let end = begin + FILE[begin..].find(");").unwrap() + 1;
    let original = &FILE[begin..end];
    let replay = FILE.replace(
        original,
        &format!(
            "source.replay(path: {:?}, port: 0, expect: EEG, clock: Session, rate: 250[Hz])",
            run.join("sink-6.jsonl").to_str().unwrap()
        ),
    );
    let source = source_at(&temp.0, &replay);
    let output = temp.0.join("replay");
    let result = cli_eof(&source, &output);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let replay_run = single_run(&output);
    assert_eq!(recorded, data(&replay_run));
    assert_eq!(manifest(&replay_run)["report"]["missing_samples"], 2);

    let matrix = format!(
        "language \"bci/0.1\"; module replay.matrix; use std.source as source; use std.recording as recording; run Main {{ let x=source.replay(path: {:?}, port: 5); recording.write(x,to:\"unused\"); }}",
        run.join("sink-6.jsonl").to_str().unwrap()
    );
    let source = source_at(&temp.0, &matrix);
    let output = temp.0.join("matrix");
    let result = cli_eof(&source, &output);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let mut expected: Vec<_> = recorded
        .iter()
        .filter(|r| r["port"] == 5)
        .cloned()
        .collect();
    for row in &mut expected {
        row["port"] = serde_json::json!(0);
    }
    assert_eq!(expected, data(&single_run(&output)));

    let limit = temp.0.join("limit");
    assert!(
        cli(&examples().join("file.bci"), &limit, "3")
            .status
            .success()
    );
    let limited = manifest(&single_run(&limit));
    assert_eq!(limited["report"]["completed_samples"], 3);
    assert_eq!(limited["report"]["end_of_input"], false);
    assert_eq!(limited["obligations"][0]["evidence"]["status"], "Guarded");

    for (case, (csv, program, exit)) in [
        (CSV.replace("C3[uV]", "C3[V]"), FILE.to_string(), 2),
        (
            CSV.replace("0.004,1,1,1,1", "0.003,1,1,1,1"),
            FILE.to_string(),
            4,
        ),
        (
            CSV.replace("0.008,3,0,1,1", "0.000,3,0,1,1"),
            FILE.to_string(),
            4,
        ),
        (CSV.to_string(), FILE.replace("Gap.Reset", "Gap.Fault"), 4),
        (
            CSV.to_string(),
            FILE.replace("Invalid.Mark", "Invalid.Fault"),
            4,
        ),
        (CSV.replace("0.004,1,1,1,1", "0.004,1"), FILE.to_string(), 4),
    ]
    .into_iter()
    .enumerate()
    {
        let csv_path = temp.0.join("data.csv");
        fs::write(&csv_path, csv).unwrap();
        let source = source_at(
            &temp.0,
            &program.replace("samples.csv", csv_path.to_str().unwrap()),
        );
        let result = cli_eof(&source, &temp.0.join(format!("error-{case}")));
        assert_eq!(
            result.status.code(),
            Some(exit),
            "case {case}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(String::from_utf8_lossy(&result.stderr).contains("E0904"));
        assert!(String::from_utf8_lossy(&result.stderr).contains("data.csv:"));
    }

    let truncated = temp.0.join("truncated");
    fs::create_dir(&truncated).unwrap();
    fs::copy(run.join("manifest.json"), truncated.join("manifest.json")).unwrap();
    let text = fs::read_to_string(run.join("sink-6.jsonl")).unwrap();
    let lines: Vec<_> = text.lines().collect();
    fs::write(
        truncated.join("sink-6.jsonl"),
        lines[..lines.len() - 2].join("\n") + "\n",
    )
    .unwrap();
    let source = source_at(
        &temp.0,
        &replay.replace(
            run.join("sink-6.jsonl").to_str().unwrap(),
            truncated.join("sink-6.jsonl").to_str().unwrap(),
        ),
    );
    let result = cli_eof(&source, &temp.0.join("truncated-result"));
    assert_eq!(result.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&result.stderr).contains("commit 관측값"));
}
fn cli(source: &Path, output: &Path, samples: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_bci"))
        .arg("run")
        .arg(source)
        .args(["--samples", samples, "--output"])
        .arg(output)
        .output()
        .unwrap()
}

#[test]
fn example_records_match_fir_covariance_reference() {
    let temp = Temporary::new();
    let output = temp.0.join("result");
    let result = cli(&examples().join("typed.bci"), &output, "1000");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let run = single_run(&output);
    let metadata = manifest(&run);
    assert_eq!(metadata["status"], "Closed");
    assert_eq!(metadata["report"]["completed_samples"], 1000);
    assert_eq!(metadata["report"]["nodes"][2]["valid_records"], 998);
    assert_eq!(metadata["report"]["nodes"][5]["valid_records"], 30);
    assert_eq!(
        metadata["obligations"][0]["evidence"]["anchor_seconds"],
        "251/250"
    );
    assert_eq!(
        metadata["obligations"][1]["evidence"]["cholesky_passes"],
        30
    );
    assert_eq!(
        metadata["obligations"][2]["evidence"]["status"],
        "Committed"
    );
    let records = fs::read_to_string(run.join("sink-6.jsonl")).unwrap();
    let data: Vec<Value> = records
        .lines()
        .skip(1)
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    let raw: Vec<_> = data.iter().filter(|r| r["port"] == 0).collect();
    let covariance: Vec<_> = data.iter().filter(|r| r["port"] == 5).collect();
    assert_eq!((raw.len(), covariance.len()), (1000, 30));
    for (k, actual) in covariance.iter().enumerate() {
        let start = 2 + k * 25;
        let signal: Vec<[f64; 2]> = (start..start + 250)
            .map(|n| {
                std::array::from_fn(|c| {
                    [0.25, 0.5, 0.25]
                        .iter()
                        .enumerate()
                        .map(|(lag, h)| h * raw[n - lag]["values"][c].as_f64().unwrap() * 1e-6)
                        .sum()
                })
            })
            .collect();
        let means: [f64; 2] =
            std::array::from_fn(|c| signal.iter().map(|row| row[c]).sum::<f64>() / 250.0);
        let mut reference = [0.0; 4];
        for row in 0..2 {
            for col in 0..2 {
                reference[row * 2 + col] = signal
                    .iter()
                    .map(|x| (x[row] - means[row]) * (x[col] - means[col]))
                    .sum::<f64>()
                    / 249.0;
            }
        }
        let variance = ((reference[0] + reference[3]) / 2.0).max(1e-12);
        for (i, expected) in reference.iter_mut().enumerate() {
            *expected = 0.9 * *expected
                + if i == 0 || i == 3 {
                    0.1 * variance
                } else {
                    0.0
                };
            let value = actual["values"][i].as_f64().unwrap();
            assert!(
                (value - *expected).abs() <= 1e-27 + 1e-10 * expected.abs(),
                "record {k}, element {i}: {value} != {expected}"
            );
        }
        assert_eq!(actual["sequence"], start + 249);
        assert_eq!(actual["source_range"][0], start - 2);
        assert_eq!(actual["flags"], 0);
        assert_eq!(actual["valid"], serde_json::json!([1, 1]));
        let values: Vec<_> = actual["values"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect();
        assert!(values[0] > 0.0 && values[0] * values[3] - values[1] * values[2] > 0.0);
    }
    let short = temp.0.join("short");
    assert!(
        cli(&examples().join("typed.bci"), &short, "10")
            .status
            .success()
    );
    let metadata = manifest(&single_run(&short));
    assert_eq!(
        metadata["obligations"][0]["evidence"]["status"],
        "Unobserved"
    );
    assert_eq!(metadata["report"]["nodes"][5]["records"], 0);
    let fixed_source = source_at(&temp.0, &EXAMPLE.replace("WindowStart.FirstValid", "0"));
    let fixed = temp.0.join("fixed");
    assert!(cli(&fixed_source, &fixed, "275").status.success());
    let fixed = manifest(&single_run(&fixed));
    assert_eq!(fixed["report"]["nodes"][5]["records"], 2);
    assert_eq!(fixed["report"]["nodes"][5]["valid_records"], 1);
    assert_eq!(fixed["report"]["nodes"][3]["first_time_seconds"], "249/250");
}

struct Probe<'a> {
    prepared: bool,
    written: usize,
    fail_at: Option<usize>,
    stop_after: Option<usize>,
    stop: &'a AtomicBool,
    finished: bool,
}
impl Recorder for Probe<'_> {
    fn prepare(&mut self, _: &ExecutionPlan) -> Result<(), String> {
        self.prepared = true;
        Ok(())
    }
    fn write(&mut self, _: usize, _: usize, _: &Frame) -> Result<(), String> {
        if self.fail_at == Some(self.written) {
            return Err("주입한 저장 오류".into());
        }
        self.written += 1;
        if self.stop_after == Some(self.written) {
            self.stop.store(true, Ordering::Relaxed);
        }
        Ok(())
    }
    fn finish(&mut self, _: &RunReport) -> Result<(), String> {
        self.finished = true;
        Ok(())
    }
}

#[test]
fn plan_fault_stop_and_io_boundaries() {
    let compiled = bci_compiler::compile(EXAMPLE, &examples()).unwrap();
    let plan = compiled.execution_plan(None, Some(1000)).unwrap();
    for (fail_at, stop_after, completed) in [(Some(5), None, 5), (None, Some(7), 7)] {
        let stop = AtomicBool::new(false);
        let mut probe = Probe {
            prepared: false,
            written: 0,
            fail_at,
            stop_after,
            stop: &stop,
            finished: false,
        };
        let report = bci_runtime::run(
            &plan,
            &bci_operators::StandardKernels,
            &bci_operators::SyntheticSources,
            &mut probe,
            &stop,
        );
        assert_eq!(report.completed_samples, completed);
        assert_eq!(report.failure.is_some(), fail_at.is_some());
        assert_eq!(report.stopped, stop_after.is_some());
        assert_eq!(report.states.last(), Some(&SessionState::Closed));
        assert!(probe.prepared && probe.finished);
    }
    for mutation in 0..4 {
        let mut broken = plan.clone();
        match mutation {
            0 => broken.memory_budget = 1,
            1 => broken.schedule.swap(0, 1),
            2 => broken.nodes[2].inputs = vec![2],
            _ => broken.obligations.clear(),
        }
        let stop = AtomicBool::new(false);
        let mut probe = Probe {
            prepared: false,
            written: 0,
            fail_at: None,
            stop_after: None,
            stop: &stop,
            finished: false,
        };
        let report = bci_runtime::run(
            &broken,
            &bci_operators::StandardKernels,
            &bci_operators::SyntheticSources,
            &mut probe,
            &stop,
        );
        assert_eq!(report.failure.unwrap().code, "E0801");
        assert!(!probe.prepared);
    }
    let tiny = EXAMPLE.replace("16[MiB]", "16[byte]");
    assert_eq!(
        bci_compiler::compile(&tiny, &examples())
            .unwrap()
            .execution_plan(None, Some(1000))
            .unwrap_err()
            .code,
        "E0801"
    );
    let offline = EXAMPLE
        .replace("! live", "! record")
        .replace("Mode.Live", "Mode.Record")
        .replace(
            "signal.fir(coeffs: Filter, warmup: Warmup.Full)",
            "signal.filtfilt(coeffs: Filter)",
        );
    assert_eq!(
        bci_compiler::compile(&offline, &examples())
            .unwrap()
            .execution_plan(None, Some(1000))
            .unwrap_err()
            .code,
        "E0803"
    );
    let temp = Temporary::new();
    let blocked = temp.0.join("file");
    fs::write(&blocked, "occupied").unwrap();
    let io = cli(&examples().join("typed.bci"), &blocked, "1000");
    assert_eq!(io.status.code(), Some(5));
    assert!(String::from_utf8_lossy(&io.stderr).contains("E0902"));
    assert_eq!(fs::read_to_string(&blocked).unwrap(), "occupied");
    let overflow = EXAMPLE.replace(
        "let smooth = Smooth(volts);",
        &format!(
            "let smooth = Smooth(volts){};",
            " |> signal.scale(factor: 1e38)".repeat(9)
        ),
    );
    let source = source_at(&temp.0, &overflow);
    let output = temp.0.join("overflow");
    let result = cli(&source, &output, "1000");
    assert_eq!(result.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&result.stderr).contains("E0903"));
    let report = manifest(&single_run(&output));
    assert_eq!(report["status"], "Faulted");
    assert_eq!(report["report"]["failure"]["code"], "E0903");
}
