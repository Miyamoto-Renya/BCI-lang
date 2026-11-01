use bci_extension::{Inputs, Kernel, StateReader, StateWriter, Tick};
use bci_schema::execution::{Frame, INVALID_INPUT, Origin};

pub(crate) struct Mix {
    weights: Vec<f64>,
    channels: usize,
}
impl Mix {
    pub fn new(weights: &[f64], channels: usize) -> Self {
        Self {
            weights: weights.to_vec(),
            channels,
        }
    }
}
impl Kernel for Mix {
    fn checkpoint(&self, _: &mut StateWriter) -> Result<(), String> {
        Ok(())
    }
    fn restore(&mut self, input: &mut StateReader<'_>) -> Result<(), String> {
        input.finish()
    }
    fn process(&mut self, _: Option<&Frame>, _: Tick, _: &mut Frame) -> Result<bool, String> {
        Err("mix에는 process_inputs로 모든 포트를 전달한다.".into())
    }
    fn process_inputs(
        &mut self,
        inputs: Inputs<'_>,
        _: Tick,
        output: &mut Frame,
    ) -> Result<bool, String> {
        if inputs.len() != self.weights.len()
            || output.values.len() != self.channels
            || output.valid.len() != self.channels
        {
            return Err("mix의 입력 수·출력 채널 수를 계약과 맞춘다.".into());
        }
        let first = inputs.get(0).ok_or("mix의 첫 포트를 연결한다.")?;
        for i in 0..inputs.len() {
            let input = inputs.get(i).ok_or("mix 입력 포트를 연결한다.")?;
            if input.values.len() != self.channels
                || input.valid.len() != self.channels
                || !first.same_tick(input)
            {
                return Err(format!(
                    "mix 입력 {i}의 채널·시각·sequence·generation·Gap을 첫 포트와 맞춘다."
                ));
            }
        }
        output.inherit(first);
        output.values.fill(0.0);
        output.valid.fill(1);
        for (i, &weight) in self.weights.iter().enumerate() {
            let input = inputs.get(i).expect("checked input");
            output.flags |= input.flags;
            output.source_first = output.source_first.min(input.source_first);
            output.source_last = output.source_last.max(input.source_last);
            if output.origin != input.origin {
                output.origin = Origin::Mixed;
            }
            for c in 0..self.channels {
                output.valid[c] &= input.valid[c];
                output.values[c] += weight * input.values[c];
            }
        }
        for c in 0..self.channels {
            if output.valid[c] == 0 {
                output.values[c] = 0.0;
                output.flags |= INVALID_INPUT;
            }
        }
        Ok(true)
    }
}
