use bci_extension::{Kernel, Tick};
use bci_schema::execution::{Frame, GAP, INVALID_INPUT, WARMUP};

pub(crate) struct Scale {
    factors: Vec<f64>,
    width: usize,
}
impl Scale {
    pub fn new(factors: &[f64], width: usize) -> Self {
        Self {
            factors: factors.to_vec(),
            width,
        }
    }
}
impl Kernel for Scale {
    fn checkpoint(&self, _: &mut bci_extension::StateWriter) -> Result<(), String> {
        Ok(())
    }
    fn restore(&mut self, input: &mut bci_extension::StateReader<'_>) -> Result<(), String> {
        input.finish()
    }
    fn process(
        &mut self,
        input: Option<&Frame>,
        _: Tick,
        output: &mut Frame,
    ) -> Result<bool, String> {
        let input = crate::input(input)?;
        output.inherit(input);
        for (channel, &factor) in self.factors.iter().enumerate() {
            for column in 0..self.width {
                let index = channel * self.width + column;
                output.values[index] = input.values[index] * factor;
            }
        }
        Ok(true)
    }
}

pub(crate) struct Fir {
    channels: usize,
    coefficients: Vec<f64>,
    values: Vec<f64>,
    valid: Vec<u8>,
    first: Vec<u64>,
    last: Vec<u64>,
    cursor: usize,
    filled: usize,
    previous: Option<(u64, u64)>,
    origins: Vec<u8>,
}
impl Fir {
    pub fn new(channels: usize, coefficients: &[f64]) -> Result<Self, String> {
        let length = coefficients.len();
        if length == 0 {
            return Err("FIR 계수를 한 개 이상 지정한다.".into());
        }
        let size = channels
            .checked_mul(length)
            .ok_or("FIR 상태 크기 범위를 초과했다.")?;
        Ok(Self {
            channels,
            coefficients: coefficients.to_vec(),
            values: crate::reserved(size, 0.0)?,
            valid: crate::reserved(size, 0)?,
            first: crate::reserved(length, 0)?,
            last: crate::reserved(length, 0)?,
            cursor: 0,
            filled: 0,
            previous: None,
            origins: crate::reserved(length, 0)?,
        })
    }
}
impl Kernel for Fir {
    fn checkpoint(&self, output: &mut bci_extension::StateWriter) -> Result<(), String> {
        output.u64(2)?;
        output.u64(self.cursor as u64)?;
        output.u64(self.filled as u64)?;
        output.optional(self.previous.map(|p| p.0))?;
        output.optional(self.previous.map(|p| p.1))?;
        crate::state::write(output, &self.values, &self.valid, &self.first, &self.last)?;
        output.bytes(&self.origins)
    }
    fn restore(&mut self, input: &mut bci_extension::StateReader<'_>) -> Result<(), String> {
        if input.u64()? != 2 {
            return Err("FIR state codec 판본은 2를 요구한다.".into());
        }
        let cursor = input.u64()?;
        let filled = input.u64()?;
        let previous = crate::state::previous(input)?;
        if cursor >= self.coefficients.len() as u64
            || filled > self.coefficients.len() as u64
            || (filled == 0) != previous.is_none()
        {
            return Err("FIR cursor·filled·previous를 맞춘다.".into());
        }
        crate::state::read(
            input,
            &mut self.values,
            &mut self.valid,
            &mut self.first,
            &mut self.last,
        )?;
        let origin_count = self.origins.len();
        self.origins.copy_from_slice(input.bytes(origin_count)?);
        for &code in &self.origins {
            bci_schema::execution::Origin::from_code(code)?;
        }
        input.finish()?;
        self.cursor = cursor as usize;
        self.filled = filled as usize;
        self.previous = previous;
        Ok(())
    }
    fn process(
        &mut self,
        input: Option<&Frame>,
        _: Tick,
        output: &mut Frame,
    ) -> Result<bool, String> {
        let input = crate::input(input)?;
        let length = self.coefficients.len();
        if self.previous.is_some_and(|(generation, n)| {
            generation != input.generation || n.checked_add(1) != Some(input.sequence)
        }) {
            self.values.fill(0.0);
            self.valid.fill(0);
            self.cursor = 0;
            self.filled = 0;
        }
        self.previous = Some((input.generation, input.sequence));
        self.origins[self.cursor] = input.origin as u8;
        self.first[self.cursor] = input.source_first;
        self.last[self.cursor] = input.source_last;
        self.filled = (self.filled + 1).min(length);
        output.inherit(input);
        for lag in 0..self.filled {
            output.origin = output
                .origin
                .combine(bci_schema::execution::Origin::from_code(
                    self.origins[(self.cursor + length - lag) % length],
                )?);
        }
        output.flags = (input.flags & GAP) | if self.filled < length { WARMUP } else { 0 };
        for channel in 0..self.channels {
            self.values[channel * length + self.cursor] = input.values[channel];
            self.valid[channel * length + self.cursor] = input.valid[channel];
            let mut sum = 0.0;
            let mut valid = 1;
            for (lag, &coefficient) in self.coefficients.iter().enumerate() {
                let position = (self.cursor + length - lag) % length;
                sum += coefficient * self.values[channel * length + position];
                valid &= self.valid[channel * length + position];
            }
            output.values[channel] = sum;
            output.valid[channel] = valid;
            if valid == 0 && self.filled == length {
                output.flags |= INVALID_INPUT;
            }
        }
        let oldest = (self.cursor + length + 1 - self.filled) % length;
        output.source_first = self.first[oldest];
        output.source_last = self.last[self.cursor];
        self.cursor = (self.cursor + 1) % length;
        Ok(true)
    }
}
