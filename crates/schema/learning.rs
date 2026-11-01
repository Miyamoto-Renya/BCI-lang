//! 학습 역할·원천 계보·고정 모델과 예측의 공통 계약.
use crate::{
    types::{Channel, ChannelSchema, DType, StreamType, ValueType},
    unit::Unit,
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// 표준 batch softmax solver와 수치 계약 판본.
pub const PROFILE: &str = "bci-standardized-softmax/0.1";
/// dataset의 최대 표본 수.
pub const MAX_ROWS: usize = 100_000;

/// 역할 사이의 통계·선택 경계.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// 매개변수와 정규화 통계를 학습한다.
    Fit,
    /// 후보 모델의 선택 지표를 계산한다.
    Tune,
    /// 고정 모델을 평가한다.
    Eval,
}
/// 같은 측정 원천의 닫힌 기여 구간.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ancestor {
    /// 파일 복사에도 유지하는 catalog 원천 ID.
    pub source: String,
    /// 획득 epoch.
    pub epoch: u64,
    /// 첫 기여 index.
    pub first: u64,
    /// 마지막 기여 index.
    pub last: u64,
}
/// 분할의 행·독립 그룹·역할과 전처리 조상.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    /// dataset 내부 고유 행 ID.
    pub id: String,
    /// subject/session/trial 정책으로 구성한 그룹 key.
    pub group: Vec<String>,
    /// Fit·Tune·Eval 중 배정한 역할.
    pub role: Role,
    /// 전처리가 읽은 전체 원천 구간.
    pub ancestors: Vec<Ancestor>,
}
/// 클래스 라벨과 채널 순서의 특징.
#[derive(Debug, Clone)]
pub struct Sample {
    /// subject/session/trial 순서의 catalog key. 분할 key를 접두사로 가진다.
    pub hierarchy: Vec<String>,
    /// 검증할 분할 항목.
    pub assignment: Assignment,
    /// class schema의 index.
    pub label: usize,
    /// 특징 벡터.
    pub values: Vec<f64>,
    /// 채널별 0 또는 1 품질.
    pub valid: Vec<u8>,
}
/// 학습 전에 고정하는 solver 설정.
#[derive(Debug, Clone, PartialEq)]
pub struct Training {
    /// 전체 Fit batch 반복 수.
    pub iterations: usize,
    /// 고정 gradient descent 간격.
    pub learning_rate: f64,
    /// weight에 적용하는 L2 계수.
    pub l2: f64,
}
impl Training {
    /// 자원·수치 정의역을 검사한다.
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=10000).contains(&self.iterations)
            || !self.learning_rate.is_finite()
            || self.learning_rate <= 0.0
            || self.learning_rate > 1.0
            || !self.l2.is_finite()
            || self.l2 < 0.0
            || self.l2 > 100.0
        {
            return Err(
                "E0702: iterations 1..10000, learning_rate (0,1], l2 [0,100]을 지정한다.".into(),
            );
        }
        Ok(())
    }
}
/// 특징 테이블과 고정 분할 protocol.
#[derive(Debug, Clone)]
pub struct Dataset {
    /// catalog dataset 정체성.
    pub id: String,
    /// 순서·측정량·단위·기준을 포함한 특징 계약.
    pub features: ChannelSchema,
    /// 순서 있는 라벨 이름.
    pub classes: Vec<String>,
    /// subject, session 또는 trial.
    pub group_by: String,
    /// Eval 관측 전에 고정한 학습 설정.
    pub training: Training,
    /// 고유 ID로 정렬한 특징 표본.
    pub rows: Vec<Sample>,
}

