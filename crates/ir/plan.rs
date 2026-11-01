//! 원천 시각 병합·간선 큐·다중 출력의 의존 순차 실행 계획.

use crate::graph::Obligation;
use bci_schema::{
    kernel::KernelSpec,
    types::{Span, StreamType},
};

/// 실행 계획의 단일 노드와 예약량.
#[derive(Debug, Clone)]
pub struct PlannedNode {
    /// TypedGraph와 동일한 ID.
    pub id: usize,
    /// 원본 연산자 이름.
    pub operator: String,
    /// 원본 호출 범위.
    pub span: Span,
    /// 입력 포트의 노드 ID.
    pub inputs: Vec<usize>,
    /// 출력의 측정·시간 계약.
    pub output: Option<StreamType>,
    /// 실행할 구체 커널과 매개변수.
    pub kernel: KernelSpec,
    /// 해당 노드 출력 슬롯의 예약 bytes.
    pub buffer_bytes: usize,
    /// 해당 커널 상태·scratch 예약 bytes.
    pub state_bytes: usize,
}

/// 입력 묶음마다 의존 순서로 실행하는 유한 계획.
#[derive(Debug, Clone)]
pub struct ExecutionPlan {
    /// 실행 진입점 이름.
    pub entry: String,
    /// ID 순서의 커널 인스턴스.
    pub nodes: Vec<PlannedNode>,
    /// 선행 노드가 먼저 발화하는 순서.
    pub schedule: Vec<usize>,
    /// ID 순서의 합성·CSV·재생 원천 목록.
    pub sources: Vec<usize>,
    /// 읽을 원천 묶음 상한. None은 모든 파일의 EOF까지 처리한다.
    pub samples: Option<u64>,
    /// 실행 작업 영역의 예산.
    pub memory_budget: usize,
    /// 모든 노드의 출력·상태 예약량 합.
    pub reserved_bytes: usize,
    /// 입력 간선의 고정 큐 용량과 예약량.
    pub queues: crate::queues::Queues,
    /// prepare·수치 guard·첫 창으로 해소할 의무.
    pub obligations: Vec<Obligation>,
}

impl ExecutionPlan {
    /// 원천 격자가 같으면 완전 동기 묶음을, 다르면 시각 순서 병합을 사용한다.
    pub fn synchronous_inputs(&self) -> bool {
        let first = self
            .sources
            .first()
            .and_then(|&id| self.nodes.get(id))
            .and_then(|n| n.output.as_ref());
        self.sources.iter().all(|&id| {
            self.nodes
                .get(id)
                .and_then(|n| n.output.as_ref())
                .map(|t| &t.occurrence)
                == first.map(|t| &t.occurrence)
        })
    }
}
