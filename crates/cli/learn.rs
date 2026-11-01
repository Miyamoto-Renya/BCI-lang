use std::{ffi::OsString, path::Path, process::ExitCode};

pub(crate) fn run(args: &[OsString]) -> ExitCode {
    let result=match args {
        [action,data,options @ ..] if action=="fit"=>output_options(options).and_then(|(output,protocol)|fit(Path::new(data),output,protocol)),
        [action,model,data,options @ ..] if action=="evaluate"=>output_options(options).and_then(|(output,protocol)|evaluate(Path::new(model),Path::new(data),output,protocol)),
        [action,data,options @ ..] if action=="select"=>select(Path::new(data),options),
        _=>Err("E0101: bci learn fit DATASET --output DIRECTORY, bci learn select DATASET --candidate MODEL --candidate MODEL --output DIRECTORY 또는 bci learn evaluate MODEL DATASET --output FILE을 사용한다.".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(if error.contains("E0902") { 5 } else { 2 })
        }
    }
}
fn output_options(options: &[OsString]) -> Result<(&Path, Option<&Path>), String> {
    let mut output = None;
    let mut protocol = None;
    let mut pairs = options.chunks_exact(2);
    for pair in &mut pairs {
        if pair[0] == "--output" && output.is_none() {
            output = Some(Path::new(&pair[1]));
        } else if pair[0] == "--protocol" && protocol.is_none() {
            protocol = Some(Path::new(&pair[1]));
        } else {
            return Err(
                "E0101: --output 경로와 선택적 --protocol 경로를 각각 한 번 지정한다.".into(),
            );
        }
    }
    if !pairs.remainder().is_empty() {
        return Err("E0101: 평가 옵션 뒤에 경로를 지정한다.".into());
    }
    Ok((output.ok_or("E0101: --output 경로를 지정한다.")?, protocol))
}
fn report(
    model: &bci_schema::learning::Model,
    data: &bci_schema::learning::Dataset,
    protocol: Option<&Path>,
) -> Result<bci_schema::learning::Evaluation, String> {
    match protocol {
        Some(path) => bci_operators::evaluation::evaluate(
            model,
            data,
            &bci_artifact::evaluation::load_protocol(path)?,
        ),
        None => bci_operators::learning::evaluate(model, data),
    }
}
fn print_analysis(e: &bci_schema::learning::Evaluation) {
    if let Some(a) = &e.analysis {
        if let Some(score) = a.estimate.balanced_accuracy {
            println!(
                "  집계 {:?}: balanced accuracy {score:.6}",
                a.protocol.aggregation
            );
        }
        if let Some(b) = &a.resampling {
            let ci = &b.balanced_accuracy_interval;
            println!(
                "  bootstrap balanced accuracy 구간 [{:.6}, {:.6}], 정의 가능한 반복 {}/{}",
                ci.lower,
                ci.upper,
                ci.defined,
                b.balanced_accuracy.len()
            );
        }
    }
}
fn fit(path: &Path, output: &Path, protocol: Option<&Path>) -> Result<(), String> {
    let data = bci_artifact::learning::load_dataset(path)?;
    if protocol.is_some()
        && data
            .rows
            .iter()
            .any(|r| r.assignment.role == bci_schema::learning::Role::Tune)
    {
        return Err(
            "E0101: Tune 후보의 평가 protocol은 learn select --protocol에 지정한다.".into(),
        );
    }
    let model = bci_operators::learning::fit(&data)?;
    let mut files = vec![("model.json", bci_artifact::learning::model_json(&model))];
    if data
        .rows
        .iter()
        .any(|r| r.assignment.role == bci_schema::learning::Role::Tune)
    {
        bci_artifact::learning::write_results(output, &files)?;
        println!(
            "후보 학습 완료: Fit {}개\n  모델 {}\n  결과: {}\nTune 선택과 최종 Eval은 learn select에서 실행한다.",
            model.fit_rows.len(),
            model.digest(),
            output.display()
        );
    } else {
        let evaluation = report(&model, &data, protocol)?;
        files.push((
            "evaluation.json",
            bci_artifact::learning::evaluation_json(&evaluation),
        ));
        bci_artifact::learning::write_results(output, &files)?;
        println!(
            "학습·평가 완료: Fit {}개, Eval {}개, 제외 Eval {}개, balanced accuracy {:.6}\n  모델 {}\n  결과: {}",
            model.fit_rows.len(),
            evaluation.predictions.len(),
            evaluation.excluded.len(),
            evaluation.balanced_accuracy,
            model.digest(),
            output.display()
        );
        print_analysis(&evaluation);
    }
    Ok(())
}
fn select(data: &Path, options: &[OsString]) -> Result<(), String> {
    let usage = "E0101: learn select DATASET에 --candidate MODEL 2..16개·--output DIRECTORY 한 개와 선택적 --protocol FILE을 지정한다.";
    let mut paths = Vec::new();
    let mut output = None;
    let mut protocol = None;
    let mut pairs = options.chunks_exact(2);
    for pair in &mut pairs {
        if pair[0] == "--candidate" && paths.len() < 16 {
            paths.push(Path::new(&pair[1]));
        } else if pair[0] == "--output" && output.is_none() {
            output = Some(Path::new(&pair[1]));
        } else if pair[0] == "--protocol" && protocol.is_none() {
            protocol = Some(Path::new(&pair[1]));
        } else {
            return Err(usage.into());
        }
    }
    if !pairs.remainder().is_empty() || paths.len() < 2 {
        return Err(usage.into());
    }
    let output = output.ok_or(usage)?;
    let data = bci_artifact::learning::load_dataset(data)?;
    let candidates = paths
        .into_iter()
        .map(bci_artifact::learning::load_model)
        .collect::<Result<Vec<_>, _>>()?;
    let model = bci_operators::learning::select(&candidates, &data)?;
    let evaluation = report(&model, &data, protocol)?;
    let selection = model
        .selection
        .as_ref()
        .ok_or("E0703: 선택 증거를 모델과 연결한다.")?;
    bci_artifact::learning::write_results(
        output,
        &[
            ("model.json", bci_artifact::learning::model_json(&model)),
            (
                "selection.json",
                bci_artifact::learning::selection_json(selection),
            ),
            (
                "evaluation.json",
                bci_artifact::learning::evaluation_json(&evaluation),
            ),
        ],
    )?;
    let (_, tune_score) =
        bci_schema::selection::metric(&selection.confusion[selection.winner], model.classes.len())?;
    println!(
        "선택·평가 완료: 후보 {}개 중 {}번, Tune {}개, balanced accuracy {:.6}\n  Eval {}개, balanced accuracy {:.6}\n  모델 {}\n  결과: {}",
        candidates.len(),
        selection.winner + 1,
        selection.tune_rows.len(),
        tune_score,
        evaluation.predictions.len(),
        evaluation.balanced_accuracy,
        model.digest(),
        output.display()
    );
    print_analysis(&evaluation);
    Ok(())
}
fn evaluate(
    model: &Path,
    data: &Path,
    output: &Path,
    protocol: Option<&Path>,
) -> Result<(), String> {
    let model = bci_artifact::learning::load_model(model)?;
    let data = bci_artifact::learning::load_dataset(data)?;
    let report = report(&model, &data, protocol)?;
    bci_artifact::learning::write_new(output, &bci_artifact::learning::evaluation_json(&report))?;
    println!(
        "평가 완료: Eval {}개, balanced accuracy {:.6}\n  결과: {}",
        report.predictions.len(),
        report.balanced_accuracy,
        output.display()
    );
    print_analysis(&report);
    Ok(())
}
