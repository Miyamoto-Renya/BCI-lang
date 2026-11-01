use bci_schema::{
    execution::{Frame, Origin},
    input::{InputError, InputSource, SourceFactory, SourceKind, SourceSpec},
    number::Rational,
    types::Occurrence,
};

/// 기존 합성 신호를 공통 입력 인터페이스로 생성한다.
pub struct SyntheticSources;
impl SourceFactory for SyntheticSources {
    fn open(&self, spec: &SourceSpec) -> Result<Box<dyn InputSource>, InputError> {
        let Occurrence::Uniform { rate, phase } = spec.stream.occurrence else {
            return Err(InputError::contract("합성 원천에 Uniform 격자를 지정한다."));
        };
        if spec.kind != SourceKind::Synthetic {
            return Err(InputError::contract("파일 입력은 FileSources에 연결한다."));
        }
        Ok(Box::new(Synthetic {
            rate,
            phase,
            index: 0,
        }))
    }
}
struct Synthetic {
    rate: Rational,
    phase: Rational,
    index: u64,
}
impl InputSource for Synthetic {
    fn read(&mut self, output: &mut Frame) -> Result<bool, InputError> {
        let relative = Rational::new(self.index as i128, 1)
            .and_then(|n| n.divide(self.rate))
            .map_err(InputError::contract)?;
        output.time = self
            .phase
            .checked_add(relative)
            .map_err(InputError::contract)?;
        output.sequence = self.index;
        output.source_first = self.index;
        output.source_last = self.index;
        output.generation = 0;
        output.origin = Origin::Synthetic;
        output.gap = None;
        output.flags = 0;
        output.valid.fill(1);
        let t = relative.to_f64();
        for (channel, value) in output.values.iter_mut().enumerate() {
            let angle = std::f64::consts::TAU * (10.0 + 3.0 * channel as f64) * t;
            *value = angle.sin() + 0.25 * (std::f64::consts::TAU * 3.0 * t + channel as f64).cos();
        }
        self.index = self
            .index
            .checked_add(1)
            .ok_or_else(|| InputError::contract("합성 표본 index 범위를 초과했다."))?;
        Ok(true)
    }
}
