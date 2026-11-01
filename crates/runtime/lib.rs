//! Actor, 일정, buffer pool, 실행 상태 전이와 supervisor의 소유 패키지다.
//!
//! 검증 경계는 bci-verify, 연산자 호출 경계는 bci-extension,
//! 장치 호출 경계는 bci-devices가 제공한다. 구체 구현체는 상위 계층이 주입한다.

mod checkpoint;
mod ingress;
mod scheduler;
mod sequential;
pub use bci_ir::checkpoint::Checkpoint;
pub use bci_ir::plan::ExecutionPlan;
pub use bci_schema::execution::{Frame, RunError, RunReport, SessionState};
pub use bci_schema::input::{InputError, InputSource, SourceFactory, SourceKind, SourceSpec};
pub use checkpoint::CheckpointOptions;
pub use sequential::{Recorder, run, run_with_checkpoint};
