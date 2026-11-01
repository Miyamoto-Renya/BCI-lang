use bci_extension::{Kernel, Tick};
use bci_schema::execution::{Frame, INVALID_INPUT};

pub(crate) struct Window {
    channels: usize,
    size: usize,
    hop: u64,
    start: Option<u64>,
    phase: Option<u64>,
    values: Vec<f64>,
    valid: Vec<u8>,
    first: Vec<u64>,
    last: Vec<u64>,
    flags: Vec<u8>,
    cursor: usize,
    filled: usize,
    consecutive: usize,
    previous: Option<(u64, u64)>,
    origins: Vec<u8>,
}
impl Window {
    pub fn new(
        channels: usize,
        size: usize,
        hop: usize,
        start: Option<u64>,
    ) -> Result<Self, String> {
        if size == 0 || hop == 0 {
            return Err("창 크기와 hop은 양수를 요구한다.".into());
        }
        let count = channels
            .checked_mul(size)
            .ok_or("창 상태 크기 범위를 초과했다.")?;
        Ok(Self {
            channels,
            size,
            hop: hop as u64,
            start,
            phase: start,
            values: crate::reserved(count, 0.0)?,
            valid: crate::reserved(count, 0)?,
            first: crate::reserved(size, 0)?,
            last: crate::reserved(size, 0)?,
            flags: crate::reserved(size, 0)?,
            cursor: 0,
            filled: 0,
            consecutive: 0,
            previous: None,
            origins: crate::reserved(size, 0)?,
        })
    }
}
impl Kernel for Window {
    fn checkpoint(&self, output: &mut bci_extension::StateWriter) -> Result<(), String> {
        output.u64(2)?;
        output.u64(self.cursor as u64)?;
        output.u64(self.filled as u64)?;
        output.u64(self.consecutive as u64)?;
        output.optional(self.phase)?;
        output.optional(self.previous.map(|p| p.0))?;
        output.optional(self.previous.map(|p| p.1))?;
        crate::state::write(output, &self.values, &self.valid, &self.first, &self.last)?;
        output.bytes(&self.origins)?;
        output.bytes(&self.flags)
    }
    fn restore(&mut self, input: &mut bci_extension::StateReader<'_>) -> Result<(), String> {
        if input.u64()? != 2 {
            return Err("창 state codec 판본은 2를 요구한다.".into());
        }
        let cursor = input.u64()?;
        let filled = input.u64()?;
        let consecutive = input.u64()?;
        let phase = input.optional()?;
        let previous = crate::state::previous(input)?;
        if cursor >= self.size as u64
            || filled > self.size as u64
            || consecutive > filled
            || (filled == 0) != previous.is_none()
            || self.start.is_some_and(|s| phase != Some(s))
            || phase
                .zip(previous)
                .is_some_and(|(p, (_, n))| self.start.is_none() && p > n)
        {
            return Err("창 cursor·filled·phase·previous를 맞춘다.".into());
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
        self.flags.copy_from_slice(input.bytes(self.size)?);
        input.finish()?;
        if self.flags.iter().any(|&f| f > 7) {
            return Err("창 품질 flags의 범위를 맞춘다.".into());
        }
        self.cursor = cursor as usize;
        self.filled = filled as usize;
        self.consecutive = consecutive as usize;
        self.phase = phase;
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
        if self.previous.is_some_and(|(generation, n)| {
            generation != input.generation || n.checked_add(1) != Some(input.sequence)
        }) {
            self.filled = 0;
            self.consecutive = 0;
            self.cursor = 0;
            self.phase = self.start;
        }
        self.previous = Some((input.generation, input.sequence));
        self.origins[self.cursor] = input.origin as u8;
        for c in 0..self.channels {
            self.values[c * self.size + self.cursor] = input.values[c];
            self.valid[c * self.size + self.cursor] = input.valid[c];
        }
        self.first[self.cursor] = input.source_first;
        self.last[self.cursor] = input.source_last;
        self.flags[self.cursor] = input.flags;
        self.cursor = (self.cursor + 1) % self.size;
        self.filled = (self.filled + 1).min(self.size);
        self.consecutive = if input.valid.iter().all(|&v| v == 1) {
            (self.consecutive + 1).min(self.size)
        } else {
            0
        };
        if self.phase.is_none() && self.consecutive == self.size {
            self.phase = Some(input.sequence + 1 - self.size as u64);
        }
        let Some(start) = self.phase else {
            return Ok(false);
        };
        let end = start
            .checked_add(self.size as u64 - 1)
            .ok_or("창 index 범위를 초과했다.")?;
        if self.filled < self.size
            || input.sequence < end
            || !(input.sequence - end).is_multiple_of(self.hop)
        {
            return Ok(false);
        }
        output.inherit(input);
        output.flags = 0;
        for c in 0..self.channels {
            let mut valid = 1;
            for sample in 0..self.size {
                let position = (self.cursor + sample) % self.size;
                output.values[c * self.size + sample] = self.values[c * self.size + position];
                valid &= self.valid[c * self.size + position];
                output.flags |= self.flags[position];
                output.origin = output
                    .origin
                    .combine(bci_schema::execution::Origin::from_code(
                        self.origins[position],
                    )?);
            }
            output.valid[c] = valid;
            if valid == 0 {
                output.flags |= INVALID_INPUT;
            }
        }
        output.source_first = self.first[self.cursor];
        output.source_last = self.last[(self.cursor + self.size - 1) % self.size];
        Ok(true)
    }
}
