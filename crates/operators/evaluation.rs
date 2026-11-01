//! 계층별 동일 가중 confusion과 고정 seed의 cluster bootstrap.
use bci_schema::{
    evaluation::{
        Analysis, Bootstrap, CatalogRow, Cluster, Group, Interval, Metrics, Protocol, Resampling,
    },
    learning::{Dataset, Evaluation, Model, Role},
};
use std::collections::BTreeMap;

struct Accumulator {
    matrix: Vec<f64>,
    included: usize,
    excluded: usize,
    children: usize,
}
type Groups = BTreeMap<Vec<String>, Accumulator>;

fn entry<'a>(
    groups: &'a mut Groups,
    key: &[String],
    k: usize,
    cells: &mut usize,
) -> Result<&'a mut Accumulator, String> {
    if !groups.contains_key(key) {
        *cells += k * k;
        if *cells > 1_000_000 {
            return Err("E0801: 평가 집계 histogram 예약 상한은 1000000개 성분이다.".into());
        }
        groups.insert(
            key.to_vec(),
            Accumulator {
                matrix: vec![0.0; k * k],
                included: 0,
                excluded: 0,
                children: 0,
            },
        );
    }
    Ok(groups.get_mut(key).expect("inserted group"))
}
fn scores(matrix: &[f64], k: usize) -> (Option<f64>, Option<f64>) {
    let total: f64 = matrix.iter().sum();
    if total == 0.0 {
        return (None, None);
    }
    let accuracy = (0..k).map(|i| matrix[i * k + i]).sum::<f64>() / total;
    let mut balanced = 0.0;
    for class in 0..k {
        let denominator: f64 = matrix[class * k..(class + 1) * k].iter().sum();
        if denominator == 0.0 {
            return (Some(accuracy), None);
        }
        balanced += matrix[class * k + class] / denominator / k as f64;
    }
    (Some(accuracy), Some(balanced))
}
fn metrics(matrix: &[f64], k: usize) -> Metrics {
    let total: f64 = matrix.iter().sum();
    let confusion: Vec<_> = matrix
        .iter()
        .map(|&n| if total > 0.0 { n / total } else { 0.0 })
        .collect();
    let denominators = confusion.chunks(k).map(|row| row.iter().sum()).collect();
    let (accuracy, balanced_accuracy) = scores(&confusion, k);
    Metrics {
        confusion,
        denominators,
        accuracy,
        balanced_accuracy,
    }
}

/// 독립 Eval 검사를 수행하고 요청한 계층 집계·재표집 결과를 연결한다.
pub fn evaluate(model: &Model, data: &Dataset, protocol: &Protocol) -> Result<Evaluation, String> {
    protocol.validate(&data.group_by)?;
    let depth = protocol
        .aggregation
        .iter()
        .map(|u| u.depth())
        .chain(protocol.bootstrap.iter().map(|b| b.unit.depth()))
        .max()
        .unwrap_or(0);
    if let Some(row) = data
        .rows
        .iter()
        .find(|r| r.assignment.role == Role::Eval && r.hierarchy.len() < depth)
    {
        return Err(format!(
            "E0703: Eval 행 {}에 요청한 subject/session/trial 계층 key를 제공한다.",
            row.assignment.id
        ));
    }
    let mut evaluation = crate::learning::evaluate(model, data)?;
    let k = model.classes.len();
    let mut predictions = evaluation.predictions.iter();
    let mut catalog = Vec::new();
    let mut current = Groups::new();
    let mut cells = 0;
    let first_depth = protocol
        .aggregation
        .first()
        .map(|u| u.depth())
        .or_else(|| protocol.bootstrap.as_ref().map(|b| b.unit.depth()))
        .unwrap_or(0);
    for row in data.rows.iter().filter(|r| r.assignment.role == Role::Eval) {
        let included = row.valid.iter().all(|&v| v == 1);
        catalog.push(CatalogRow {
            assignment: row.assignment.clone(),
            hierarchy: row.hierarchy.clone(),
            included,
        });
        let group = entry(&mut current, &row.hierarchy[..first_depth], k, &mut cells)?;
        if included {
            let prediction = predictions
                .next()
                .ok_or("E0703: valid Eval과 예측 행을 연결한다.")?;
            if prediction.id != row.assignment.id {
                return Err("E0703: Eval 행과 예측 순서를 맞춘다.".into());
            }
            group.matrix[prediction.label * k + prediction.predicted] += 1.0;
            group.included += 1;
            group.children += 1;
        } else {
            group.excluded += 1;
        }
    }
    let mut groups = Vec::new();
    for (stage, &unit) in protocol.aggregation.iter().enumerate() {
        if stage > 0 {
            let mut next = Groups::new();
            for (key, child) in current {
                let parent = entry(&mut next, &key[..unit.depth()], k, &mut cells)?;
                parent.included += child.included;
                parent.excluded += child.excluded;
                if child.included > 0 {
                    parent.children += 1;
                    for (sum, value) in parent.matrix.iter_mut().zip(child.matrix) {
                        *sum += value;
                    }
                }
            }
            current = next;
        }
        for (key, group) in &mut current {
            if group.children > 0 {
                for value in &mut group.matrix {
                    *value /= group.children as f64;
                }
            }
            groups.push(Group {
                unit,
                key: key.clone(),
                included: group.included,
                excluded: group.excluded,
                children: group.children,
                metrics: metrics(&group.matrix, k),
            });
        }
    }
    let mut total = vec![0.0; k * k];
    let mut clusters = Groups::new();
    for (key, group) in current {
        for (sum, &value) in total.iter_mut().zip(&group.matrix) {
            *sum += value;
        }
        if let Some(bootstrap) = &protocol.bootstrap {
            let cluster = entry(&mut clusters, &key[..bootstrap.unit.depth()], k, &mut cells)?;
            cluster.included += group.included;
            cluster.excluded += group.excluded;
            cluster.children += if protocol.aggregation.is_empty() {
                group.included
            } else {
                usize::from(group.included > 0)
            };
            for (sum, value) in cluster.matrix.iter_mut().zip(group.matrix) {
                *sum += value;
            }
        }
    }
    evaluation.analysis = Some(Box::new(Analysis {
        protocol: protocol.clone(),
        eval_digest: data.role_digest(Role::Eval),
        catalog,
        groups,
        estimate: metrics(&total, k),
        resampling: protocol
            .bootstrap
            .as_ref()
            .map(|b| bootstrap(&clusters, k, b))
            .transpose()?,
    }));
    Ok(evaluation)
}

