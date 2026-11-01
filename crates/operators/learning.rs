//! Fit 정규화·batch softmax, Tune 선택과 독립 Eval 평가.
use bci_schema::learning::{Dataset, Evaluation, Model, Prediction, Role};
use bci_schema::selection::{Selection, audit, metric, provenance};

/// valid Fit 표본만으로 정규화 통계와 분류 weight를 학습한다.
pub fn fit(data: &Dataset) -> Result<Model, String> {
    data.validate()?;
    let fit: Vec<_> = data
        .rows
        .iter()
        .filter(|r| r.assignment.role == Role::Fit && r.valid.iter().all(|&v| v == 1))
        .collect();
    let d = data.features.channels.len();
    let k = data.classes.len();
    let mut counts = vec![0usize; k];
    for row in &fit {
        counts[row.label] += 1;
    }
    if counts.contains(&0) {
        return Err("E0702: valid Fit 표본을 각 클래스에 한 개 이상 배정한다.".into());
    }
    if fit
        .len()
        .checked_mul(d)
        .and_then(|n| n.checked_mul(k))
        .and_then(|n| n.checked_mul(data.training.iterations))
        .is_none_or(|n| n > 200_000_000)
    {
        return Err("E0801: 학습 작업량 N×D×K×iterations 상한은 200000000이다.".into());
    }
    let mut mean = vec![0.0; d];
    let mut m2 = vec![0.0; d];
    for (i, row) in fit.iter().enumerate() {
        for j in 0..d {
            let delta = row.values[j] - mean[j];
            mean[j] += delta / (i + 1) as f64;
            m2[j] += delta * (row.values[j] - mean[j]);
        }
    }
    let scale = m2
        .iter()
        .map(|&v| {
            if v == 0.0 {
                1.0
            } else {
                (v / fit.len() as f64).sqrt()
            }
        })
        .collect();
    let mut model = Model {
        features: data.features.clone(),
        classes: data.classes.clone(),
        mean,
        scale,
        weights: vec![0.0; d * k],
        bias: vec![0.0; k],
        training: data.training.clone(),
        dataset: data.id.clone(),
        group_by: data.group_by.clone(),
        split: data.rows.iter().map(|r| r.assignment.clone()).collect(),
        fit_rows: fit.iter().map(|r| r.assignment.id.clone()).collect(),
        fit_digest: data.fit_digest(),
        selection: None,
    };
    model.validate()?;
    let mut probabilities = vec![0.0; k];
    let mut gradient = vec![0.0; k * d];
    let mut bias = vec![0.0; k];
    for _ in 0..data.training.iterations {
        gradient.fill(0.0);
        bias.fill(0.0);
        for row in &fit {
            model.predict(&row.values, &mut probabilities)?;
            for class in 0..k {
                let error = probabilities[class] - f64::from(class == row.label);
                bias[class] += error;
                for j in 0..d {
                    gradient[class * d + j] +=
                        error * ((row.values[j] - model.mean[j]) / model.scale[j]);
                }
            }
        }
        for (weight, gradient) in model.weights.iter_mut().zip(&gradient) {
            *weight -= data.training.learning_rate
                * (*gradient / fit.len() as f64 + data.training.l2 * *weight);
        }
        for (value, gradient) in model.bias.iter_mut().zip(&bias) {
            *value -= data.training.learning_rate * gradient / fit.len() as f64;
        }
        if model
            .weights
            .iter()
            .chain(&model.bias)
            .any(|v| !v.is_finite())
        {
            return Err(
                "E0903: 학습 weight가 유한 수치 범위를 초과했다. 특징 크기·학습률을 조정한다."
                    .into(),
            );
        }
    }
    Ok(model)
}

/// 학습 그룹·원천과 Eval의 분리를 재검사하고 클래스별 분모를 계산한다.
pub fn evaluate(model: &Model, data: &Dataset) -> Result<Evaluation, String> {
    model.validate()?;
    data.validate()?;
    if model.features.channels != data.features.channels
        || model.classes != data.classes
        || model.group_by != data.group_by
    {
        return Err("E0703: 평가 특징·클래스 순서·그룹 정책을 모델과 맞춘다.".into());
    }
    let eval: Vec<_> = data
        .rows
        .iter()
        .filter(|r| r.assignment.role == Role::Eval)
        .map(|r| r.assignment.clone())
        .collect();
    let mut parts = provenance(model);
    parts.push(&eval);
    audit(parts)?;
    score(model, data, Role::Eval)
}

