//! 표준 신호 처리·특징·결정 연산자의 구현과 참조 커널을 소유한다.
//!
//! 커널은 bci-extension의 SDK를 사용하고 선언 계약은 bci-schema로 표현한다.
//! 기본 kernel 집합의 선택과 runtime 등록은 조립 계층이 수행한다.

pub mod evaluation;
mod iir;
pub mod learning;
mod linear;
mod matrix;
mod mix;
mod prediction;
mod resample;
mod source;
mod spatial;
mod spectral;
mod state;
mod window;
pub use source::SyntheticSources;

use bci_extension::{Kernel, KernelFactory};
use bci_schema::kernel::KernelSpec;

/// f64 합성 신호·선형 처리·창·공분산의 표준 실행 번들.
pub struct StandardKernels;

impl KernelFactory for StandardKernels {
    fn create(&self, specification: &KernelSpec) -> Result<Box<dyn Kernel>, String> {
        match specification {
            KernelSpec::Predict { model } => {
                model.validate()?;
                Ok(Box::new(prediction::Predict {
                    model: *model.clone(),
                }))
            }
            KernelSpec::Threshold { minimum, .. } => {
                Ok(Box::new(prediction::Threshold { minimum: *minimum }))
            }
            KernelSpec::Psd {
                channels,
                samples,
                rate,
                taper,
            } => Ok(Box::new(spectral::Psd::new(
                *channels, *samples, *rate, *taper,
            )?)),
            KernelSpec::BandPower {
                channels,
                samples,
                rate,
                band,
            } => Ok(Box::new(spectral::BandPower::new(
                *channels, *samples, *rate, band,
            )?)),
            KernelSpec::LogPower {
                floor, reference, ..
            } => Ok(Box::new(spectral::LogPower::new(*floor, *reference))),
            KernelSpec::ChannelMap {
                input_channels,
                indices,
                width,
            } => Ok(Box::new(spatial::ChannelMap::new(
                *input_channels,
                indices,
                *width,
            ))),
            KernelSpec::Rereference { weights, width } => {
                Ok(Box::new(spatial::Rereference::new(weights, *width)?))
            }
            KernelSpec::Mix { weights, channels } => {
                Ok(Box::new(mix::Mix::new(weights, *channels)))
            }
            KernelSpec::Source(_) => Err("입력 원천은 SourceFactory 경계로 생성한다.".into()),
            KernelSpec::Extension { .. } => {
                Err("사용자 커널은 Extensions factory에 연결한다.".into())
            }
            KernelSpec::Scale { factors, width } => {
                Ok(Box::new(linear::Scale::new(factors, *width)))
            }
            KernelSpec::Resample {
                channels,
                specification,
            } => Ok(Box::new(resample::Resample::new(*channels, specification)?)),
            KernelSpec::Iir {
                channels,
                realization,
            } => Ok(Box::new(iir::Iir::new(*channels, realization)?)),
            KernelSpec::Fir {
                channels,
                coefficients,
            } => Ok(Box::new(linear::Fir::new(*channels, coefficients)?)),
            KernelSpec::Window {
                channels,
                size,
                hop,
                start,
            } => Ok(Box::new(window::Window::new(
                *channels, *size, *hop, *start,
            )?)),
            KernelSpec::Covariance { channels, samples } => {
                Ok(Box::new(matrix::Covariance::new(*channels, *samples)?))
            }
            KernelSpec::Regularize {
                channels,
                shrinkage,
                floor,
            } => Ok(Box::new(matrix::Regularize::new(
                *channels, *shrinkage, *floor,
            )?)),
            KernelSpec::Record { .. } => Err("기록 sink는 Recorder 경계로 생성한다.".into()),
        }
    }
}

fn reserved<T: Clone>(size: usize, value: T) -> Result<Vec<T>, String> {
    let mut result = Vec::new();
    result.try_reserve_exact(size).map_err(|e| e.to_string())?;
    result.resize(size, value);
    Ok(result)
}

fn input(
    input: Option<&bci_schema::execution::Frame>,
) -> Result<&bci_schema::execution::Frame, String> {
    input.ok_or_else(|| "수치 커널의 입력 슬롯을 연결한다.".into())
}
