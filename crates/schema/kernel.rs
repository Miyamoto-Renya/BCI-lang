//! 실행 번들이 공유하는 구체 커널 구성과 예약량 계약.

use crate::{execution::Frame, input::SourceSpec};

/// 기록 sink당 고정 staging 버퍼 크기.
pub const RECORD_BUFFER_BYTES: usize = 65_536;

/// 스키마 검사 후 구체화한 표준 순차 커널 구성.
#[derive(Debug, Clone, PartialEq)]
pub enum KernelSpec {
    /// 준비 단계에서 고정한 정규화·softmax 모델.
    Predict {
        /// 입력 계약·수치·학습 계보.
        model: Box<crate::learning::Model>,
    },
    /// 최대 확률과 고정 minimum으로 Select·Hold를 계산한다.
    Threshold {
        /// 클래스 순서.
        classes: Vec<String>,
        /// 최소 확률.
        minimum: f64,
    },
    /// 합성·CSV·기록 reader가 게시하는 입력 포트.
    Source(Box<SourceSpec>),
    /// 같은 타입·tick의 벡터를 인수 순서로 가중 합성한다.
    Mix {
        /// 입력 포트 순서의 무차원 계수.
        weights: Vec<f64>,
        /// 각 벡터의 채널 수.
        channels: usize,
    },
    /// 채널 선택·재정렬의 고유 입력 index 매핑.
    ChannelMap {
        /// 입력 채널 수.
        input_channels: usize,
        /// 출력 순서의 입력 index.
        indices: Vec<usize>,
        /// 채널당 값 수, Vec는 1이고 Epoch는 표본 수다.
        width: usize,
    },
    /// 전압 기준 변환 A = I − 1wᵀ.
    Rereference {
        /// 입력 채널 순서의 정확한 무차원 계수, 합은 1이다.
        weights: Vec<crate::number::Rational>,
        /// 채널당 값 수.
        width: usize,
    },
    /// registry 계약과 대조한 pointwise·stateful 사용자 연산자.
    Extension {
        /// 정적 번들에 등록한 계약·구현 정체성.
        operator: Box<crate::extension::OperatorSpec>,
        /// 입출력 벡터의 채널 수.
        channels: usize,
    },
    /// 채널별 선형 배율. 값의 모든 column에 적용한다.
    Scale {
        /// 채널별 배율.
        factors: Vec<f64>,
        /// 채널당 값 수.
        width: usize,
    },
    /// 고정 SOS의 transposed direct form II 상태 필터.
    Iir {
        /// 채널 수.
        channels: usize,
        /// 표본률·정규 계수·초기화 계약.
        realization: Box<crate::iir::Realization>,
    },
    /// 고정 인과 polyphase 재표본화.
    Resample {
        /// 채널 수.
        channels: usize,
        /// 격자·계수 계약.
        specification: Box<crate::resample::Resampling>,
    },
    /// 인과 FIR, `h[0]`이 최신 입력에 곱해진다.
    Fir {
        /// 채널 수.
        channels: usize,
        /// 무차원 계수.
        coefficients: Vec<f64>,
    },
    /// 고정 표본 창. start=None이면 FirstValid다.
    Window {
        /// 채널 수.
        channels: usize,
        /// 창 표본 수.
        size: usize,
        /// 시작 index 간격.
        hop: usize,
        /// 고정 시작 index 또는 첫 valid 구간.
        start: Option<u64>,
    },
    /// 직접 DFT의 단측 전력 스펙트럼 밀도.
    Psd {
        /// 채널 수.
        channels: usize,
        /// DFT 표본 수.
        samples: usize,
        /// 내부 표본률 Hz.
        rate: crate::number::Rational,
        /// 창 함수.
        taper: crate::spectral::Taper,
    },
    /// 지정 bin의 PSD 밀도를 적분한 전력.
    BandPower {
        /// 채널 수.
        channels: usize,
        /// 원래 DFT 표본 수.
        samples: usize,
        /// 내부 표본률 Hz.
        rate: crate::number::Rational,
        /// 정확한 주파수 경계.
        band: crate::spectral::Band,
    },
    /// ln((E+floor)/reference) 특징.
    LogPower {
        /// 채널 수.
        channels: usize,
        /// 양의 안정화 전력.
        floor: crate::number::Rational,
        /// 양의 기준 전력.
        reference: crate::number::Rational,
        /// floor·reference의 입력 단위.
        unit: crate::unit::Unit,
    },
    /// N−1로 나눈 표본 공분산.
    Covariance {
        /// 채널 수.
        channels: usize,
        /// 표본 수.
        samples: usize,
    },
    /// shrinkage와 Cholesky guard.
    Regularize {
        /// 채널 수.
        channels: usize,
        /// 0 < lambda <= 1.
        shrinkage: f64,
        /// 양의 입력 단위 공분산 floor.
        floor: f64,
    },
    /// 같은 tick의 현재 포트들을 개별 기록한다.
    Record {
        /// 소스 기준 상대 경로 또는 절대 경로.
        destination: String,
    },
}