struct SplitMix64(u64);
impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }
    fn index(&mut self, bound: usize) -> usize {
        let bound = bound as u64;
        let threshold = bound.wrapping_neg() % bound;
        loop {
            let value = self.next();
            if value >= threshold {
                return (value % bound) as usize;
            }
        }
    }
}
fn interval(mut values: Vec<f64>, confidence: f64) -> Result<Interval, String> {
    if values.len() < 2 {
        return Err("E0704 MetricUndefined: percentile 구간은 지표가 정의되는 bootstrap 반복 2개 이상을 요구한다.".into());
    }
    values.sort_by(f64::total_cmp);
    let percentile = |p: f64| {
        let position = (values.len() - 1) as f64 * p;
        let lower = position.floor() as usize;
        let upper = position.ceil() as usize;
        values[lower] + (values[upper] - values[lower]) * (position - lower as f64)
    };
    Ok(Interval {
        lower: percentile((1.0 - confidence) / 2.0),
        upper: percentile((1.0 + confidence) / 2.0),
        defined: values.len(),
    })
}
fn bootstrap(groups: &Groups, k: usize, spec: &Bootstrap) -> Result<Resampling, String> {
    let clusters: Vec<_> = groups.iter().filter(|(_, g)| g.children > 0).collect();
    if clusters.len() < 2 {
        return Err("E0704 MetricUndefined: bootstrap은 valid 자료를 가진 독립 cluster 2개 이상을 요구한다.".into());
    }
    if clusters.len() as u64 * k as u64 * k as u64 * spec.iterations as u64 > 200_000_000 {
        return Err("E0801: bootstrap 작업량 cluster 수×K×K×반복 수 상한은 200000000이다.".into());
    }
    let mut rng = SplitMix64(spec.seed);
    let mut matrix = vec![0.0; k * k];
    let mut accuracy = Vec::with_capacity(spec.iterations);
    let mut balanced_accuracy = Vec::with_capacity(spec.iterations);
    for _ in 0..spec.iterations {
        matrix.fill(0.0);
        for _ in 0..clusters.len() {
            let cluster = clusters[rng.index(clusters.len())].1;
            for (sum, &value) in matrix.iter_mut().zip(&cluster.matrix) {
                *sum += value;
            }
        }
        let (acc, balanced) = scores(&matrix, k);
        accuracy.push(acc.ok_or("E0704: bootstrap valid 분모를 확인한다.")?);
        balanced_accuracy.push(balanced);
    }
    Ok(Resampling {
        clusters: clusters
            .into_iter()
            .map(|(key, g)| Cluster {
                key: key.clone(),
                mass: g.children,
            })
            .collect(),
        accuracy_interval: interval(accuracy.clone(), spec.confidence)?,
        balanced_accuracy_interval: interval(
            balanced_accuracy.iter().flatten().copied().collect(),
            spec.confidence,
        )?,
        accuracy,
        balanced_accuracy,
    })
}