fn score(model: &Model, data: &Dataset, role: Role) -> Result<Evaluation, String> {
    let k = model.classes.len();
    let rows = data
        .rows
        .iter()
        .filter(|r| r.assignment.role == role)
        .count();
    if rows as u64 * k as u64 * model.features.channels.len() as u64 > 200_000_000 {
        return Err("E0801: 지표 계산 작업량 N×D×K 상한은 200000000이다.".into());
    }
    let mut result = Evaluation {
        analysis: None,
        model_digest: model.digest(),
        dataset: data.id.clone(),
        classes: model.classes.clone(),
        confusion: vec![0; k * k],
        balanced_accuracy: 0.0,
        accuracy: 0.0,
        predictions: Vec::new(),
        excluded: Vec::new(),
        lineage: data
            .rows
            .iter()
            .filter(|r| r.assignment.role == role)
            .map(|r| r.assignment.clone())
            .collect(),
    };
    for row in data.rows.iter().filter(|r| r.assignment.role == role) {
        if row.valid.contains(&0) {
            result.excluded.push(row.assignment.id.clone());
            continue;
        }
        let mut probabilities = vec![0.0; k];
        model.predict(&row.values, &mut probabilities)?;
        let predicted = winner(&probabilities);
        result.confusion[row.label * k + predicted] += 1;
        result.predictions.push(Prediction {
            id: row.assignment.id.clone(),
            label: row.label,
            predicted,
            probabilities,
        });
    }
    if let Some(class) = result
        .confusion
        .chunks(k)
        .position(|counts| counts.iter().all(|&n| n == 0))
    {
        return Err(format!(
            "E0704 MetricUndefined: {role:?} 클래스 {}의 valid 표본 분모가 0이다.",
            model.classes[class]
        ));
    }
    let (_, balanced) = metric(&result.confusion, k)?;
    let correct: u64 = (0..k).map(|i| result.confusion[i * k + i]).sum();
    result.balanced_accuracy = balanced;
    result.accuracy = correct as f64 / result.predictions.len() as f64;
    Ok(result)
}

/// Tune의 balanced accuracy로 후보를 선택하고 전체 후보 계보를 모델에 보존한다.
pub fn select(candidates: &[Model], data: &Dataset) -> Result<Model, String> {
    data.validate()?;
    if !(2..=16).contains(&candidates.len()) || candidates.iter().any(|m| m.selection.is_some()) {
        return Err("E0703: 선택 입력에 직접 학습 후보 2..16개를 지정한다.".into());
    }
    for model in candidates {
        model.validate()?;
        if model.features.channels != data.features.channels
            || model.classes != data.classes
            || model.group_by != data.group_by
        {
            return Err("E0703: 후보와 Tune의 특징·클래스 순서·그룹 정책을 맞춘다.".into());
        }
    }
    let split: Vec<_> = data.rows.iter().map(|r| r.assignment.clone()).collect();
    audit(
        candidates
            .iter()
            .map(|m| m.split.as_slice())
            .chain(std::iter::once(split.as_slice())),
    )?;
    let tune: Vec<_> = data
        .rows
        .iter()
        .filter(|r| r.assignment.role == Role::Tune)
        .collect();
    if tune.len() as u64
        * candidates.len() as u64
        * data.classes.len() as u64
        * data.features.channels.len() as u64
        > 200_000_000
    {
        return Err("E0801: 선택 작업량 N×후보 수×D×K 상한은 200000000이다.".into());
    }
    let mut confusion = Vec::new();
    let mut winner = 0;
    let mut best = -1.0;
    for (i, candidate) in candidates.iter().enumerate() {
        let report = score(candidate, data, Role::Tune)?;
        if report.balanced_accuracy > best {
            winner = i;
            best = report.balanced_accuracy;
        }
        confusion.push(report.confusion);
    }
    let mut selected = candidates[winner].clone();
    selected.selection = Some(Box::new(Selection {
        dataset: data.id.clone(),
        tune: tune.iter().map(|r| r.assignment.clone()).collect(),
        tune_rows: tune
            .iter()
            .filter(|r| r.valid.iter().all(|&v| v == 1))
            .map(|r| r.assignment.id.clone())
            .collect(),
        tune_digest: data.role_digest(Role::Tune),
        candidates: candidates.to_vec(),
        confusion,
        winner,
    }));
    selected.validate()?;
    Ok(selected)
}

/// 동점은 class schema의 앞선 index로 결정한다.
pub(crate) fn winner(values: &[f64]) -> usize {
    let mut winner = 0;
    for i in 1..values.len() {
        if values[i] > values[winner] {
            winner = i;
        }
    }
    winner
}
