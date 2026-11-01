//! 준비 단계에 예약한 간선 ring으로 다중 출력과 합류를 실행한다.
use crate::sequential::Recorder;
use bci_extension::{Inputs, Kernel, StateReader, StateWriter, Tick};
use bci_ir::plan::ExecutionPlan;
use bci_schema::{
    execution::{Frame, RunError, RunReport},
    input::GridCursor,
    kernel::KernelSpec,
};
use std::panic::{AssertUnwindSafe, catch_unwind};

struct Ring {
    frames: Vec<Frame>,
    head: usize,
    len: usize,
}
impl Ring {
    fn new(values: usize, channels: usize, capacity: usize) -> Result<Self, String> {
        Ok(Self {
            frames: (0..capacity)
                .map(|_| Frame::allocate(values, channels))
                .collect::<Result<_, _>>()?,
            head: 0,
            len: 0,
        })
    }
    fn front(&self) -> &Frame {
        &self.frames[self.head]
    }
    fn pop(&mut self) {
        self.head = (self.head + 1) % self.frames.len();
        self.len -= 1;
    }
    fn push(&mut self, frame: &Frame) -> Result<(), String> {
        if self.len == self.frames.len() {
            return Err(
                "간선 큐가 예약 용량에 도달했다. 합류 경로의 격자·진행 속도를 확인한다.".into(),
            );
        }
        let position = (self.head + self.len) % self.frames.len();
        self.frames[position].copy_from(frame);
        self.len += 1;
        Ok(())
    }
}
pub(crate) struct Scheduler {
    queues: Vec<Vec<Ring>>,
    routes: Vec<Vec<(usize, usize)>>,
    reset_marks: Vec<bool>,
}
impl Scheduler {
    pub fn new(plan: &ExecutionPlan) -> Result<Self, String> {
        let mut queues = Vec::new();
        let mut routes = vec![Vec::new(); plan.nodes.len()];
        for node in &plan.nodes {
            let mut inputs = Vec::new();
            for (port, &source) in node.inputs.iter().enumerate() {
                let (values, channels) = plan.nodes[source].kernel.layout()?;
                inputs.push(Ring::new(
                    values,
                    channels,
                    plan.queues.capacities[node.id][port],
                )?);
                routes[source].push((node.id, port));
            }
            queues.push(inputs);
        }
        Ok(Self {
            queues,
            routes,
            reset_marks: vec![false; plan.nodes.len()],
        })
    }
    pub fn reset_from(&mut self, plan: &ExecutionPlan, source: usize) {
        self.reset_marks.fill(false);
        self.reset_marks[source] = true;
        for node in &plan.nodes {
            if node.inputs.iter().any(|&id| self.reset_marks[id]) {
                self.reset_marks[node.id] = true;
                for ring in &mut self.queues[node.id] {
                    ring.head = 0;
                    ring.len = 0;
                }
            }
        }
    }
    pub fn is_empty(&self) -> bool {
        self.queues.iter().flatten().all(|q| q.len == 0)
    }
    pub fn pending(&self) -> usize {
        self.queues.iter().flatten().map(|q| q.len).sum()
    }
    fn publish(&mut self, source: usize, frame: &Frame) -> Result<(), String> {
        for &(node, port) in &self.routes[source] {
            self.queues[node][port].push(frame)?;
        }
        Ok(())
    }
    pub fn checkpoint(&self, output: &mut StateWriter) -> Result<(), String> {
        output.u64(1)?;
        for inputs in &self.queues {
            for ring in inputs {
                output.u64(ring.len as u64)?;
                for i in 0..ring.len {
                    output.frame(&ring.frames[(ring.head + i) % ring.frames.len()])?;
                }
            }
        }
        Ok(())
    }
    pub fn restore(
        &mut self,
        plan: &ExecutionPlan,
        grids: &[GridCursor],
        input: &mut StateReader<'_>,
    ) -> Result<(), String> {
        if input.u64()? != 1 {
            return Err("간선 큐 state codec 판본은 1을 요구한다.".into());
        }
        for node in &plan.nodes {
            for (port, ring) in self.queues[node.id].iter_mut().enumerate() {
                let length = input.u64()?;
                if length > ring.frames.len() as u64 {
                    return Err("복원 큐 길이를 예약 용량에 맞춘다.".into());
                }
                let source = node.inputs[port];
                let occurrence = &plan.nodes[source]
                    .output
                    .as_ref()
                    .expect("verified type")
                    .occurrence;
                let mut cursor = GridCursor::default();
                for frame in &mut ring.frames[..length as usize] {
                    input.frame(frame)?;
                    bci_schema::learning::typed_frame(
                        &plan.nodes[source]
                            .output
                            .as_ref()
                            .expect("verified type")
                            .value,
                        frame,
                    )?;
                    cursor.check(occurrence, frame)?;
                    if grids[source]
                        .last_time
                        .is_none_or(|time| !frame.time.compare(time).is_ok_and(|o| !o.is_gt()))
                        || grids[source]
                            .generation
                            .is_none_or(|g| frame.generation > g)
                    {
                        return Err("큐 레코드를 생산자 checkpoint 격자 범위와 맞춘다.".into());
                    }
                }
                ring.head = 0;
                ring.len = length as usize;
            }
        }
        input.finish()
    }
    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        plan: &ExecutionPlan,
        present: &[bool],
        kernels: &mut [Option<Box<dyn Kernel>>],
        frames: &mut [Frame],
        grids: &mut [GridCursor],
        recorder: &mut dyn Recorder,
        report: &mut RunReport,
        index: u64,
    ) -> Result<(), RunError> {
        let fault = |id: usize, code, message| RunError {
            code,
            node: Some(id),
            span: plan.nodes[id].span,
            message,
        };
        for &id in &plan.sources {
            if !present[id] {
                continue;
            }
            bci_schema::learning::typed_frame(
                &plan.nodes[id].output.as_ref().expect("source type").value,
                &frames[id],
            )
            .map_err(|e| fault(id, "E0904", e))?;
            report.nodes[id].observe(&frames[id]);
            self.publish(id, &frames[id])
                .map_err(|e| fault(id, "E0901", e))?;
        }
        for &id in &plan.schedule {
            let node = &plan.nodes[id];
            if matches!(node.kernel, KernelSpec::Source(_)) {
                continue;
            }
            if matches!(node.kernel, KernelSpec::Record { .. }) {
                for (port, &source) in node.inputs.iter().enumerate() {
                    let ring = &mut self.queues[id][port];
                    while ring.len > 0 {
                        recorder
                            .write(id, source, ring.front())
                            .map_err(|e| fault(id, "E0902", e))?;
                        report.nodes[id].observe(ring.front());
                        ring.pop();
                    }
                }
                continue;
            }
            while self.queues[id].iter().all(|q| q.len > 0) {
                for (port, &source) in node.inputs.iter().enumerate() {
                    frames[source].copy_from(self.queues[id][port].front());
                    self.queues[id][port].pop();
                }
                let (before, after) = frames.split_at_mut(id);
                let output = &mut after[0];
                let kernel = kernels[id].as_mut().expect("prepared kernel");
                let mut emitted = catch_unwind(AssertUnwindSafe(|| {
                    kernel.process_inputs(Inputs::new(before, &node.inputs), Tick { index }, output)
                }))
                .map_err(|_| {
                    fault(
                        id,
                        "E0908",
                        format!(
                            "{}: 원천 tick {index}의 커널 호출에서 panic이 발생했다.",
                            node.operator
                        ),
                    )
                })?
                .map_err(|e| fault(id, "E0903", e))?;
                let limit = match &node.kernel {
                    KernelSpec::Resample { specification, .. } => specification.burst(),
                    _ => 1,
                };
                let mut count = 0;
                loop {
                    if emitted {
                        count += 1;
                        if count > limit
                            || output.values.iter().any(|v| !v.is_finite())
                            || output.valid.iter().any(|&v| v > 1)
                        {
                            return Err(fault(
                                id,
                                "E0903",
                                "커널의 출력 burst·유한 수치·품질 계약을 확인한다.".into(),
                            ));
                        }
                        bci_schema::learning::typed_frame(
                            &node.output.as_ref().expect("verified output").value,
                            output,
                        )
                        .map_err(|e| fault(id, "E0903", e))?;
                        grids[id]
                            .check(
                                &node.output.as_ref().expect("verified output").occurrence,
                                output,
                            )
                            .map_err(|e| fault(id, "E0901", e))?;
                        report.nodes[id].observe(output);
                        self.publish(id, output)
                            .map_err(|e| fault(id, "E0901", e))?;
                    }
                    emitted = catch_unwind(AssertUnwindSafe(|| kernel.drain(output)))
                        .map_err(|_| {
                            fault(id, "E0908", "커널 출력 배출에서 panic이 발생했다.".into())
                        })?
                        .map_err(|e| fault(id, "E0903", e))?;
                    if !emitted {
                        break;
                    }
                }
            }
        }
        Ok(())
    }
}
