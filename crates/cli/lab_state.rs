use bci_extension::{ResetCause, SampleView, StateReader, StateWriter, StatefulOperator, operator};

/// 유효 입력의 채널별 누적 평균.
pub struct RunningMean {
    sums: Vec<f64>,
    counts: Vec<u64>,
}

#[operator(
    name = "lab.signal.running_mean",
    version = "0.1.0",
    kind = "stateful",
    contract = "running_mean.operator.toml"
)]
impl StatefulOperator for RunningMean {
    fn prepare(channels: usize) -> Result<Self, String> {
        let mut sums = Vec::new();
        let mut counts = Vec::new();
        sums.try_reserve_exact(channels)
            .map_err(|e| e.to_string())?;
        counts
            .try_reserve_exact(channels)
            .map_err(|e| e.to_string())?;
        sums.resize(channels, 0.0);
        counts.resize(channels, 0);
        Ok(Self { sums, counts })
    }
    fn state_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + (self.sums.capacity() + self.counts.capacity()) * 8
    }
    fn step(
        &mut self,
        input: SampleView<'_>,
        output: &mut [f64],
        _: &mut [u8],
    ) -> Result<(), String> {
        for (c, value) in output.iter_mut().enumerate() {
            if input.valid[c] == 1 {
                self.sums[c] += input.values[c];
                self.counts[c] = self.counts[c]
                    .checked_add(1)
                    .ok_or("평균 count 범위를 초과했다.")?;
                *value = self.sums[c] / self.counts[c] as f64;
            }
        }
        Ok(())
    }
    fn reset(&mut self, _: ResetCause) -> Result<(), String> {
        self.sums.fill(0.0);
        self.counts.fill(0);
        Ok(())
    }
    fn checkpoint(&self, output: &mut StateWriter) -> Result<(), String> {
        for (&sum, &count) in self.sums.iter().zip(&self.counts) {
            output.f64(sum)?;
            output.u64(count)?;
        }
        Ok(())
    }
    fn restore(&mut self, input: &mut StateReader<'_>) -> Result<(), String> {
        for (sum, count) in self.sums.iter_mut().zip(&mut self.counts) {
            *sum = input.f64()?;
            *count = input.u64()?;
            if *count > 1_000_000_000 || (*count == 0 && *sum != 0.0) {
                return Err("평균 count·sum 불변식을 맞춘다.".into());
            }
        }
        input.finish()
    }
}
