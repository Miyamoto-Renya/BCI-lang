//! 소스 위치와 타입·효과·후속 의무를 가진 처리 그래프.

use bci_schema::types::{Effect, Span, StreamType};
use std::collections::BTreeMap;

/// 그래프 내 노드 식별자.
pub type NodeId = usize;

/// 타입이 확정된 연산자 인스턴스.
#[derive(Debug, Clone)]
pub struct TypedNode {
    /// 그래프 내 순번.
    pub id: NodeId,
    /// 연산자 정규 이름.
    pub operator: String,
    /// 원본 호출 위치.
    pub span: Span,
    /// 실제 입력 노드 ID, 인수 순서 유지.
    pub inputs: Vec<NodeId>,
    /// stream을 생성하는 노드의 타입.
    pub output: Option<StreamType>,
    /// 정규화한 구성 인수.
    pub parameters: BTreeMap<String, crate::value::ArgumentValue>,
    /// 입력 경로를 포함한 합성 효과.
    pub effect: Effect,
}

/// bind 또는 실행 경계에서 확인할 조건.
#[derive(Debug, Clone)]
pub struct Obligation {
    /// 의무의 순번.
    pub id: usize,
    /// 발생 노드.
    pub node: NodeId,
    /// 조건의 종류.
    pub kind: String,
    /// 필요한 증거와 검사 조건.
    pub predicate: String,
    /// 조건을 요구한 위치.
    pub span: Span,
}

/// 한 run 진입점의 형식화 그래프.
#[derive(Debug, Clone)]
pub struct TypedGraph {
    /// 모듈과 진입점 이름.
    pub entry: String,
    /// 진입점이 선언한 실행 모드·자원 구성.
    pub configuration: BTreeMap<String, crate::value::ArgumentValue>,
    /// 의존 순서로 생성한 노드.
    pub nodes: Vec<TypedNode>,
    /// 이름 해석 결과와 포트의 대응.
    pub bindings: BTreeMap<String, NodeId>,
    /// 추가 확인이 필요한 계약.
    pub obligations: Vec<Obligation>,
    /// 진입점 전체 효과.
    pub effect: Effect,
}
