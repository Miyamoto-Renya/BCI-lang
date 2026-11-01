use bci_extension::{Kernel, StateReader, StateWriter, Tick};
use bci_schema::{
    execution::{Frame, INVALID_INPUT},
    learning::{Model, probability},
};

pub(crate) struct Predict {
    pub model: Model,
}
impl Kernel for Predict {
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
        output.inherit_metadata(input);
        if input.valid.contains(&0) {
            output.values.fill(0.0);
            output.valid.fill(0);
            output.flags |= INVALID_INPUT;
        } else {
            self.model.predict(&input.values, &mut output.values)?;
            output.valid.fill(1);
        }
        Ok(true)
    }
}
pub(crate) struct Threshold {
    pub minimum: f64,
}
impl Kernel for Threshold {
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
        output.inherit_metadata(input);
        if input.valid.contains(&0) {
            output.values.copy_from_slice(&[-1.0, 0.0]);
            output.valid.fill(0);
            output.flags |= INVALID_INPUT;
        } else {
            probability(&input.values)?;
            let selected = crate::learning::winner(&input.values);
            let confidence = input.values[selected];
            output.values[0] = if confidence >= self.minimum {
                selected as f64
            } else {
                -1.0
            };
            output.values[1] = confidence;
            output.valid.fill(1);
        }
        Ok(true)
    }
}