impl KernelSpec {
    /// 출력 값 개수와 품질 채널 수. sink는 (0,0)이다.
    pub fn layout(&self) -> Result<(usize, usize), String> {
        let checked = |a: usize, b: usize| {
            a.checked_mul(b)
                .ok_or_else(|| "배열 예약량이 usize 범위를 초과했다.".to_string())
        };
        Ok(match self {
            Self::Predict { model } => (model.classes.len(), model.classes.len()),
            Self::Threshold { .. } => (2, 2),
            Self::Source(source) => source.layout()?,
            Self::ChannelMap { indices, width, .. } => {
                (checked(indices.len(), *width)?, indices.len())
            }
            Self::Rereference { weights, width } => {
                (checked(weights.len(), *width)?, weights.len())
            }
            Self::Mix { channels, .. } => (*channels, *channels),
            Self::Extension { channels, .. } => (*channels, *channels),
            Self::Psd {
                channels, samples, ..
            } => (checked(*channels, samples / 2 + 1)?, *channels),
            Self::BandPower { channels, .. } | Self::LogPower { channels, .. } => {
                (*channels, *channels)
            }
            Self::Resample { channels, .. } => (*channels, *channels),
            Self::Iir { channels, .. } => (*channels, *channels),
            Self::Fir { channels, .. } => (*channels, *channels),
            Self::Scale { factors, width } => (checked(factors.len(), *width)?, factors.len()),
            Self::Window { channels, size, .. } => (checked(*channels, *size)?, *channels),
            Self::Covariance { channels, .. } | Self::Regularize { channels, .. } => {
                (checked(*channels, *channels)?, *channels)
            }
            Self::Record { .. } => (0, 0),
        })
    }

    /// 출력 슬롯의 값·품질·Frame 구조 예약량.
    pub fn buffer_bytes(&self) -> Result<usize, String> {
        let (values, channels) = self.layout()?;
        values
            .checked_mul(8)
            .and_then(|v| v.checked_add(channels))
            .and_then(|v| v.checked_add(std::mem::size_of::<Frame>()))
            .ok_or_else(|| "출력 버퍼 예약량 범위를 초과했다.".into())
    }

    /// 가변 상태·scratch·고정 제어 구조·기록 staging의 상한.
    pub fn state_bytes(&self) -> Result<usize, String> {
        let size = match self {
            Self::Predict { model } => return model.memory(),
            Self::Threshold { classes, .. } => {
                Some(classes.iter().map(String::len).sum::<usize>() + classes.len() * 24)
            }
            Self::Source(source) => source.state_bytes()?.checked_add(self.buffer_bytes()?),
            Self::Resample {
                channels,
                specification,
            } => return specification.memory(*channels),
            Self::Iir {
                channels,
                realization,
            } => return realization.state_bytes(*channels),
            Self::ChannelMap { indices, .. } => {
                indices.len().checked_mul(std::mem::size_of::<usize>())
            }
            Self::Rereference { weights, .. } => weights.len().checked_mul(16),
            Self::Mix { weights, .. } => weights.len().checked_mul(8),
            Self::Extension { operator, channels } => Some(match &operator.state {
                Some(state) => state.memory(*channels)?,
                None => 0,
            }),
            Self::Scale { factors, .. } => factors.len().checked_mul(8),
            Self::Fir {
                channels,
                coefficients,
            } => channels
                .checked_mul(coefficients.len())
                .and_then(|n| n.checked_mul(9))
                .and_then(|n| {
                    coefficients
                        .len()
                        .checked_mul(8)
                        .and_then(|c| n.checked_add(c))
                })
                .and_then(|n| {
                    coefficients
                        .len()
                        .checked_mul(17)
                        .and_then(|c| n.checked_add(c))
                }),
            Self::Window { channels, size, .. } => channels
                .checked_mul(*size)
                .and_then(|n| n.checked_mul(9))
                .and_then(|n| size.checked_mul(18).and_then(|m| n.checked_add(m))),
            Self::Psd { samples, .. } => samples.checked_mul(24),
            Self::BandPower { .. } | Self::LogPower { .. } => Some(0),
            Self::Covariance { channels, .. } => channels.checked_mul(8),
            Self::Regularize { channels, .. } => channels
                .checked_mul(*channels)
                .and_then(|n| n.checked_mul(8)),
            Self::Record { .. } => Some(RECORD_BUFFER_BYTES + 1024),
        };
        size.and_then(|n| n.checked_add(1024))
            .ok_or_else(|| "커널 상태 예약량 범위를 초과했다.".into())
    }

    /// 커널별 저장 payload와 wrapper 제어 상태의 상한.
    pub fn checkpoint_bytes(&self) -> Result<usize, String> {
        match self {
            Self::Extension { operator, channels } if operator.state.is_some() => operator
                .state
                .as_ref()
                .expect("state")
                .checkpoint_limit(*channels)?
                .checked_add(64)
                .ok_or_else(|| "checkpoint 상한 범위를 초과했다.".into()),
            Self::Resample { .. } | Self::Iir { .. } | Self::Fir { .. } | Self::Window { .. } => {
                self.state_bytes()
            }
            _ => Ok(0),
        }
    }
}