/// 고정 클래스 순서를 검사한다.
pub fn classes_valid(classes: &[String]) -> bool {
    (2..=32).contains(&classes.len())
        && classes
            .iter()
            .all(|s| !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control))
        && classes.iter().collect::<BTreeSet<_>>().len() == classes.len()
}
fn key_valid(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}
fn features_valid(features: &ChannelSchema) -> bool {
    let mut ids = BTreeSet::new();
    (1..=256).contains(&features.channels.len())
        && features.channels.iter().all(|c| {
            key_valid(&c.id)
                && ids.insert(&c.id)
                && key_valid(&c.quantity)
                && key_valid(&c.reference)
                && c.unit.scale.is_positive()
        })
}
/// 그룹 분리와 같은 원천의 겹치는 support를 검사한다.
pub fn check_split(assignments: &[Assignment]) -> Result<(), String> {
    if assignments.is_empty() || assignments.len() > MAX_ROWS {
        return Err("E0701: 분할 행 수는 1..100000을 요구한다.".into());
    }
    let mut ids = BTreeSet::new();
    let mut groups = BTreeMap::new();
    let mut ranges = Vec::new();
    for row in assignments {
        if !key_valid(&row.id)
            || !ids.insert(&row.id)
            || row.group.is_empty()
            || row.group.len() > 3
            || row.group.iter().any(|g| !key_valid(g))
            || !(1..=64).contains(&row.ancestors.len())
        {
            return Err(format!(
                "E0701: {}의 고유 ID·그룹·ancestor 1..64개를 확인한다.",
                row.id
            ));
        }
        if let Some((role, id)) = groups.insert(&row.group, (row.role, &row.id))
            && role != row.role
        {
            return Err(format!(
                "E0701 SplitOverlap: {id}와 {}가 같은 그룹 {:?}에서 서로 다른 역할로 나뉜다.",
                row.id, row.group
            ));
        }
        for range in &row.ancestors {
            if !key_valid(&range.source) || range.first > range.last || range.last > i64::MAX as u64
            {
                return Err(format!("E0701: {}의 원천 ID·닫힌 구간을 확인한다.", row.id));
            }
            ranges.push((range, row));
        }
    }
    ranges.sort_by(|a, b| a.0.cmp(b.0));
    let mut component: Option<(&Ancestor, &Assignment, u64)> = None;
    for (range, row) in ranges {
        if let Some((start, previous, end)) = component
            && start.source == range.source
            && start.epoch == range.epoch
            && range.first <= end
        {
            if previous.group != row.group || previous.role != row.role {
                return Err(format!(
                    "E0701 SplitOverlap: {}와 {}의 원천 {} epoch {} 구간이 겹친다.",
                    previous.id, row.id, range.source, range.epoch
                ));
            }
            component = Some((start, previous, end.max(range.last)));
        } else {
            component = Some((range, row, range.last));
        }
    }
    Ok(())
}
impl Dataset {
    /// 실제 valid Fit 행의 값·라벨·원천과 특징 계약을 식별한다.
    pub fn fit_digest(&self) -> String {
        self.role_digest(Role::Fit)
    }
    /// 지정 역할의 valid 값·라벨·원천과 특징 계약을 식별한다.
    pub fn role_digest(&self, role: Role) -> String {
        let mut hash = Sha256::new();
        field(
            &mut hash,
            match role {
                Role::Fit => b"bci-fit-data/0.1",
                Role::Tune => b"bci-tune-data/0.1",
                Role::Eval => b"bci-eval-data/0.1",
            },
        );
        field(&mut hash, self.id.as_bytes());
        field(&mut hash, format!("{:?}", self.features).as_bytes());
        field(&mut hash, format!("{:?}", self.classes).as_bytes());
        for row in self
            .rows
            .iter()
            .filter(|r| r.assignment.role == role && r.valid.iter().all(|&v| v == 1))
        {
            field(&mut hash, format!("{:?}", row.assignment).as_bytes());
            hash.update((row.label as u64).to_le_bytes());
            for value in &row.values {
                hash.update(value.to_bits().to_le_bytes());
            }
        }
        format!("{:x}", hash.finalize())
    }
    /// schema·라벨·품질·분할과 학습 작업량을 검사한다.
    pub fn validate(&self) -> Result<(), String> {
        self.training.validate()?;
        if !key_valid(&self.id)
            || !classes_valid(&self.classes)
            || !(1..=256).contains(&self.features.channels.len())
            || !["subject", "session", "trial"].contains(&self.group_by.as_str())
        {
            return Err(
                "E0702: dataset ID·클래스 2..32개·특징 1..256개·그룹 정책을 확인한다.".into(),
            );
        }
        let depth = match self.group_by.as_str() {
            "subject" => 1,
            "session" => 2,
            _ => 3,
        };
        let mut feature_ids = BTreeSet::new();
        if self
            .features
            .channels
            .iter()
            .any(|c| !key_valid(&c.id) || !feature_ids.insert(&c.id) || !c.unit.scale.is_positive())
        {
            return Err("E0703: 고유 특징 ID와 양의 단위 배율을 지정한다.".into());
        }
        if self
            .rows
            .windows(2)
            .any(|p| p[0].assignment.id >= p[1].assignment.id)
        {
            return Err("E0701: dataset 행을 고유 ID 순서로 정렬한다.".into());
        }
        for row in &self.rows {
            if row.hierarchy.len() < depth
                || row.hierarchy.len() > 3
                || row.hierarchy.iter().any(|key| !key_valid(key))
                || !row.hierarchy.starts_with(&row.assignment.group)
            {
                return Err(
                    "E0701: catalog 계층을 subject/session/trial 순서와 분할 그룹 접두사에 맞춘다."
                        .into(),
                );
            }
            if row.values.len() != self.features.channels.len()
                || row.valid.len() != row.values.len()
                || row.valid.iter().any(|&v| v > 1)
                || row.values.iter().any(|v| !v.is_finite())
                || row.label >= self.classes.len()
                || row.assignment.group.len() != depth
            {
                return Err(format!(
                    "E0702: {}의 특징·품질·라벨·그룹 깊이를 계약과 맞춘다.",
                    row.assignment.id
                ));
            }
        }
        check_split(
            &self
                .rows
                .iter()
                .map(|r| r.assignment.clone())
                .collect::<Vec<_>>(),
        )
    }
}

