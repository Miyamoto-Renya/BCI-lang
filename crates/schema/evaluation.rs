//! 평가 집계 순서·cluster 재표집·분모와 보고서 계약.
use crate::learning::{Assignment, field};
use sha2::{Digest, Sha256};

/// catalog 계층의 집계 또는 재표집 단위.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Unit {
    /// 피험자 전체.
    Subject,
    /// 피험자 안의 세션 전체.
    Session,
    /// 피험자·세션 안의 시행 전체.
    Trial,
}
impl Unit {
    /// 그룹 key의 접두사 길이.
    pub fn depth(self) -> usize {
        match self {
            Self::Subject => 1,
            Self::Session => 2,
            Self::Trial => 3,
        }
    }
}

/// 복원 추출 cluster bootstrap의 고정 매개변수.
#[derive(Debug, Clone, PartialEq)]
pub struct Bootstrap {
    /// 함께 복원 추출하는 독립 catalog 단위.
    pub unit: Unit,
    /// 시도할 복원 추출 횟수, 100..10000.
    pub iterations: usize,
    /// 양측 percentile 구간의 확률 질량, (0,1).
    pub confidence: f64,
    /// SplitMix64/0.1의 초기 상태.
    pub seed: u64,
}

/// Sample에서 시작해 명시한 계층 순서로 동일 가중치를 적용한다.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Protocol {
    /// 예: Trial → Session → Subject. 빈 배열은 Sample 집계다.
    pub aggregation: Vec<Unit>,
    /// 선택적 cluster bootstrap 설정.
    pub bootstrap: Option<Bootstrap>,
}
impl Protocol {
    /// 계층 순서·재표집 단위·작업 범위를 검사한다.
    pub fn validate(&self, group_by: &str) -> Result<(), String> {
        if self.aggregation.len() > 3
            || self
                .aggregation
                .windows(2)
                .any(|p| p[0].depth() <= p[1].depth())
        {
            return Err(
                "E0703: 집계 순서는 Trial → Session → Subject의 부분 수열을 요구한다.".into(),
            );
        }
        let independent = match group_by {
            "subject" => 1,
            "session" => 2,
            "trial" => 3,
            _ => return Err("E0703: 평가 그룹 정책은 subject·session·trial을 요구한다.".into()),
        };
        if let Some(b) = &self.bootstrap
            && (!(100..=10000).contains(&b.iterations)
                || !b.confidence.is_finite()
                || b.confidence <= 0.0
                || b.confidence >= 1.0
                || b.unit.depth() > independent
                || self
                    .aggregation
                    .last()
                    .is_some_and(|u| b.unit.depth() > u.depth()))
        {
            return Err("E0703: bootstrap은 100..10000회·confidence (0,1)·분할 및 최종 집계와 같거나 상위인 재표집 단위를 요구한다.".into());
        }
        Ok(())
    }
    /// 집계·난수·분모·percentile 계약을 식별한다.
    pub fn digest(&self) -> String {
        let mut hash = Sha256::new();
        field(&mut hash, b"bci-evaluation-protocol/0.1:EqualChildMass:RequireAll:SplitMix64:ExcludeUndefined:LinearPercentile");
        field(&mut hash, format!("{:?}", self.aggregation).as_bytes());
        if let Some(b) = &self.bootstrap {
            field(&mut hash, format!("{:?}", b.unit).as_bytes());
            hash.update((b.iterations as u64).to_le_bytes());
            hash.update(b.confidence.to_bits().to_le_bytes());
            hash.update(b.seed.to_le_bytes());
        }
        format!("{:x}", hash.finalize())
    }
}

/// 동일 가중 집계 후 클래스별 질량·정답률.
#[derive(Debug, Clone)]
pub struct Metrics {
    /// label-major K×K, 전체 질량은 1 또는 빈 그룹의 0이다.
    pub confusion: Vec<f64>,
    /// 클래스별 label 질량.
    pub denominators: Vec<f64>,
    /// 전체 valid 질량 중 정답 비율. 빈 그룹은 None이다.
    pub accuracy: Option<f64>,
    /// 모든 클래스의 분모가 양수일 때의 recall 평균.
    pub balanced_accuracy: Option<f64>,
}

/// 한 집계 단계의 그룹과 그 그룹에 기여한 분모.
#[derive(Debug, Clone)]
pub struct Group {
    /// 이 그룹의 계층 단위.
    pub unit: Unit,
    /// subject부터 시작하는 전체 경로.
    pub key: Vec<String>,
    /// valid 원래 표본 수.
    pub included: usize,
    /// RequireAll 품질 제외 표본 수.
    pub excluded: usize,
    /// 동일 가중치로 합친 valid 자식 수.
    pub children: usize,
    /// 해당 단계의 가중 confusion과 지표.
    pub metrics: Metrics,
}

/// 재표집 cluster별 원래 표본·최종 집계 단위 수.
#[derive(Debug, Clone)]
pub struct Cluster {
    /// 복원 추출 목록에서 사용할 정렬한 catalog 경로.
    pub key: Vec<String>,
    /// 이 cluster에 기여한 최종 집계 단위 수.
    pub mass: usize,
}

/// percentile 구간과 지표별 정의 가능한 반복 수.
#[derive(Debug, Clone)]
pub struct Interval {
    /// 아래쪽 percentile.
    pub lower: f64,
    /// 위쪽 percentile.
    pub upper: f64,
    /// 지표를 정의할 수 있는 bootstrap 반복 수.
    pub defined: usize,
}

/// seed·cluster 순서로 재계산 가능한 bootstrap 결과.
#[derive(Debug, Clone)]
pub struct Resampling {
    /// 고유 key의 사전식 순서. mass > 0인 cluster만 복원 추출한다.
    pub clusters: Vec<Cluster>,
    /// 반복 순서의 accuracy.
    pub accuracy: Vec<f64>,
    /// 반복 순서의 balanced accuracy. 결측 클래스 반복은 None이다.
    pub balanced_accuracy: Vec<Option<f64>>,
    /// accuracy의 percentile 구간.
    pub accuracy_interval: Interval,
    /// 정의 가능한 balanced accuracy의 percentile 구간.
    pub balanced_accuracy_interval: Interval,
}

/// 품질 제외 행까지 보존하는 평가 catalog 연결.
#[derive(Debug, Clone)]
pub struct CatalogRow {
    /// 분할의 원래 행·그룹·원천 구간.
    pub assignment: Assignment,
    /// subject/session/trial key 접두사.
    pub hierarchy: Vec<String>,
    /// RequireAll 품질 포함 여부.
    pub included: bool,
}

/// 고정 모델 평가에 덧붙이는 집계·불확실성 분석.
#[derive(Debug, Clone)]
pub struct Analysis {
    /// 사전에 지정한 집계·재표집 계약.
    pub protocol: Protocol,
    /// valid Eval 값·라벨·원천의 digest.
    pub eval_digest: String,
    /// 품질 제외 행을 포함한 전체 Eval catalog.
    pub catalog: Vec<CatalogRow>,
    /// 요청한 단계 순서와 그룹 key 순서의 지표.
    pub groups: Vec<Group>,
    /// 마지막 집계 단계의 동일 가중 결과.
    pub estimate: Metrics,
    /// 요청한 bootstrap 결과.
    pub resampling: Option<Resampling>,
}
