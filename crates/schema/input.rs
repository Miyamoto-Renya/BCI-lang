//! 합성·파일·장치 입력이 공유하는 읽기, 종료와 시간 격자 계약.

use crate::{
    execution::Frame,
    number::Rational,
    types::{Occurrence, StreamType, ValueType},
};
use std::path::PathBuf;

/// 누락된 격자 표본의 처리.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GapPolicy {
    /// 새 generation에서 상태를 초기화한다.
    Reset,
    /// 누락 범위와 함께 실행을 중단한다.
    Fault,
}

/// CSV 수치 값의 정의역 위반 처리.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidPolicy {
    /// 0 값·invalid mask로 게시한다.
    Mark,
    /// 파일·행·채널을 보고하고 중단한다.
    Fault,
}

/// 원천의 실제 입력 방식.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceKind {
    /// 표준 합성 신호 공식.
    Synthetic,
    /// 시간·채널·단위를 가진 CSV.
    Csv {
        /// 입력 파일.
        path: PathBuf,
        /// 누락 정책.
        gap: GapPolicy,
        /// 무효 값 정책.
        invalid: InvalidPolicy,
    },
    /// BCi JSON Lines의 포트 하나.
    Replay {
        /// 입력 JSON Lines 파일.
        path: PathBuf,
        /// 기록의 포트 ID.
        port: usize,
        /// 누락 정책.
        gap: GapPolicy,
    },
}

/// 계획과 reader가 대조하는 원천 계약.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSpec {
    /// 입력 위치와 정책.
    pub kind: SourceKind,
    /// 입력 포트의 전체 의미 타입.
    pub stream: StreamType,
    /// 파일 한 레코드의 최대 UTF-8 bytes. 합성 입력은 0.
    pub row_bytes: usize,
}

impl SourceSpec {
    /// 값 배열 크기와 채널 수.
    pub fn layout(&self) -> Result<(usize, usize), String> {
        match self.stream.value {
            ValueType::Probability(ref classes) => Ok((classes.len(), classes.len())),
            ValueType::Decision(_) => Ok((2, 2)),
            ValueType::Vector(_, c) => Ok((c, c)),
            ValueType::Spectrum(_, c, n, _) => c
                .checked_mul(n / 2 + 1)
                .map(|v| (v, c))
                .ok_or_else(|| "원천 PSD 배열 크기 범위를 초과했다.".into()),
            ValueType::Epoch(_, c, n, _) => c
                .checked_mul(n)
                .map(|v| (v, c))
                .ok_or_else(|| "원천 배열 크기 범위를 초과했다.".into()),
            ValueType::Covariance(_, c, _) => c
                .checked_mul(c)
                .map(|v| (v, c))
                .ok_or_else(|| "원천 배열 크기 범위를 초과했다.".into()),
        }
    }
    /// reader·행 문자열·JSON 파싱 임시 공간의 예약 상한.
    pub fn state_bytes(&self) -> Result<usize, String> {
        if self.kind == SourceKind::Synthetic {
            return Ok(0);
        }
        if !(4096..=1_048_576).contains(&self.row_bytes) {
            return Err("입력 행 상한은 4096..1048576 bytes를 요구한다.".into());
        }
        self.row_bytes
            .checked_mul(32)
            .and_then(|n| n.checked_add(65_536))
            .ok_or_else(|| "입력 버퍼 예약량 범위를 초과했다.".into())
    }
}

/// f64 값·품질과 시간 필드를 직렬화하는 한 행의 상한.
pub fn row_bound(values: usize, channels: usize) -> Result<usize, String> {
    values
        .checked_mul(64)
        .and_then(|n| channels.checked_mul(16).and_then(|c| n.checked_add(c)))
        .and_then(|n| n.checked_add(4096))
        .filter(|&n| n <= 1_048_576)
        .ok_or_else(|| {
            "파일 포트의 행 예약량은 1 MiB 이하여야 한다. 채널·창 크기를 조정한다.".into()
        })
}

/// 입력 위치를 포함한 파일·시간·품질 진단.
#[derive(Debug, Clone)]
pub struct InputError {
    /// I/O는 E0902, 형식·시간·품질 계약은 E0904.
    pub code: &'static str,
    /// 파일·행과 위반한 조건.
    pub message: String,
}
impl InputError {
    /// 읽기 실패를 구성한다.
    pub fn io(message: impl Into<String>) -> Self {
        Self {
            code: "E0902",
            message: message.into(),
        }
    }
    /// 입력 계약 위반을 구성한다.
    pub fn contract(message: impl Into<String>) -> Self {
        Self {
            code: "E0904",
            message: message.into(),
        }
    }
}

