use crate::checkpoint::{self, CheckpointOptions, Recovery};
use crate::ingress::{self, Reader};
use bci_extension::{Kernel, KernelFactory, StateReader, StateWriter};
use bci_ir::checkpoint::{Checkpoint, MAX_STATE_BYTES};
use bci_ir::plan::ExecutionPlan;
use bci_schema::{
    execution::{Frame, NodeStats, RunError, RunReport, SessionState},
    input::{GridCursor, SourceFactory},
    kernel::KernelSpec,
    types::Span,
};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};

/// 실제 저장 codec을 상위 조립 계층에서 연결하는 경계.
pub trait Recorder {
    /// 출력 파일·메타데이터와 고정 staging을 준비한다.
    fn prepare(&mut self, plan: &ExecutionPlan) -> Result<(), String>;
    /// 한 sink의 한 입력 포트에서 게시한 레코드를 동기 기록한다.
    fn write(&mut self, sink: usize, source: usize, frame: &Frame) -> Result<(), String>;
    /// 파일을 flush·sync·close하고 성공 또는 실패 보고서를 commit한다.
    fn finish(&mut self, report: &RunReport) -> Result<(), String>;
}

/// 계획을 검사하고 유한 표본 또는 외부 stop까지 실행한다.
pub fn run(
    plan: &ExecutionPlan,
    factory: &dyn KernelFactory,
    sources: &dyn SourceFactory,
    recorder: &mut dyn Recorder,
    stop: &AtomicBool,
) -> RunReport {
    run_with_checkpoint(
        plan,
        factory,
        sources,
        recorder,
        stop,
        CheckpointOptions::default(),
    )
    .0
}

/// 완료한 입력 경계의 전체 커널·격자를 저장하거나 복원해 실행한다.
pub fn run_with_checkpoint(
    plan: &ExecutionPlan,
    factory: &dyn KernelFactory,
    sources: &dyn SourceFactory,
    recorder: &mut dyn Recorder,
    stop: &AtomicBool,
    options: CheckpointOptions<'_>,
) -> (RunReport, Option<Checkpoint>) {
    let mut report = RunReport {
        pending_records: 0,
        input_offset: options.resume.map_or(0, |s| s.position),
        entry: plan.entry.clone(),
        requested_samples: plan.samples,
        end_of_input: false,
        gaps: 0,
        missing_samples: 0,
        completed_samples: 0,
        reserved_bytes: plan.reserved_bytes,
        elapsed_seconds: 0.0,
        stopped: false,
        states: vec![SessionState::Created, SessionState::Preparing],
        nodes: vec![NodeStats::default(); plan.nodes.len()],
        failure: None,
    };
    let mut recovery = Recovery {
        options,
        recording: false,
        result: None,
    };
    let result = execute(
        plan,
        factory,
        sources,
        recorder,
        stop,
        &mut report,
        &mut recovery,
    );
    if let Err(error) = result {
        report.failure = Some(error);
        report.states.push(SessionState::Faulted);
    }
    report.states.push(SessionState::Draining);
    report.states.push(SessionState::Closed);
    if recovery.recording
        && let Err(message) = recorder.finish(&report)
    {
        report.states.pop();
        report.states.extend([
            SessionState::Faulted,
            SessionState::Draining,
            SessionState::Closed,
        ]);
        if let Some(error) = &mut report.failure {
            error.message.push_str(&format!("; 종료 기록: {message}"));
        } else {
            report.failure = Some(RunError {
                code: "E0902",
                node: None,
                span: Span::default(),
                message,
            });
        }
        let _ = recorder.finish(&report);
    }
    if report.failure.is_some() {
        recovery.result = None;
    }
    (report, recovery.result)
}

