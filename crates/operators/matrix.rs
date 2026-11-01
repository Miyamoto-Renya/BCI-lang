use bci_extension::{Kernel, Tick};
use bci_schema::execution::{Frame, INVALID_INPUT};

pub(crate) struct Covariance {
    channels: usize,
    samples: usize,
    means: Vec<f64>,
}
impl Covariance {
    pub fn new(channels: usize, samples: usize) -> Result<Self, String> {
        if samples < 2 {
            return Err("공분산은 두 표본 이상을 요구한다.".into());
        }
        Ok(Self {
            channels,
            samples,
            means: crate::reserved(channels, 0.0)?,
        })
    }
}
impl Kernel for Covariance {
    fn checkpoint(&self, _: &mut bci_extension::StateWriter) -> Result<(), String> {
        Ok(())
    }
    fn restore(&mut self, input: &mut bci_extension::StateReader<'_>) -> Result<(), String> {
        self.means.fill(0.0);
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
        if input.valid.contains(&0) {
            output.valid.fill(0);
            output.values.fill(0.0);
            output.flags |= INVALID_INPUT;
            return Ok(true);
        }
        for c in 0..self.channels {
            self.means[c] = input.values[c * self.samples..(c + 1) * self.samples]
                .iter()
                .sum::<f64>()
                / self.samples as f64;
        }
        for row in 0..self.channels {
            for column in row..self.channels {
                let mut sum = 0.0;
                for n in 0..self.samples {
                    sum += (input.values[row * self.samples + n] - self.means[row])
                        * (input.values[column * self.samples + n] - self.means[column]);
                }
                let value = sum / (self.samples - 1) as f64;
                output.values[row * self.channels + column] = value;
                output.values[column * self.channels + row] = value;
            }
        }
        Ok(true)
    }
}

pub(crate) struct Regularize {
    channels: usize,
    shrinkage: f64,
    floor: f64,
    cholesky: Vec<f64>,
}
impl Regularize {
    pub fn new(channels: usize, shrinkage: f64, floor: f64) -> Result<Self, String> {
        let count = channels
            .checked_mul(channels)
            .ok_or("행렬 크기 범위를 초과했다.")?;
        Ok(Self {
            channels,
            shrinkage,
            floor,
            cholesky: crate::reserved(count, 0.0)?,
        })
    }
}
impl Kernel for Regularize {
    fn checkpoint(&self, _: &mut bci_extension::StateWriter) -> Result<(), String> {
        Ok(())
    }
    fn restore(&mut self, input: &mut bci_extension::StateReader<'_>) -> Result<(), String> {
        self.cholesky.fill(0.0);
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
        if input.valid.contains(&0) {
            output.valid.fill(0);
            output.values.fill(0.0);
            return Ok(true);
        }
        let c = self.channels;
        let trace = (0..c).map(|i| input.values[i * c + i]).sum::<f64>();
        if !trace.is_finite() {
            return Err("공분산 trace가 유한 범위를 초과했다. 입력 크기와 단위를 조정한다.".into());
        }
        let variance = (trace / c as f64).max(self.floor);
        for row in 0..c {
            for column in 0..c {
                output.values[row * c + column] = (1.0 - self.shrinkage)
                    * input.values[row * c + column]
                    + if row == column {
                        self.shrinkage * variance
                    } else {
                        0.0
                    };
            }
        }
        self.cholesky.fill(0.0);
        for row in 0..c {
            for column in 0..=row {
                let correction = (0..column)
                    .map(|k| self.cholesky[row * c + k] * self.cholesky[column * c + k])
                    .sum::<f64>();
                let residual = output.values[row * c + column] - correction;
                let value = if row == column {
                    if !residual.is_finite() || residual <= 0.0 {
                        return Err("Cholesky 대각 잔차가 양의 유한 범위를 벗어났다. shrinkage·floor와 입력 수치 범위를 조정한다.".into());
                    }
                    residual.sqrt()
                } else {
                    residual / self.cholesky[column * c + column]
                };
                if !value.is_finite() {
                    return Err(
                        "Cholesky 값이 유한 범위를 초과했다. 입력 수치 범위를 조정한다.".into(),
                    );
                }
                self.cholesky[row * c + column] = value;
            }
        }
        Ok(true)
    }
}
