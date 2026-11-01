use bci_extension::{Kernel, StateReader, StateWriter, Tick};
use bci_schema::{
    execution::{Frame, GAP, Gap, INVALID_INPUT, WARMUP},
    input::grid_index,
    number::Rational,
    resample::Resampling,
};

pub(crate) struct Resample {
    spec: Resampling,
    offset: u64,
    output_rate: Rational,
    history: Vec<Frame>,
    cursor: usize,
    filled: usize,
    index: Option<u64>,
    next: u64,
    end: u64,
    gap: Option<Gap>,
}
impl Resample {
    pub fn new(channels: usize, spec: &Resampling) -> Result<Self, String> {
        spec.offset()?;
        let history = (0..spec.history())
            .map(|_| Frame::allocate(channels, channels))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            spec: spec.clone(),
            offset: spec.offset()?,
            output_rate: spec.output_rate()?,
            history,
            cursor: 0,
            filled: 0,
            index: None,
            next: 0,
            end: 0,
            gap: None,
        })
    }
    fn emit(&mut self, output: &mut Frame) -> Result<bool, String> {
        let high = self
            .next
            .checked_mul(self.spec.down as u64)
            .and_then(|n| n.checked_add(self.offset))
            .ok_or("재표본화 출력 index 범위를 초과했다.")?;
        if high >= self.end {
            return Ok(false);
        }
        let current = self.index.ok_or("재표본화 입력 이력을 연결한다.")?;
        let newest = &self.history[(self.cursor + self.history.len() - 1) % self.history.len()];
        output.inherit(newest);
        output.sequence = self.next;
        output.time = self
            .spec
            .phase
            .checked_add(Rational::new(self.next as i128, 1)?.divide(self.output_rate)?)?;
        output.values.fill(0.0);
        output.valid.fill(1);
        output.flags = 0;
        output.gap = self.gap.take();
        if output.gap.is_some() {
            output.flags |= GAP;
        }
        let mut contributed = false;
        let phase = (high % self.spec.up as u64) as usize;
        for k in (phase..self.spec.coefficients.len()).step_by(self.spec.up) {
            let coefficient = self.spec.coefficients[k];
            if coefficient == 0.0 {
                continue;
            }
            let lag = high
                .checked_sub(k as u64)
                .map(|n| n / self.spec.up as u64)
                .and_then(|n| current.checked_sub(n));
            let Some(lag) = lag.filter(|&lag| lag < self.filled as u64) else {
                output.valid.fill(0);
                output.flags |= WARMUP;
                continue;
            };
            let frame = &self.history
                [(self.cursor + self.history.len() - 1 - lag as usize) % self.history.len()];
            if contributed {
                output.source_first = output.source_first.min(frame.source_first);
                output.source_last = output.source_last.max(frame.source_last);
                output.origin = output.origin.combine(frame.origin);
            } else {
                output.source_first = frame.source_first;
                output.source_last = frame.source_last;
                output.origin = frame.origin;
                contributed = true;
            }
            output.flags |= frame.flags & (WARMUP | INVALID_INPUT);
            for c in 0..output.values.len() {
                output.values[c] += coefficient * frame.values[c];
                output.valid[c] &= frame.valid[c];
                if frame.valid[c] == 0 {
                    output.flags |= INVALID_INPUT;
                }
            }
        }
        // 부분 support의 수치는 유지하고 valid·Warmup으로 경계를 표시한다.
        self.next = self
            .next
            .checked_add(1)
            .ok_or("재표본화 출력 index 범위를 초과했다.")?;
        Ok(true)
    }
}
impl Kernel for Resample {
    fn process(
        &mut self,
        input: Option<&Frame>,
        _: Tick,
        output: &mut Frame,
    ) -> Result<bool, String> {
        let input = crate::input(input)?;
        let index = grid_index(input.time, self.spec.rate, self.spec.input_phase)?;
        let reset = self.index.is_none()
            || input.gap.is_some()
            || self.history[(self.cursor + self.history.len() - 1) % self.history.len()].generation
                != input.generation;
        if reset {
            self.filled = 0;
            self.cursor = 0;
            let high = index
                .checked_mul(self.spec.up as u64)
                .ok_or("재표본화 입력 index 범위를 초과했다.")?;
            self.next = high
                .saturating_sub(self.offset)
                .div_ceil(self.spec.down as u64);
            self.gap = input.gap;
        } else if self.index.and_then(|n| n.checked_add(1)) != Some(index) {
            return Err("재표본화 입력은 generation 안에서 연속 격자를 요구한다.".into());
        }
        self.history[self.cursor].copy_from(input);
        self.cursor = (self.cursor + 1) % self.history.len();
        self.filled = (self.filled + 1).min(self.history.len());
        self.index = Some(index);
        self.end = index
            .checked_mul(self.spec.up as u64)
            .and_then(|n| n.checked_add(1))
            .ok_or("재표본화 입력 index 범위를 초과했다.")?;
        self.emit(output)
    }
    fn drain(&mut self, output: &mut Frame) -> Result<bool, String> {
        self.emit(output)
    }
    fn checkpoint(&self, output: &mut StateWriter) -> Result<(), String> {
        output.u64(1)?;
        output.u64(self.cursor as u64)?;
        output.u64(self.filled as u64)?;
        output.optional(self.index)?;
        output.u64(self.next)?;
        output.u64(self.end)?;
        output.optional(self.gap.map(|g| g.first))?;
        output.optional(self.gap.map(|g| g.last))?;
        for frame in &self.history {
            output.frame(frame)?;
        }
        Ok(())
    }
    fn restore(&mut self, input: &mut StateReader<'_>) -> Result<(), String> {
        if input.u64()? != 1 {
            return Err("재표본화 state codec 판본은 1을 요구한다.".into());
        }
        let cursor = input.u64()?;
        let filled = input.u64()?;
        let index = input.optional()?;
        let next = input.u64()?;
        let end = input.u64()?;
        let gap = match (input.optional()?, input.optional()?) {
            (None, None) => None,
            (Some(first), Some(last)) if first <= last => Some(Gap { first, last }),
            _ => return Err("재표본화 Gap 상태를 확인한다.".into()),
        };
        if cursor >= self.history.len() as u64
            || filled > self.history.len() as u64
            || (filled < self.history.len() as u64 && cursor != filled)
            || (filled == 0) != index.is_none()
            || index.is_some_and(|n| filled > n.saturating_add(1))
            || index.map_or(end != 0, |n| {
                n.checked_mul(self.spec.up as u64)
                    .and_then(|n| n.checked_add(1))
                    != Some(end)
            })
            || next
                != end
                    .saturating_sub(self.offset)
                    .div_ceil(self.spec.down as u64)
        {
            return Err("재표본화 cursor·입력 이력·출력 경계를 맞춘다.".into());
        }
        for frame in &mut self.history {
            input.frame(frame)?;
        }
        input.finish()?;
        if let Some(index) = index {
            let generation = self.history
                [(cursor as usize + self.history.len() - 1) % self.history.len()]
            .generation;
            for lag in 0..filled as usize {
                let frame = &self.history
                    [(cursor as usize + self.history.len() - 1 - lag) % self.history.len()];
                if grid_index(frame.time, self.spec.rate, self.spec.input_phase)?
                    != index - lag as u64
                    || frame.generation != generation
                {
                    return Err("재표본화 이력 시각을 저장한 입력 index와 맞춘다.".into());
                }
            }
        }
        self.cursor = cursor as usize;
        self.filled = filled as usize;
        self.index = index;
        self.next = next;
        self.end = end;
        self.gap = gap;
        Ok(())
    }
}
