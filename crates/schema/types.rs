//! 소스 위치, 측정 스키마, 스트림 타입과 합성 효과.

use crate::{number::Rational, unit::Unit};
use std::{collections::BTreeSet, fmt};

/// 원본 UTF-8 소스의 반열린 byte 범위.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Span {
    /// 시작 byte offset.
    pub start: usize,
    /// 끝 byte offset.
    pub end: usize,
}

/// 채널의 명목 의미와 수치 단위.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Channel {
    /// 원천 내 채널 식별자.
    pub id: String,
    /// 명목 측정량.
    pub quantity: String,
    /// 수치 단위.
    pub unit: Unit,
    /// 측정 기준 식별자.
    pub reference: String,
}

/// 순서를 유지하는 채널 스키마.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelSchema {
    /// 사람이 읽는 선언 이름.
    pub name: String,
    /// 의미 비교에 사용하는 순서 있는 채널.
    pub channels: Vec<Channel>,
}

/// 값의 저장 표현.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DType {
    /// 32비트 부동소수점.
    F32,
    /// 64비트 부동소수점.
    F64,
    /// 16비트 부호 정수.
    I16,
    /// 32비트 부호 정수.
    I32,
    /// 참·거짓 값.
    Bool,
}

impl fmt::Display for DType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::F32 => "f32",
            Self::F64 => "f64",
            Self::I16 => "i16",
            Self::I32 => "i32",
            Self::Bool => "bool",
        })
    }
}

/// stream의 레코드 값 구조.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueType {
    /// 고정 클래스 순서의 f64 softmax 확률.
    Probability(Vec<String>),
    /// [선택 class index 또는 -1(Hold), 최대 확률]의 f64 판정.
    Decision(Vec<String>),
    /// 채널 순서의 벡터.
    Vector(DType, usize),
    /// 채널·표본 크기와 내부 표본률을 가진 epoch.
    Epoch(DType, usize, usize, Rational),
    /// 단측 PSD: 채널·원래 DFT 표본 수·내부 표본률, bin 수는 N/2+1이다.
    Spectrum(DType, usize, usize, Rational),
    /// 공분산과 양정치 refinement 여부.
    Covariance(DType, usize, bool),
}

impl fmt::Display for ValueType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Probability(classes) => write!(f, "Probability<{classes:?}>"),
            Self::Decision(classes) => write!(f, "Decision<{classes:?}>"),
            Self::Vector(d, c) => write!(f, "Vec<{d},{c}>"),
            Self::Epoch(d, c, n, rate) => write!(f, "Epoch<{d},{c},{n}; grid={rate}Hz>"),
            Self::Spectrum(d, c, n, rate) => write!(f, "Spectrum<{d},{c},{n}; grid={rate}Hz>"),
            Self::Covariance(d, c, spd) => {
                write!(f, "{}<{d},{c}>", if *spd { "SPD" } else { "Cov" })
            }
        }
    }
}

/// stream 레코드의 명목 발생 규칙.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Occurrence {
    /// 기준 Hz 표본률과 초 단위 위상.
    Uniform {
        /// 레코드 발생률.
        rate: Rational,
        /// 기준 시계의 위상.
        phase: Rational,
    },
    /// 발생률이 확정되고 첫 valid 창의 위상을 bind/run에서 결정하는 격자.
    PendingUniform {
        /// 레코드 발생률.
        rate: Rational,
        /// 위상 의무의 명목 식별자.
        phase_key: String,
    },
    /// 레코드별 사건 시간.
    Events,
}

impl fmt::Display for Occurrence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Uniform { rate, phase } => write!(f, "Uniform({rate}Hz, {phase}s)"),
            Self::PendingUniform { rate, phase_key } => {
                write!(f, "Uniform({rate}Hz, Pending({phase_key}))")
            }
            Self::Events => f.write_str("Events"),
        }
    }
}

/// 포트 연결에서 검사하는 전체 스트림 타입.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamType {
    /// 레코드의 값 구조.
    pub value: ValueType,
    /// 채널 의미와 순서.
    pub schema: ChannelSchema,
    /// 명목 clock domain과 origin의 식별자.
    pub clock: String,
    /// 발생률과 위상.
    pub occurrence: Occurrence,
    /// 품질 어휘 식별자.
    pub quality: String,
}

impl StreamType {
    /// 일치에 실패한 첫 계약과 그 진단 코드를 반환한다.
    pub fn mismatch(&self, expected: &Self) -> Option<(&'static str, String)> {
        let actual_units: Vec<_> = self.schema.channels.iter().map(|c| &c.unit).collect();
        let expected_units: Vec<_> = expected.schema.channels.iter().map(|c| &c.unit).collect();
        if actual_units.len() == expected_units.len() && actual_units != expected_units {
            return Some((
                "E0203",
                format!("단위 불일치: 입력 {self}; 요구 {expected}. convert_unit을 연결한다."),
            ));
        }
        if self.schema.channels != expected.schema.channels {
            return Some((
                "E0204",
                format!(
                    "채널·측정량·기준 불일치: 입력 {self}; 요구 {expected}. 채널 스키마를 맞춘다."
                ),
            ));
        }
        if self.value != expected.value {
            return Some((
                "E0201",
                format!(
                    "값 타입 불일치: 입력 {}; 요구 {}.",
                    self.value, expected.value
                ),
            ));
        }
        if self.clock != expected.clock {
            return Some((
                "E0401",
                format!(
                    "시계 불일치: 입력 {}; 요구 {}. time.align을 연결한다.",
                    self.clock, expected.clock
                ),
            ));
        }
        if self.occurrence != expected.occurrence {
            return Some((
                "E0205",
                format!(
                    "발생 규칙 불일치: 입력 {}; 요구 {}. 표본률과 위상을 맞춘다.",
                    self.occurrence, expected.occurrence
                ),
            ));
        }
        if self.quality != expected.quality {
            return Some((
                "E0206",
                format!(
                    "품질 어휘 불일치: 입력 {}; 요구 {}.",
                    self.quality, expected.quality
                ),
            ));
        }
        None
    }
}

impl fmt::Display for StreamType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let channels = self
            .schema
            .channels
            .iter()
            .map(|c| format!("{}:{}[{}]@{}", c.id, c.quantity, c.unit, c.reference))
            .collect::<Vec<_>>()
            .join(",");
        write!(
            f,
            "Stream<{}, [{}], {}, {}, {}>",
            self.value, channels, self.clock, self.occurrence, self.quality
        )
    }
}

/// 입력 종료 의존성을 구분하는 효과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// 가용 입력에서 진행하는 모드.
    Live,
    /// 기록 범위와 종료를 소비하는 모드.
    Record,
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Live => "Live",
            Self::Record => "Record",
        })
    }
}

/// 그래프에서 합성하는 모드·기능·외부 출력 효과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effect {
    /// 처리 모드.
    pub mode: Mode,
    /// 필요한 장치 기능.
    pub capabilities: BTreeSet<String>,
    /// 출력 자원 식별자.
    pub writes: BTreeSet<String>,
}

impl Effect {
    /// 부수 자원이 없는 효과를 만든다.
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            capabilities: BTreeSet::new(),
            writes: BTreeSet::new(),
        }
    }
    /// 연결 경로의 효과를 합친다.
    pub fn combine(&mut self, rhs: &Self) {
        if rhs.mode == Mode::Record {
            self.mode = Mode::Record;
        }
        self.capabilities.extend(rhs.capabilities.iter().cloned());
        self.writes.extend(rhs.writes.iter().cloned());
    }
}
