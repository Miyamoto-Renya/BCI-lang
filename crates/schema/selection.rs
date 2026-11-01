//! Tune 후보 선택과 Eval까지 이어지는 전체 후보 계보 계약.
use crate::learning::{Assignment, MAX_ROWS, Model, Role, check_split, field};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// 선언 순서의 후보를 balanced accuracy로 비교한 증거.
#[derive(Debug, Clone, PartialEq)]
pub struct Selection {
    /// Tune catalog ID.
    pub dataset: String,
    /// 품질 제외 표본까지 포함한 Tune 분할.
    pub tune: Vec<Assignment>,
    /// 실제 지표에 기여한 고유 ID 순서의 Tune 행.
    pub tune_rows: Vec<String>,
    /// valid Tune 값·라벨·원천의 digest.
    pub tune_digest: String,
    /// 선택에 참여한 직접 학습 모델 2..16개, 명령 지정 순서.
    pub candidates: Vec<Model>,
    /// 후보별 label-major K×K confusion counts.
    pub confusion: Vec<Vec<u64>>,
    /// balanced accuracy가 최대인 첫 후보 index.
    pub winner: usize,
}

/// 클래스별 valid 분모와 고정 클래스 순서의 recall 평균을 계산한다.
pub fn metric(confusion: &[u64], classes: usize) -> Result<(Vec<u64>, f64), String> {
    if !(2..=32).contains(&classes) || confusion.len() != classes * classes {
        return Err("E0703: confusion shape은 K×K를 요구한다.".into());
    }
    let mut counts = Vec::with_capacity(classes);
    let mut balanced = 0.0;
    for class in 0..classes {
        let total = confusion[class * classes..(class + 1) * classes]
            .iter()
            .try_fold(0u64, |sum, &n| sum.checked_add(n))
            .filter(|&n| n > 0 && n <= MAX_ROWS as u64)
            .ok_or("E0704 MetricUndefined: 각 클래스의 valid 분모는 1..100000을 요구한다.")?;
        counts.push(total);
        balanced += confusion[class * classes + class] as f64 / total as f64 / classes as f64;
    }
    Ok((counts, balanced))
}

/// 여러 catalog의 ID를 짧은 위치 ID로 바꾸고 통합 분할을 검사한다.
/// 원래 ID와 역할은 모델·선택 artifact에 보존한다.
pub fn audit<'a>(parts: impl IntoIterator<Item = &'a [Assignment]>) -> Result<(), String> {
    let mut joined = Vec::new();
    for (part, rows) in parts.into_iter().enumerate() {
        if joined.len() + rows.len() > MAX_ROWS {
            return Err("E0801: 통합 선택·평가 계보 상한은 100000행이다.".into());
        }
        joined.extend(rows.iter().enumerate().map(|(i, row)| {
            let mut row = row.clone();
            row.id = format!("catalog:{part}:row:{i}");
            row
        }));
    }
    check_split(&joined)
}

/// 고정 모델에 영향을 준 모든 후보의 분할과 Tune 분할을 반환한다.
pub fn provenance(model: &Model) -> Vec<&[Assignment]> {
    match &model.selection {
        Some(selection) => selection
            .candidates
            .iter()
            .map(|m| m.split.as_slice())
            .chain(std::iter::once(selection.tune.as_slice()))
            .collect(),
        None => vec![model.split.as_slice()],
    }
}

impl Selection {
    /// 후보 계약·Tune 분모·선택 순서와 선택된 모델의 수치를 검사한다.
    pub fn validate(&self, selected: &Model) -> Result<(), String> {
        if !(2..=16).contains(&self.candidates.len())
            || self.confusion.len() != self.candidates.len()
            || self.winner >= self.candidates.len()
            || self.candidates.iter().any(|m| m.selection.is_some())
            || self.dataset.is_empty()
            || self.dataset.len() > 256
            || self.dataset.chars().any(char::is_control)
            || self.tune_digest.len() != 64
            || !self.tune_digest.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(
                "E0703: 선택은 직접 학습 후보 2..16개·Tune 정체성·후보별 confusion을 요구한다."
                    .into(),
            );
        }
        let mut ids = BTreeSet::new();
        for candidate in &self.candidates {
            candidate.validate()?;
            if candidate.features.channels != selected.features.channels
                || candidate.classes != selected.classes
                || candidate.group_by != selected.group_by
                || !ids.insert(candidate.digest())
            {
                return Err("E0703: 고유 후보의 특징·클래스 순서·그룹 정책을 맞춘다.".into());
            }
        }
        let depth = match selected.group_by.as_str() {
            "subject" => 1,
            "session" => 2,
            _ => 3,
        };
        check_split(&self.tune)?;
        let assignments: BTreeMap<_, _> = self.tune.iter().map(|a| (&a.id, a)).collect();
        if self
            .tune
            .iter()
            .any(|a| a.role != Role::Tune || a.group.len() != depth)
            || self.tune.windows(2).any(|p| p[0].id >= p[1].id)
            || self.tune_rows.is_empty()
            || self.tune_rows.len() > MAX_ROWS
            || self.tune_rows.windows(2).any(|p| p[0] >= p[1])
            || self
                .tune_rows
                .iter()
                .any(|id| !assignments.contains_key(id))
        {
            return Err(
                "E0702 TuneRoleViolation: 실제 선택 행을 정렬한 Tune 분할과 연결한다.".into(),
            );
        }
        audit(
            self.candidates
                .iter()
                .map(|m| m.split.as_slice())
                .chain(std::iter::once(self.tune.as_slice())),
        )?;
        let mut denominators = None;
        let mut best = 0;
        let mut best_score = -1.0;
        for (i, confusion) in self.confusion.iter().enumerate() {
            let (counts, score) = metric(confusion, selected.classes.len())?;
            if counts.iter().sum::<u64>() != self.tune_rows.len() as u64
                || denominators
                    .as_ref()
                    .is_some_and(|expected| expected != &counts)
            {
                return Err(
                    "E0703: 모든 후보의 클래스별 분모를 같은 valid Tune 행에 맞춘다.".into(),
                );
            }
            denominators = Some(counts);
            if score > best_score {
                best = i;
                best_score = score;
            }
        }
        let mut base = selected.clone();
        base.selection = None;
        if best != self.winner || base.digest() != self.candidates[self.winner].digest() {
            return Err("E0703: 최대 balanced accuracy의 첫 후보를 선택 모델과 연결한다.".into());
        }
        Ok(())
    }

    /// 전체 후보와 선택 증거를 순서대로 묶는 의미 digest.
    pub fn digest(&self) -> String {
        let mut hash = Sha256::new();
        field(&mut hash, b"bci-selection-balanced-accuracy-first/0.1");
        for s in [&self.dataset, &self.tune_digest] {
            field(&mut hash, s.as_bytes());
        }
        field(&mut hash, format!("{:?}", self.tune).as_bytes());
        field(&mut hash, format!("{:?}", self.tune_rows).as_bytes());
        for candidate in &self.candidates {
            field(&mut hash, candidate.digest().as_bytes());
        }
        field(&mut hash, format!("{:?}", self.confusion).as_bytes());
        hash.update((self.winner as u64).to_le_bytes());
        format!("{:x}", hash.finalize())
    }
}
