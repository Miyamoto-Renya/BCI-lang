use bci_ir::{
    checkpoint::{Checkpoint, MAX_STATE_BYTES},
    plan::ExecutionPlan,
};
use bci_schema::execution::Frame;
use sha2::{Digest, Sha256};

/// 복원할 경계와 정상 종료 시 상태 캡처 여부.
#[derive(Default)]
pub struct CheckpointOptions<'a> {
    /// 이전에 완료한 호출 경계.
    pub resume: Option<&'a Checkpoint>,
    /// 종료 상태를 반환할지 여부.
    pub save: bool,
}

pub(crate) struct Recovery<'a> {
    pub options: CheckpointOptions<'a>,
    pub recording: bool,
    pub result: Option<Checkpoint>,
}

pub(crate) fn plan_digest(plan: &ExecutionPlan) -> [u8; 32] {
    let mut normalized = plan.clone();
    normalized.samples = None;
    Sha256::digest(format!("bci-checkpoint-plan/0.1:{normalized:?}").as_bytes()).into()
}

pub(crate) fn validate(plan: &ExecutionPlan, saved: &Checkpoint) -> Result<(), String> {
    if saved.plan_digest != plan_digest(plan)
        || saved.kernels.len() != plan.nodes.len()
        || saved.grids.len() != plan.nodes.len()
        || saved.position > 1_000_000_000
    {
        return Err("checkpoint의 계획·커널 수·원천 위치를 현재 실행과 맞춘다.".into());
    }
    if saved.scheduler.len() > plan.queues.bytes {
        return Err("checkpoint 큐 payload가 예약량을 초과했다.".into());
    }
    let mut bytes = saved.scheduler.len();
    for (node, payload) in plan.nodes.iter().zip(&saved.kernels) {
        if payload.len() > node.kernel.checkpoint_bytes()? {
            return Err("checkpoint payload가 커널 상한을 초과했다.".into());
        }
        bytes = bytes
            .checked_add(payload.len())
            .ok_or("checkpoint 크기 범위를 초과했다.")?;
        if let Some(output) = &node.output {
            saved.grids[node.id].validate(&output.occurrence)?;
        } else if saved.grids[node.id] != Default::default() {
            return Err("sink의 격자 상태는 빈 값으로 저장한다.".into());
        }
    }
    if bytes > MAX_STATE_BYTES {
        return Err("checkpoint 전체 payload 상한은 16 MiB다.".into());
    }
    Ok(())
}

pub(crate) fn prefix(previous: [u8; 32], frame: &Frame) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(previous);
    for n in [
        frame.sequence,
        frame.generation,
        frame.source_first,
        frame.source_last,
        frame.values.len() as u64,
        frame.valid.len() as u64,
    ] {
        hash.update(n.to_le_bytes());
    }
    hash.update([
        frame.flags,
        frame.origin as u8,
        u8::from(frame.gap.is_some()),
    ]);
    if let Some(gap) = frame.gap {
        hash.update(gap.first.to_le_bytes());
        hash.update(gap.last.to_le_bytes());
    }
    for v in &frame.values {
        hash.update(v.to_bits().to_le_bytes());
    }
    hash.update(&frame.valid);
    hash.update(frame.time.to_string().as_bytes());
    hash.finalize().into()
}