/// 실행 준비에서 고정하는 정규화·softmax 모델과 완전한 분할 계보.
#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    /// 입력 특징 계약.
    pub features: ChannelSchema,
    /// 출력 클래스 순서.
    pub classes: Vec<String>,
    /// Fit population mean.
    pub mean: Vec<f64>,
    /// Fit population std; 상수 특징은 1.
    pub scale: Vec<f64>,
    /// class-major 선형 weight.
    pub weights: Vec<f64>,
    /// 클래스별 bias.
    pub bias: Vec<f64>,
    /// 학습 전에 고정한 solver 설정.
    pub training: Training,
    /// catalog dataset ID.
    pub dataset: String,
    /// 독립 그룹 정책.
    pub group_by: String,
    /// 역할과 ancestor 구간의 검증 가능한 분할.
    pub split: Vec<Assignment>,
    /// 실제 valid Fit 표본 ID.
    pub fit_rows: Vec<String>,
    /// 실제 Fit 값·라벨·스키마의 의미 digest.
    pub fit_digest: String,
    /// Tune 선택의 후보·지표·계보. 직접 학습 모델은 None이다.
    pub selection: Option<Box<crate::selection::Selection>>,
}
impl Model {
    /// shape·유한 수치·양의 정규화와 역할 계보를 검사한다.
    pub fn validate(&self) -> Result<(), String> {
        self.training.validate()?;
        let d = self.features.channels.len();
        let k = self.classes.len();
        if !features_valid(&self.features)
            || !classes_valid(&self.classes)
            || self.mean.len() != d
            || self.scale.len() != d
            || self.weights.len() != d * k
            || self.bias.len() != k
            || self.scale.iter().any(|s| !s.is_finite() || *s <= 0.0)
            || self
                .mean
                .iter()
                .chain(&self.weights)
                .chain(&self.bias)
                .any(|v| !v.is_finite())
            || !key_valid(&self.dataset)
            || !["subject", "session", "trial"].contains(&self.group_by.as_str())
            || self.fit_digest.len() != 64
            || !self.fit_digest.bytes().all(|c| c.is_ascii_hexdigit())
        {
            return Err(
                "E0703 ModelSchemaMismatch: 모델 shape·수치·클래스·학습 정체성을 확인한다.".into(),
            );
        }
        check_split(&self.split)?;
        if self.split.windows(2).any(|p| p[0].id >= p[1].id) {
            return Err("E0701: 모델 분할을 고유 ID 순서로 저장한다.".into());
        }
        let depth = match self.group_by.as_str() {
            "subject" => 1,
            "session" => 2,
            _ => 3,
        };
        let assignments: BTreeMap<_, _> = self.split.iter().map(|a| (&a.id, a)).collect();
        if self.split.iter().any(|a| a.group.len() != depth)
            || self.fit_rows.is_empty()
            || self.fit_rows.windows(2).any(|p| p[0] >= p[1])
            || self
                .fit_rows
                .iter()
                .any(|id| assignments.get(id).is_none_or(|a| a.role != Role::Fit))
        {
            return Err("E0702 FitRoleViolation: 실제 학습 행을 Fit 분할과 연결한다.".into());
        }
        if let Some(selection) = &self.selection {
            selection.validate(self)?;
        }
        Ok(())
    }
    /// 판본별 의미 필드·정확한 f64 bit·순서를 고정한 모델 digest.
    pub fn digest(&self) -> String {
        let mut hash = Sha256::new();
        field(&mut hash, PROFILE.as_bytes());
        field(&mut hash, format!("{:?}", self.features).as_bytes());
        for class in &self.classes {
            field(&mut hash, class.as_bytes());
        }
        for values in [&self.mean, &self.scale, &self.weights, &self.bias] {
            hash.update((values.len() as u64).to_le_bytes());
            for value in values {
                hash.update(value.to_bits().to_le_bytes());
            }
        }
        field(&mut hash, format!("{:?}", self.training).as_bytes());
        for value in [&self.dataset, &self.group_by, &self.fit_digest] {
            field(&mut hash, value.as_bytes());
        }
        field(&mut hash, format!("{:?}", self.split).as_bytes());
        field(&mut hash, format!("{:?}", self.fit_rows).as_bytes());
        if let Some(selection) = &self.selection {
            field(&mut hash, selection.digest().as_bytes());
        }
        format!("{:x}", hash.finalize())
    }
    /// 특징 타입과 학습 schema를 대조하고 클래스 확률 타입을 만든다.
    pub fn output(&self, input: &StreamType) -> Result<StreamType, String> {
        self.validate()?;
        if input.value != ValueType::Vector(DType::F64, self.features.channels.len())
            || input.schema.channels != self.features.channels
        {
            return Err("E0703 ModelSchemaMismatch: 예측 입력의 특징 순서·측정량·단위·기준을 모델과 맞춘다.".into());
        }
        let reference = format!("model:{}", self.digest());
        let mut output = input.clone();
        output.value = ValueType::Probability(self.classes.clone());
        output.schema = ChannelSchema {
            name: "softmax_probability".into(),
            channels: self
                .classes
                .iter()
                .map(|id| Channel {
                    id: id.clone(),
                    quantity: "probability.softmax".into(),
                    unit: Unit::scalar(),
                    reference: reference.clone(),
                })
                .collect(),
        };
        Ok(output)
    }
    /// 준비한 출력 slice에 안정화 softmax를 계산한다.
    pub fn predict(&self, values: &[f64], output: &mut [f64]) -> Result<(), String> {
        let d = self.mean.len();
        if values.len() != d || output.len() != self.classes.len() {
            return Err("예측 입출력 layout을 모델과 맞춘다.".into());
        }
        for (k, score) in output.iter_mut().enumerate() {
            *score = self.bias[k];
            for (j, &x) in values.iter().enumerate() {
                *score += self.weights[k * d + j] * ((x - self.mean[j]) / self.scale[j]);
            }
            if !score.is_finite() {
                return Err(
                    "예측 logit이 유한 수치 범위를 초과했다. 특징·정규화·weight를 확인한다.".into(),
                );
            }
        }
        let maximum = output.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let mut sum = 0.0;
        for score in output.iter_mut() {
            *score = (*score - maximum).exp();
            sum += *score;
        }
        for score in output.iter_mut() {
            *score /= sum;
        }
        probability(output)
    }
    /// 실행 모델·기술자·작업 공간의 보수적 예약량.
    pub fn memory(&self) -> Result<usize, String> {
        let metadata = format!("{self:?}").len();
        metadata
            .checked_mul(4)
            .and_then(|n| n.checked_add(4096))
            .ok_or_else(|| "모델 예약량 범위를 초과했다.".into())
    }
}
/// 의미 field의 길이와 bytes를 hash에 추가한다.
pub fn field(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}
/// 유한 비음수·합 1의 확률을 검사한다.
pub fn probability(values: &[f64]) -> Result<(), String> {
    if !(2..=32).contains(&values.len())
        || values
            .iter()
            .any(|v| !v.is_finite() || *v < 0.0 || *v > 1.0)
        || (values.iter().sum::<f64>() - 1.0).abs() > 1e-9
    {
        return Err("확률은 유한 0..1 값과 합 1±1e-9를 요구한다.".into());
    }
    Ok(())
}
/// threshold 타입과 최소 신뢰도를 검사한다.
pub fn decision(input: &StreamType, minimum: f64) -> Result<StreamType, String> {
    typed_stream(input)?;
    let ValueType::Probability(classes) = &input.value else {
        return Err("판정 입력에 Probability 포트를 연결한다.".into());
    };
    if !classes_valid(classes) || !minimum.is_finite() || !(0.0..=1.0).contains(&minimum) {
        return Err("판정 minimum은 0..1의 유한 무차원 수치를 요구한다.".into());
    }
    let mut output = input.clone();
    output.value = ValueType::Decision(classes.clone());
    let reference = input
        .schema
        .channels
        .first()
        .ok_or("확률 클래스 스키마를 지정한다.")?
        .reference
        .clone();
    output.schema = ChannelSchema {
        name: "decision".into(),
        channels: [
            ("selection", "decision.class_index"),
            ("confidence", "probability.maximum"),
        ]
        .into_iter()
        .map(|(id, quantity)| Channel {
            id: id.into(),
            quantity: quantity.into(),
            unit: Unit::scalar(),
            reference: reference.clone(),
        })
        .collect(),
    };
    Ok(output)
}

