use bci_extension::{Kernel, StateReader, StateWriter, Tick};
use bci_schema::{
    execution::{Frame, INVALID_INPUT},
    number::Rational,
    spectral::{Band, Taper},
};

pub(crate) struct Psd {
    channels: usize,
    samples: usize,
    window: Vec<f64>,
    cosine: Vec<f64>,
    sine: Vec<f64>,
    denominator: f64,
}
impl Psd {
    pub fn new(
        channels: usize,
        samples: usize,
        rate: Rational,
        taper: Taper,
    ) -> Result<Self, String> {
        let mut window = crate::reserved(samples, 1.0)?;
        let mut cosine = crate::reserved(samples, 0.0)?;
        let mut sine = crate::reserved(samples, 0.0)?;
        for n in 0..samples {
            let angle = std::f64::consts::TAU * n as f64 / samples as f64;
            (sine[n], cosine[n]) = angle.sin_cos();
            if n == 0 {
                sine[n] = 0.0;
                cosine[n] = 1.0;
            }
            if samples.is_multiple_of(2) && n == samples / 2 {
                sine[n] = 0.0;
                cosine[n] = -1.0;
            }
            if taper == Taper::Hann {
                window[n] = 0.5 * (1.0 - cosine[n]);
            }
        }
        let denominator = rate.to_f64() * window.iter().map(|w| w * w).sum::<f64>();
        if !denominator.is_finite() || denominator <= 0.0 {
            return Err("PSD 표본률·창 에너지의 곱은 양의 유한 수치를 요구한다.".into());
        }
        Ok(Self {
            channels,
            samples,
            window,
            cosine,
            sine,
            denominator,
        })
    }
}
impl Kernel for Psd {
    fn checkpoint(&self, _: &mut StateWriter) -> Result<(), String> {
        Ok(())
    }
    fn restore(&mut self, input: &mut StateReader<'_>) -> Result<(), String> {
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
        let bins = self.samples / 2 + 1;
        output.values.fill(0.0);
        for c in 0..self.channels {
            if input.valid[c] == 0 {
                output.flags |= INVALID_INPUT;
                continue;
            }
            for k in 0..bins {
                let (mut real, mut imaginary) = (0.0, 0.0);
                for n in 0..self.samples {
                    let value = self.window[n] * input.values[c * self.samples + n];
                    let phase = (k * n) % self.samples;
                    real += value * self.cosine[phase];
                    imaginary -= value * self.sine[phase];
                }
                let eta = if k == 0 || (self.samples.is_multiple_of(2) && k == self.samples / 2) {
                    1.0
                } else {
                    2.0
                };
                output.values[c * bins + k] =
                    (real * real + imaginary * imaginary) / self.denominator * eta;
            }
        }
        Ok(true)
    }
}

pub(crate) struct BandPower {
    channels: usize,
    bins: usize,
    first: usize,
    end: usize,
    step: f64,
}
impl BandPower {
    pub fn new(
        channels: usize,
        samples: usize,
        rate: Rational,
        band: &Band,
    ) -> Result<Self, String> {
        let (first, end) = bci_schema::spectral::bins(samples, rate, band)?;
        Ok(Self {
            channels,
            bins: samples / 2 + 1,
            first,
            end,
            step: rate.divide(Rational::integer(samples as i64))?.to_f64(),
        })
    }
}
impl Kernel for BandPower {
    fn checkpoint(&self, _: &mut StateWriter) -> Result<(), String> {
        Ok(())
    }
    fn restore(&mut self, input: &mut StateReader<'_>) -> Result<(), String> {
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
        output.values.fill(0.0);
        for c in 0..self.channels {
            if input.valid[c] == 0 {
                output.flags |= INVALID_INPUT;
                continue;
            }
            let mut sum = 0.0;
            for &value in &input.values[c * self.bins + self.first..c * self.bins + self.end] {
                if value < 0.0 {
                    return Err("대역 전력의 PSD 값은 0 이상을 요구한다.".into());
                }
                sum += value;
            }
            output.values[c] = sum * self.step;
        }
        Ok(true)
    }
}

pub(crate) struct LogPower {
    floor: f64,
    reference: f64,
}
impl LogPower {
    pub fn new(floor: Rational, reference: Rational) -> Self {
        Self {
            floor: floor.to_f64(),
            reference: reference.to_f64(),
        }
    }
}
impl Kernel for LogPower {
    fn checkpoint(&self, _: &mut StateWriter) -> Result<(), String> {
        Ok(())
    }
    fn restore(&mut self, input: &mut StateReader<'_>) -> Result<(), String> {
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
        for c in 0..output.values.len() {
            output.values[c] = if input.valid[c] == 0 {
                output.flags |= INVALID_INPUT;
                0.0
            } else {
                let value = input.values[c];
                if value < 0.0 {
                    return Err("로그 특징의 입력 전력은 0 이상을 요구한다.".into());
                }
                ((value + self.floor) / self.reference).ln()
            };
        }
        Ok(true)
    }
}