fn execute(
    plan: &ExecutionPlan,
    factory: &dyn KernelFactory,
    sources: &dyn SourceFactory,
    recorder: &mut dyn Recorder,
    stop: &AtomicBool,
    report: &mut RunReport,
    recovery: &mut Recovery<'_>,
) -> Result<(), RunError> {
    let checked = bci_verify::verify(plan)?;
    let plan = checked.plan();
    let lead = plan.sources[0];
    let fault = |id: usize, code: &'static str, message: String| RunError {
        code,
        node: Some(id),
        span: plan.nodes[id].span,
        message,
    };
    for node in &plan.nodes {
        if let KernelSpec::Extension { operator, .. } = &node.kernel
            && factory.extension_contract(&operator.name) != Some(operator.as_ref())
        {
            return Err(fault(
                node.id,
                "E0510",
                format!(
                    "{}: 계획의 계약·함수·reference를 실행 번들과 맞춘다. {}",
                    operator.name, operator.rust_source
                ),
            ));
        }
    }
    if let Some(saved) = recovery.options.resume {
        checkpoint::validate(plan, saved).map_err(|e| fault(lead, "E0909", e))?;
    }
    if recovery.options.save {
        let mut total = plan.queues.bytes;
        for node in &plan.nodes {
            total = total
                .checked_add(
                    node.kernel
                        .checkpoint_bytes()
                        .map_err(|e| fault(node.id, "E0909", e))?,
                )
                .filter(|&n| n <= MAX_STATE_BYTES)
                .ok_or_else(|| {
                    fault(
                        node.id,
                        "E0909",
                        "checkpoint 전체 예약 상한은 16 MiB다.".into(),
                    )
                })?;
        }
    }
    let mut kernels: Vec<Option<Box<dyn Kernel>>> = Vec::new();
    let mut frames = Vec::new();
    let mut readers = Vec::with_capacity(plan.sources.len());
    for node in &plan.nodes {
        let (values, channels) = node
            .kernel
            .layout()
            .map_err(|e| fault(node.id, "E0801", e))?;
        frames.push(Frame::allocate(values, channels).map_err(|e| {
            fault(
                node.id,
                "E0801",
                format!("출력 슬롯 예약 실패: {e}. memory 예산과 가용 메모리를 확인한다."),
            )
        })?);
        kernels.push(if let KernelSpec::Source(spec) = &node.kernel {
            readers.push(Reader {
                id: node.id,
                pending: Frame::allocate(values, channels)
                    .map_err(|e| fault(node.id, "E0801", e))?,
                ready: false,
                ended: false,
                input: sources
                    .open(spec)
                    .map_err(|e| fault(node.id, e.code, e.message))?,
            });
            None
        } else if matches!(node.kernel, KernelSpec::Record { .. }) {
            None
        } else {
            Some(
                catch_unwind(AssertUnwindSafe(|| factory.create(&node.kernel)))
                    .map_err(|_| {
                        fault(
                            node.id,
                            "E0908",
                            format!("{} 커널 준비에서 panic이 발생했다.", node.operator),
                        )
                    })?
                    .map_err(|e| {
                        fault(
                            node.id,
                            if matches!(node.kernel, KernelSpec::Extension { .. }) {
                                "E0510"
                            } else {
                                "E0803"
                            },
                            e,
                        )
                    })?,
            )
        });
    }
    let mut scheduler =
        crate::scheduler::Scheduler::new(plan).map_err(|e| fault(lead, "E0801", e))?;
    let mut grids: Vec<_> = (0..plan.nodes.len())
        .map(|_| GridCursor::default())
        .collect();
    let mut prefix = [0u8; 32];
    let mut present = vec![false; plan.nodes.len()];
    if let Some(saved) = recovery.options.resume {
        for _ in 0..saved.position {
            if !ingress::read(plan, &mut readers, &mut frames, &mut present)? {
                return Err(fault(
                    lead,
                    "E0909",
                    "입력이 checkpoint 위치 전에 끝났다.".into(),
                ));
            }
            for &id in &plan.sources {
                if !present[id] {
                    continue;
                }
                grids[id]
                    .check(
                        &plan.nodes[id]
                            .output
                            .as_ref()
                            .expect("source type")
                            .occurrence,
                        &frames[id],
                    )
                    .map_err(|e| fault(id, "E0909", e))?;
                prefix = checkpoint::prefix(prefix, &frames[id]);
            }
        }
        if prefix != saved.prefix_digest
            || plan.sources.iter().any(|&id| grids[id] != saved.grids[id])
        {
            return Err(fault(
                lead,
                "E0909",
                "원천 묶음 prefix의 값·시각·품질이 checkpoint와 다르다. 원래 입력들을 복원한다."
                    .into(),
            ));
        }
        for (id, kernel) in kernels.iter_mut().enumerate() {
            if let Some(kernel) = kernel {
                let mut reader = StateReader::new(&saved.kernels[id]);
                catch_unwind(AssertUnwindSafe(|| kernel.restore(&mut reader)))
                    .map_err(|_| fault(id, "E0908", "상태 복원에서 panic이 발생했다.".into()))?
                    .and_then(|_| reader.finish())
                    .map_err(|e| fault(id, "E0909", e))?;
            }
        }
        grids = saved.grids.clone();
        scheduler
            .restore(plan, &grids, &mut StateReader::new(&saved.scheduler))
            .map_err(|e| fault(lead, "E0909", e))?;
    }
    recovery.recording = true;
    recorder.prepare(plan).map_err(|e| RunError {
        code: "E0902",
        node: None,
        span: Span::default(),
        message: e,
    })?;
    report
        .states
        .extend([SessionState::Ready, SessionState::Running]);
    let started = Instant::now();
    let result = (|| {
        for local_index in 0..plan.samples.unwrap_or(u64::MAX) {
            let index = report
                .input_offset
                .checked_add(local_index)
                .filter(|&n| n < 1_000_000_000)
                .ok_or_else(|| {
                    fault(
                        lead,
                        "E0909",
                        "누적 원천 레코드 상한은 1000000000개다.".into(),
                    )
                })?;
            if stop.load(Ordering::Relaxed) {
                report.stopped = true;
                break;
            }
            if !ingress::read(plan, &mut readers, &mut frames, &mut present)? {
                if !scheduler.is_empty() {
                    return Err(fault(lead,"E0901","EOF에서 합류 대기 레코드가 남았다. 입력 구간과 재표본화 경로의 출력 범위를 맞춘다.".into()));
                }
                report.end_of_input = true;
                break;
            }
            for &id in &plan.sources {
                if !present[id] {
                    continue;
                }
                let anchor = &frames[id];
                if let Some(gap) = anchor.gap {
                    let missing = gap
                        .last
                        .checked_sub(gap.first)
                        .and_then(|n| n.checked_add(1))
                        .ok_or_else(|| {
                            fault(id, "E0904", "Gap 범위의 순서와 크기를 확인한다.".into())
                        })?;
                    scheduler.reset_from(plan, id);
                    if !plan.synchronous_inputs() || id == lead {
                        report.gaps += 1;
                        report.missing_samples =
                            report.missing_samples.checked_add(missing).ok_or_else(|| {
                                fault(id, "E0904", "누락 표본 합의 범위를 초과했다.".into())
                            })?;
                    }
                } else if grids[id].generation.is_some_and(|g| g != anchor.generation) {
                    scheduler.reset_from(plan, id);
                    if !plan.synchronous_inputs() || id == lead {
                        report.gaps += 1;
                    }
                }
            }
            for &id in &plan.sources {
                if !present[id] {
                    continue;
                }
                grids[id]
                    .check(
                        &plan.nodes[id]
                            .output
                            .as_ref()
                            .expect("source type")
                            .occurrence,
                        &frames[id],
                    )
                    .map_err(|e| fault(id, "E0904", e))?;
                if recovery.options.save || recovery.options.resume.is_some() {
                    prefix = checkpoint::prefix(prefix, &frames[id]);
                }
            }
            scheduler.step(
                plan,
                &present,
                &mut kernels,
                &mut frames,
                &mut grids,
                recorder,
                report,
                index,
            )?;
            report.completed_samples += 1;
        }
        Ok(())
    })();
    report.elapsed_seconds = started.elapsed().as_secs_f64();
    report.pending_records = scheduler.pending();
    result?;
    if recovery.options.save {
        let mut payloads = Vec::new();
        let mut total = plan.queues.bytes;
        for (id, kernel) in kernels.iter().enumerate() {
            let limit = plan.nodes[id]
                .kernel
                .checkpoint_bytes()
                .map_err(|e| fault(id, "E0909", e))?;
            total = total
                .checked_add(limit)
                .filter(|&n| n <= MAX_STATE_BYTES)
                .ok_or_else(|| {
                    fault(id, "E0909", "checkpoint 전체 예약 상한은 16 MiB다.".into())
                })?;
            let mut writer = StateWriter::new(limit).map_err(|e| fault(id, "E0909", e))?;
            if let Some(kernel) = kernel {
                catch_unwind(AssertUnwindSafe(|| kernel.checkpoint(&mut writer)))
                    .map_err(|_| fault(id, "E0908", "상태 저장에서 panic이 발생했다.".into()))?
                    .map_err(|e| fault(id, "E0909", e))?;
            }
            payloads.push(writer.finish());
        }
        let mut scheduler_state =
            StateWriter::new(plan.queues.bytes).map_err(|e| fault(lead, "E0909", e))?;
        scheduler
            .checkpoint(&mut scheduler_state)
            .map_err(|e| fault(lead, "E0909", e))?;
        recovery.result = Some(Checkpoint {
            scheduler: scheduler_state.finish(),
            plan_digest: checkpoint::plan_digest(plan),
            position: report.input_offset + report.completed_samples,
            prefix_digest: prefix,
            kernels: payloads,
            grids,
        });
    }
    Ok(())
}