/// 확률·판정 포트의 클래스·채널·모델 정체성을 확인한다.
pub fn typed_stream(stream: &StreamType) -> Result<(), String> {
    let (classes, expected): (&[String], Vec<(&str, &str)>) = match &stream.value {
        ValueType::Probability(c) => (
            c,
            c.iter()
                .map(|s| (s.as_str(), "probability.softmax"))
                .collect(),
        ),
        ValueType::Decision(c) => (
            c,
            vec![
                ("selection", "decision.class_index"),
                ("confidence", "probability.maximum"),
            ],
        ),
        _ => return Ok(()),
    };
    let reference = stream
        .schema
        .channels
        .first()
        .map(|c| c.reference.as_str())
        .unwrap_or("");
    let model = reference.strip_prefix("model:").unwrap_or("");
    if !classes_valid(classes)
        || expected.len() != stream.schema.channels.len()
        || model.len() != 64
        || !model.bytes().all(|c| c.is_ascii_hexdigit())
        || stream
            .schema
            .channels
            .iter()
            .zip(expected)
            .any(|(c, (id, q))| {
                c.id != id
                    || c.quantity != q
                    || c.unit != Unit::scalar()
                    || c.reference != reference
            })
    {
        return Err("확률·판정의 클래스 순서·스키마·모델 digest를 맞춘다.".into());
    }
    Ok(())
}
/// 기록·runtime에서 확률·판정의 수치와 무효 payload를 검사한다.
pub fn typed_frame(ty: &ValueType, frame: &crate::execution::Frame) -> Result<(), String> {
    let classes = match ty {
        ValueType::Probability(c) | ValueType::Decision(c) => c,
        _ => return Ok(()),
    };
    let invalid = frame.valid.contains(&0);
    if invalid
        && (frame.valid.iter().any(|&v| v != 0)
            || frame.flags & crate::execution::INVALID_INPUT == 0)
    {
        return Err("예측 무효 레코드는 전체 mask 0과 InvalidInput을 요구한다.".into());
    }
    match ty {
        ValueType::Probability(_) if invalid => {
            if frame.values.iter().any(|&v| v != 0.0) {
                return Err("무효 확률 payload는 0으로 기록한다.".into());
            }
        }
        ValueType::Probability(_) => probability(&frame.values)?,
        ValueType::Decision(_)
            if frame.values.len() != 2
                || (invalid && frame.values != [-1.0, 0.0])
                || (!invalid
                    && ((frame.values[0] != -1.0
                        && (frame.values[0] < 0.0
                            || frame.values[0] >= classes.len() as f64
                            || frame.values[0].fract() != 0.0))
                        || !(0.0..=1.0).contains(&frame.values[1]))) =>
        {
            return Err("판정은 class index 또는 Hold(-1)와 0..1 confidence를 요구한다.".into());
        }
        _ => {}
    }
    Ok(())
}

