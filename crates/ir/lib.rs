//! TypedGraph, BoundGraph, ExecutionPlan과 증명서의 데이터 구조를 소유한다.
//!
//! compiler가 계획을 생성하고 verify가 같은 데이터 구조를 독립 검사한다.
//! 타입·스키마 참조는 bci-schema의 계약을 사용한다.

pub mod checkpoint;
pub mod graph;
pub mod plan;
pub mod queues;

pub mod value;
