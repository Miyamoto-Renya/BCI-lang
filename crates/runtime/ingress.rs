//! 동기 격자 또는 시각 순서로 원천 묶음을 준비하는 입력 경계.
use bci_ir::plan::ExecutionPlan;
use bci_schema::{
    execution::{Frame, RunError},
    input::InputSource,
};

pub(crate) struct Reader {
    pub id: usize,
    pub input: Box<dyn InputSource>,
    pub pending: Frame,
    pub ready: bool,
    pub ended: bool,
}

/// 격자가 같으면 완전 동기 묶음을, 다르면 가장 이른 시각의 원천들을 수락한다.
pub(crate) fn read(
    plan: &ExecutionPlan,
    readers: &mut [Reader],
    frames: &mut [Frame],
    present: &mut [bool],
) -> Result<bool, RunError> {
    let fault = |id: usize, message: String| RunError {
        code: "E0904",
        node: Some(id),
        span: plan.nodes[id].span,
        message,
    };
    present.fill(false);
    if !plan.synchronous_inputs() {
        let mut earliest = None;
        for reader in readers.iter_mut() {
            if !reader.ready && !reader.ended {
                reader.ready = reader
                    .input
                    .read(&mut reader.pending)
                    .map_err(|e| RunError {
                        code: e.code,
                        node: Some(reader.id),
                        span: plan.nodes[reader.id].span,
                        message: e.message,
                    })?;
                reader.ended = !reader.ready;
            }
            if reader.ready {
                if reader.pending.values.iter().any(|v| !v.is_finite())
                    || reader.pending.valid.iter().any(|&v| v > 1)
                {
                    return Err(fault(
                        reader.id,
                        "원천의 유한 수치·valid 범위를 확인한다.".into(),
                    ));
                }
                if earliest.is_none()
                    || reader
                        .pending
                        .time
                        .compare(earliest.expect("time"))
                        .map_err(|e| fault(reader.id, e))?
                        .is_lt()
                {
                    earliest = Some(reader.pending.time);
                }
            }
        }
        for reader in readers {
            if reader.ready && Some(reader.pending.time) == earliest {
                frames[reader.id].copy_from(&reader.pending);
                present[reader.id] = true;
                reader.ready = false;
            }
        }
        return Ok(earliest.is_some());
    }
    let mut active = None;
    for reader in readers {
        let id = reader.id;
        let read = reader.input.read(&mut frames[id]).map_err(|e| RunError {
            code: e.code,
            node: Some(id),
            span: plan.nodes[id].span,
            message: e.message,
        })?;
        if active.is_some_and(|v| v != read) {
            return Err(fault(
                id,
                format!(
                    "원천 %{id}와 %{}의 종료 위치가 다르다. 동일 길이의 입력 구간 또는 공통 --samples 상한을 지정한다.",
                    plan.sources[0]
                ),
            ));
        }
        active = Some(read);
        if read {
            present[id] = true;
            let frame = &frames[id];
            if frame.values.iter().any(|v| !v.is_finite()) || frame.valid.iter().any(|&v| v > 1) {
                return Err(fault(
                    id,
                    "원천 묶음의 유한 수치·valid 범위를 확인한다.".into(),
                ));
            }
            let first = &frames[plan.sources[0]];
            if !first.same_tick(frame) {
                return Err(fault(
                    id,
                    format!(
                        "원천 %{id}의 시각 {}s·index {}·generation {}·Gap {:?}가 원천 %{}의 {}s·index {}·generation {}·Gap {:?}와 다르다. 동기 입력 구간을 맞춘다.",
                        frame.time,
                        frame.sequence,
                        frame.generation,
                        frame.gap,
                        plan.sources[0],
                        first.time,
                        first.sequence,
                        first.generation,
                        first.gap
                    ),
                ));
            }
        }
    }
    Ok(active == Some(true))
}
