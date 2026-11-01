use bci_extension::{Kernel, StateReader, StateWriter, Tick};
use bci_schema::{
    execution::{Frame, INVALID_INPUT, Origin, WARMUP},
    iir::Realization,
};

pub(crate) struct Iir {
    channels: usize,
    coefficients: Vec<[f64; 5]>,
    delays: Vec<[f64; 2]>,
    counts: Vec<u64>,
    warmup: u64,
    previous: Option<(u64, u64)>,
    first: Option<u64>,
    origin: Option<Origin>,
    faulted: bool,
}
impl Iir {
    pub fn new(channels: usize, realization: &Realization) -> Result<Self, String> {
        realization.validate()?;
        let size = channels
            .checked_mul(realization.sections.len())
            .ok_or("IIR 상태 크기 범위를 초과했다.")?;
        Ok(Self {
            channels,
            coefficients: realization
                .sections
                .iter()
                .map(|s| s.map(|n| n.to_f64()))
                .collect(),
            delays: crate::reserved(size, [0.0; 2])?,
            counts: crate::reserved(channels, 0)?,
            warmup: realization.warmup as u64,
            previous: None,
            first: None,
            origin: None,
            faulted: false,
        })
    }
}
impl Kernel for Iir {
    fn process(
        &mut self,
        input: Option<&Frame>,
        _: Tick,
        output: &mut Frame,
    ) -> Result<bool, String> {
        if self.faulted {
            return Err("IIR Fault 이후 새 객체나 checkpoint로 상태를 복원한다.".into());
        }
        self.faulted = true;
        let input = crate::input(input)?;
        if [
            input.values.len(),
            input.valid.len(),
            output.values.len(),
            output.valid.len(),
        ]
        .iter()
        .any(|&n| n != self.channels)
        {
            return Err("IIR 배열 크기를 채널 계약과 맞춘다.".into());
        }
        if input.gap.is_some() || self.previous.is_some_and(|(g, _)| g != input.generation) {
            self.delays.fill([0.0; 2]);
            self.counts.fill(0);
            self.first = None;
            self.origin = None;
        }
        output.inherit(input);
        let first = self
            .first
            .map_or(input.source_first, |n| n.min(input.source_first));
        let origin = self
            .origin
            .map_or(input.origin, |o| o.combine(input.origin));
        let sections = self.coefficients.len();
        for c in 0..self.channels {
            let states = &mut self.delays[c * sections..(c + 1) * sections];
            if input.valid[c] == 0 {
                states.fill([0.0; 2]);
                self.counts[c] = 0;
                output.values[c] = 0.0;
                output.flags |= INVALID_INPUT;
                continue;
            }
            let mut x = input.values[c];
            for (state, coefficient) in states.iter_mut().zip(&self.coefficients) {
                let [b0, b1, b2, a1, a2] = *coefficient;
                let y = b0 * x + state[0];
                let z1 = b1 * x - a1 * y + state[1];
                let z2 = b2 * x - a2 * y;
                if [y, z1, z2].iter().any(|v| !v.is_finite()) {
                    return Err(
                        "IIR 출력·지연 상태가 비유한 값이다. 입력 크기와 SOS 계수를 확인한다."
                            .into(),
                    );
                }
                *state = [z1, z2];
                x = y;
            }
            output.values[c] = x;
            if self.counts[c] < self.warmup {
                output.valid[c] = 0;
                output.flags |= WARMUP;
                self.counts[c] += 1;
            }
        }
        output.source_first = first;
        output.origin = origin;
        self.previous = Some((input.generation, input.sequence));
        self.first = Some(first);
        self.origin = Some(origin);
        self.faulted = false;
        Ok(true)
    }
    fn checkpoint(&self, out: &mut StateWriter) -> Result<(), String> {
        if self.faulted {
            return Err("IIR checkpoint는 완료한 호출 경계를 요구한다.".into());
        }
        out.u64(1)?;
        out.optional(self.previous.map(|p| p.0))?;
        out.optional(self.previous.map(|p| p.1))?;
        out.optional(self.first)?;
        out.bytes(&[self.origin.map_or(255, |o| o as u8)])?;
        for &n in &self.counts {
            out.u64(n)?;
        }
        for pair in &self.delays {
            for &v in pair {
                out.f64(v)?;
            }
        }
        Ok(())
    }
    fn restore(&mut self, input: &mut StateReader<'_>) -> Result<(), String> {
        self.faulted = true;
        if input.u64()? != 1 {
            return Err("IIR state codec 판본은 1을 요구한다.".into());
        }
        let previous = crate::state::previous(input)?;
        let first = input.optional()?;
        let code = input.bytes(1)?[0];
        let origin = if code == 255 {
            None
        } else {
            Some(Origin::from_code(code)?)
        };
        if previous.is_some() != first.is_some()
            || previous.is_some() != origin.is_some()
            || first.zip(previous).is_some_and(|(f, (_, n))| f > n)
        {
            return Err("IIR generation·원천 범위·origin을 함께 복원한다.".into());
        }
        for n in &mut self.counts {
            *n = input.u64()?;
            if *n > self.warmup {
                return Err("IIR warmup 카운터가 상한을 초과했다.".into());
            }
        }
        for pair in &mut self.delays {
            for v in pair {
                *v = input.f64()?;
            }
        }
        input.finish()?;
        if previous.is_none()
            && (self.counts.iter().any(|&n| n != 0)
                || self.delays.iter().flatten().any(|&n| n != 0.0))
        {
            return Err("처리 전 IIR checkpoint의 상태는 0을 요구한다.".into());
        }
        self.previous = previous;
        self.first = first;
        self.origin = origin;
        self.faulted = false;
        Ok(())
    }
}
