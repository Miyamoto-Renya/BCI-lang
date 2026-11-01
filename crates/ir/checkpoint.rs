//! 완료한 입력 묶음 경계의 순차 실행 상태.

/// 전체 codec payload의 저장 상한.
pub const MAX_STATE_BYTES: usize = 16 * 1024 * 1024;

/// 계획·원천 prefix·각 커널과 격자를 묶은 복원 단위.
#[derive(Debug, Clone)]
pub struct Checkpoint {
    /// 표본 상한을 제외한 계획의 SHA-256.
    pub plan_digest: [u8; 32],
    /// 입력 병합 정책으로 처리한 묶음 수.
    pub position: u64,
    /// 처리한 입력 prefix의 의미 값 SHA-256 체인.
    pub prefix_digest: [u8; 32],
    /// 노드 ID 순서의 불투명 kernel payload.
    pub kernels: Vec<Vec<u8>>,
    /// 합류를 기다리는 간선 큐의 상태.
    pub scheduler: Vec<u8>,
    /// 노드 ID 순서의 시간 격자 상태.
    pub grids: Vec<bci_schema::input::GridCursor>,
}
