//! TypedGraph의 정확한 상수와 기술자 인수.

use crate::graph::NodeId;
use bci_schema::{
    number::Rational,
    types::{ChannelSchema, Occurrence},
    unit::Unit,
};
use std::collections::BTreeMap;

/// 구체화 그래프가 보존하는 타입 있는 인수 값.
#[derive(Debug, Clone)]
pub enum ArgumentValue {
    /// 검증한 고정 모델 artifact.
    Model(Box<bci_schema::learning::Model>),
    /// 파일 경로·타입·읽기 정책이 정해진 원천 기술자.
    Source(Box<bci_schema::input::SourceSpec>),
    /// 사용자 커널의 컴파일 시점 계약과 정체성.
    Extension(Box<bci_schema::extension::OperatorSpec>),
    /// 정확한 상수와 단위.
    Number(Rational, Unit),
    /// 문자열.
    Text(String),
    /// 논리 상수.
    Bool(bool),
    /// 검사한 명목 enum 값.
    Enum(String),
    /// 고정 배열.
    Array(Vec<ArgumentValue>),
    /// 순서를 가진 값 또는 포트 묶음.
    Tuple(Vec<ArgumentValue>),
    /// 이름 있는 구성 필드.
    Record(BTreeMap<String, ArgumentValue>),
    /// 순서 있는 채널 기술자.
    Schema(ChannelSchema),
    /// 명목 clock 식별자.
    Clock(String),
    /// 발생 규칙.
    Rate(Occurrence),
    /// 장치 바인딩 기술자.
    Device {
        /// 소스 선언의 명목 ID.
        identity: String,
        /// 프로필 경로.
        profile: String,
        /// 장치 설정.
        config: BTreeMap<String, ArgumentValue>,
    },
    /// 명목 시계 사상과 후속 증거 참조.
    Mapping {
        /// 매핑 기술자 경로.
        path: String,
        /// 원천 clock ID.
        from: String,
        /// 대상 clock ID.
        to: String,
        /// 초 단위 명목 기울기.
        slope: Rational,
        /// 대상 초 단위 offset.
        offset: Rational,
    },
    /// 안정성을 검사한 고정 SOS 실현.
    Iir(Box<bci_schema::iir::Realization>),
    /// 정확한 필터 계수.
    Coefficients(Vec<Rational>),
    /// 실행 자원 요구.
    Resource(BTreeMap<String, ArgumentValue>),
    /// 그래프의 출력 포트 참조.
    Stream(NodeId),
    /// sink·구성 호출의 종료 값.
    Void,
}

impl ArgumentValue {
    /// 설명 도구에 표시할 짧은 표현을 만든다.
    pub fn describe(&self) -> String {
        match self {
            Self::Model(model) => format!("Model({})", model.digest()),
            Self::Source(source) => format!("Source({:?})", source.kind),
            Self::Extension(spec) => format!(
                "Extension({}@{}, {})",
                spec.name, spec.version, spec.function_digest
            ),
            Self::Number(n, u) => format!("{n}[{u}]"),
            Self::Text(t) => format!("{t:?}"),
            Self::Bool(b) => b.to_string(),
            Self::Enum(t) => t.clone(),
            Self::Array(values) => format!(
                "[{}]",
                values
                    .iter()
                    .map(Self::describe)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Tuple(values) => format!(
                "({})",
                values
                    .iter()
                    .map(Self::describe)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Record(values) | Self::Resource(values) => format!(
                "#{{{}}}",
                values
                    .iter()
                    .map(|(k, v)| format!("{k}: {}", v.describe()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Schema(schema) => format!("Schema({})", schema.name),
            Self::Clock(clock) => format!("Clock({clock})"),
            Self::Rate(rate) => rate.to_string(),
            Self::Device {
                identity,
                profile,
                config,
            } => format!("Device({identity}, {profile}, {} fields)", config.len()),
            Self::Mapping {
                path,
                from,
                to,
                slope,
                offset,
            } => format!("Mapping({from} -> {to}, slope={slope}, offset={offset}, {path})"),
            Self::Iir(realization) => format!(
                "SOS({} sections, {}Hz, {})",
                realization.sections.len(),
                realization.rate,
                realization.digest()
            ),
            Self::Coefficients(values) => format!("Coefficients({})", values.len()),
            Self::Stream(node) => format!("%{node}"),
            Self::Void => "Unit".into(),
        }
    }
    /// 포함된 포트 참조를 인수 순서로 반환한다.
    pub fn streams(&self) -> Vec<NodeId> {
        match self {
            Self::Stream(id) => vec![*id],
            Self::Tuple(values) | Self::Array(values) => {
                values.iter().flat_map(Self::streams).collect()
            }
            Self::Record(values) | Self::Resource(values) => {
                values.values().flat_map(Self::streams).collect()
            }
            _ => Vec::new(),
        }
    }
}