/// 고정 Eval 행과 예측의 연결.
#[derive(Debug, Clone)]
pub struct Prediction {
    /// catalog 행 ID.
    pub id: String,
    /// 실제 클래스 index.
    pub label: usize,
    /// 첫 최대 확률의 클래스 index.
    pub predicted: usize,
    /// 클래스 순서의 확률.
    pub probabilities: Vec<f64>,
}
/// RequireAll 품질 정책으로 계산한 독립 평가.
#[derive(Debug, Clone)]
pub struct Evaluation {
    /// 명시적 집계·재표집 protocol의 추가 보고서.
    pub analysis: Option<Box<crate::evaluation::Analysis>>,
    /// 평가한 고정 모델 digest.
    pub model_digest: String,
    /// 평가 dataset ID.
    pub dataset: String,
    /// 클래스 순서.
    pub classes: Vec<String>,
    /// label-major K×K confusion counts.
    pub confusion: Vec<u64>,
    /// 클래스별 recall 평균.
    pub balanced_accuracy: f64,
    /// 전체 정답 비율.
    pub accuracy: f64,
    /// valid Eval 행의 예측과 라벨.
    pub predictions: Vec<Prediction>,
    /// RequireAll에서 제외한 Eval 행 ID.
    pub excluded: Vec<String>,
    /// 평가 원천의 감사 가능한 범위·그룹.
    pub lineage: Vec<Assignment>,
}