/// prepare에서 연 입력. false는 정상 EOF를 뜻한다.
pub trait InputSource {
    /// 다음 레코드를 예약된 출력 슬롯에 읽는다.
    fn read(&mut self, output: &mut Frame) -> Result<bool, InputError>;
}

/// CLI·장치 bundle이 실제 reader를 연결하는 경계.
pub trait SourceFactory {
    /// 헤더·입력 타입을 검사하고 읽기 상태를 준비한다.
    fn open(&self, specification: &SourceSpec) -> Result<Box<dyn InputSource>, InputError>;
}

/// 현재 generation의 격자와 전체 시간 단조성을 확인한다.
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct GridCursor {
    /// 마지막 generation.
    pub generation: Option<u64>,
    /// generation의 첫 시각.
    pub phase: Option<Rational>,
    /// 마지막 수락 시각.
    pub last_time: Option<Rational>,
    /// 마지막 격자 index.
    pub last_index: Option<u64>,
}
impl GridCursor {
    /// checkpoint의 격자 필드와 발생 규칙을 대조한다.
    pub fn validate(&self, occurrence: &Occurrence) -> Result<(), String> {
        let fields = [
            self.generation.is_some(),
            self.phase.is_some(),
            self.last_time.is_some(),
            self.last_index.is_some(),
        ];
        if fields.iter().all(|v| !v) {
            return Ok(());
        }
        if !fields.iter().all(|v| *v) {
            return Err("격자 checkpoint 필드를 함께 지정한다.".into());
        }
        let (rate, phase) = match occurrence {
            Occurrence::Uniform { rate, phase } => (*rate, *phase),
            Occurrence::PendingUniform { rate, .. } => (*rate, self.phase.expect("phase")),
            _ => return Err("복원에는 Uniform 격자를 요구한다.".into()),
        };
        if grid_index(self.last_time.expect("time"), rate, phase)?
            != self.last_index.expect("index")
            || self
                .phase
                .expect("phase")
                .compare(self.last_time.expect("time"))?
                .is_gt()
        {
            return Err("checkpoint 시각·위상·index를 발생 격자와 맞춘다.".into());
        }
        Ok(())
    }
    /// 격자 index를 검사한다. generation 전환은 새로운 Pending 위상을 설정한다.
    pub fn check(&mut self, occurrence: &Occurrence, frame: &Frame) -> Result<(), String> {
        if self
            .last_time
            .is_some_and(|t| frame.time.compare(t).is_ok_and(|v| !v.is_gt()))
        {
            return Err("시각은 이전 레코드보다 커야 한다.".into());
        }
        if let Some(last) = self.last_time {
            frame.time.compare(last)?;
        }
        if self.generation.is_some_and(|g| frame.generation < g) {
            return Err("generation은 이전 값 이상이어야 한다. 기록 순서를 확인한다.".into());
        }
        let changed = self.generation != Some(frame.generation);
        if changed {
            self.phase = Some(frame.time);
            self.last_index = None;
        }
        let (rate, phase) = match occurrence {
            Occurrence::Uniform { rate, phase } => (*rate, *phase),
            Occurrence::PendingUniform { rate, .. } => (*rate, self.phase.unwrap_or(frame.time)),
            Occurrence::Events => return Err("파일 실행은 Uniform 시간 격자를 요구한다.".into()),
        };
        let index = grid_index(frame.time, rate, phase)?;
        if let Some(previous) = self.last_index
            && previous.checked_add(1) != Some(index)
        {
            return Err(
                "동일 generation의 레코드는 연속 격자를 요구한다. 누락 경계에 Gap 정책을 적용한다."
                    .into(),
            );
        }
        self.generation = Some(frame.generation);
        self.last_time = Some(frame.time);
        self.last_index = Some(index);
        Ok(())
    }
}

/// 정확한 시각을 0 이상 i64::MAX 이하의 격자 index로 변환한다.
pub fn grid_index(time: Rational, rate: Rational, phase: Rational) -> Result<u64, String> {
    let index = time
        .checked_add(phase.negate()?)?
        .multiply(rate)?
        .natural()?;
    u64::try_from(index)
        .ok()
        .filter(|&n| n <= i64::MAX as u64)
        .ok_or_else(|| "시간 격자 index는 0..i64::MAX 범위를 요구한다.".into())
}

/// 십진수 또는 기록의 정수 분수 표기를 읽는다.
pub fn exact_time(text: &str) -> Result<Rational, String> {
    if let Some((a, b)) = text.split_once('/') {
        Rational::new(
            a.parse().map_err(|_| "분자를 정수로 작성한다.")?,
            b.parse().map_err(|_| "분모를 정수로 작성한다.")?,
        )
    } else {
        text.parse()
    }
}
