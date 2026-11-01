use bci_extension::{Kernel, StateReader, StateWriter, Tick};
use bci_schema::{
    execution::{Frame, INVALID_INPUT},
    number::Rational,
};

pub(crate) struct ChannelMap {
    input_channels: usize,
    indices: Vec<usize>,
    width: usize,
}
impl ChannelMap {
    pub fn new(input_channels: usize, indices: &[usize], width: usize) -> Self {
        Self {
            input_channels,
            indices: indices.to_vec(),
            width,
        }
    }
}
impl Kernel for ChannelMap {
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
        if input.values.len() != self.input_channels * self.width
            || input.valid.len() != self.input_channels
            || output.values.len() != self.indices.len() * self.width
            || output.valid.len() != self.indices.len()
        {
            return Err("채널 매핑의 입출력 배열 크기를 계획과 맞춘다.".into());
        }
        output.inherit_metadata(input);
        output.flags &= !INVALID_INPUT;
        for (c, &source) in self.indices.iter().enumerate() {
            output.values[c * self.width..(c + 1) * self.width]
                .copy_from_slice(&input.values[source * self.width..(source + 1) * self.width]);
            output.valid[c] = input.valid[source];
            if output.valid[c] == 0 {
                output.flags |= INVALID_INPUT;
            }
        }
        Ok(true)
    }
}

pub(crate) struct Rereference {
    off_diagonal: Vec<f64>,
    diagonal: Vec<f64>,
    width: usize,
}
impl Rereference {
    pub fn new(weights: &[Rational], width: usize) -> Result<Self, String> {
        let mut off_diagonal = crate::reserved(weights.len(), 0.0)?;
        let mut diagonal = crate::reserved(weights.len(), 0.0)?;
        for (c, &weight) in weights.iter().enumerate() {
            let negative = weight.negate()?;
            off_diagonal[c] = negative.to_f64();
            diagonal[c] = Rational::ONE.checked_add(negative)?.to_f64();
        }
        Ok(Self {
            off_diagonal,
            diagonal,
            width,
        })
    }
}
impl Kernel for Rereference {
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
        let channels = self.diagonal.len();
        if input.values.len() != channels * self.width
            || input.valid.len() != channels
            || output.values.len() != channels * self.width
            || output.valid.len() != channels
        {
            return Err("재기준화의 입출력 배열 크기를 계획과 맞춘다.".into());
        }
        output.inherit(input);
        output.flags &= !INVALID_INPUT;
        output.values.fill(0.0);
        output.valid.fill(1);
        for c in 0..channels {
            for j in 0..channels {
                let coefficient = if c == j {
                    self.diagonal[j]
                } else {
                    self.off_diagonal[j]
                };
                if coefficient == 0.0 {
                    continue;
                }
                output.valid[c] &= input.valid[j];
                for sample in 0..self.width {
                    output.values[c * self.width + sample] +=
                        coefficient * input.values[j * self.width + sample];
                }
            }
            if output.valid[c] == 0 {
                output.values[c * self.width..(c + 1) * self.width].fill(0.0);
                output.flags |= INVALID_INPUT;
            }
        }
        Ok(true)
    }
}
