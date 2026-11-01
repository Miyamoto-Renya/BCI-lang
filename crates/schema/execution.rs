//! 유한 실행의 데이터, 품질, 상태와 관측 보고서.

use crate::{number::Rational, types::Span};

/// FIR 초기 상태를 사용하는 표본.
pub const WARMUP: u8 = 1;
/// 입력 품질에서 전파한 무효 표본.
pub const INVALID_INPUT: u8 = 2;
/// 누락된 표본 뒤의 첫 레코드.
pub const GAP: u8 = 4;

/// 원천 값의 생성 방식.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Origin {
    /// 계산으로 생성한 값.
    Synthetic,
    /// 측정 CSV에서 읽은 값.
    Measurement,
    /// 합성·측정 등 서로 다른 origin의 합성 결과.
    Mixed,
}
impl Origin {
    /// 기여한 원천 방식의 합집합을 요약한다.
    pub fn combine(self, other: Self) -> Self {
        if self == other { self } else { Self::Mixed }
    }
    /// 상태 codec의 origin tag를 확인한다.
    pub fn from_code(code: u8) -> Result<Self, String> {
        match code {
            0 => Ok(Self::Synthetic),
            1 => Ok(Self::Measurement),
            2 => Ok(Self::Mixed),
            _ => Err("origin tag는 0..2 범위를 요구한다.".into()),
        }
    }
}

/// 누락된 원천 표본의 닫힌 index 범위.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gap {
    /// 첫 누락 index.
    pub first: u64,
    /// 마지막 누락 index.
    pub last: u64,
}

/// 노드당 한 개를 예약해 재사용하는 채널 우선 데이터 슬롯.
#[derive(Debug)]
pub struct Frame {
    /// Vec: C, Epoch: C×N, 공분산: C×C의 row-major 값.
    pub values: Vec<f64>,
    /// 채널별 유효성, 0 또는 1.
    pub valid: Vec<u8>,
    /// 포트의 anchor index. 재표본화는 출력 격자 index를 사용한다.
    pub sequence: u64,
    /// Gap 이후 상태를 구분하는 세대.
    pub generation: u64,
    /// 원천의 생성 방식.
    pub origin: Origin,
    /// 이 레코드 바로 앞의 원천 누락 범위.
    pub gap: Option<Gap>,
    /// 출력 anchor의 정확한 clock-relative 초.
    pub time: Rational,
    /// 출력에 기여한 첫 원천 표본.
    pub source_first: u64,
    /// 출력에 기여한 마지막 원천 표본.
    pub source_last: u64,
    /// WARMUP·INVALID_INPUT 비트 집합.
    pub flags: u8,
}

impl Frame {
    /// 같은 논리 tick·generation·Gap 경계인지 확인한다.
    pub fn same_tick(&self, other: &Self) -> bool {
        self.time == other.time
            && self.sequence == other.sequence
            && self.generation == other.generation
            && self.gap == other.gap
    }
    /// 값·품질 버퍼를 준비 단계에 예약한다.
    pub fn allocate(values: usize, channels: usize) -> Result<Self, String> {
        let mut data = Vec::new();
        data.try_reserve_exact(values).map_err(|e| e.to_string())?;
        data.resize(values, 0.0);
        let mut valid = Vec::new();
        valid
            .try_reserve_exact(channels)
            .map_err(|e| e.to_string())?;
        valid.resize(channels, 0);
        Ok(Self {
            values: data,
            valid,
            sequence: 0,
            generation: 0,
            origin: Origin::Synthetic,
            gap: None,
            time: Rational::ZERO,
            source_first: 0,
            source_last: 0,
            flags: 0,
        })
    }

    /// 같은 layout의 예약 슬롯에 전체 레코드를 복사한다.
    pub fn copy_from(&mut self, input: &Self) {
        self.inherit(input);
        self.values.copy_from_slice(&input.values);
    }

    /// 같은 사건의 시간·원천 범위·품질을 복사한다.
    pub fn inherit(&mut self, input: &Self) {
        self.inherit_metadata(input);
        self.valid.copy_from_slice(&input.valid);
    }

    /// 채널 수가 바뀌는 연산에 시간·원천 범위·origin·전체 flags를 전달한다.
    pub fn inherit_metadata(&mut self, input: &Self) {
        self.sequence = input.sequence;
        self.generation = input.generation;
        self.origin = input.origin;
        self.gap = input.gap;
        self.time = input.time;
        self.source_first = input.source_first;
        self.source_last = input.source_last;
        self.flags = input.flags;
    }
}

/// 세션의 순차 상태 전이.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// 실행 요청 수락.
    Created,
    /// 검증·메모리·기록 준비.
    Preparing,
    /// 실행 자원 준비 완료.
    Ready,
    /// 원천 표본 처리.
    Running,
    /// 최초 실패 수락.
    Faulted,
    /// 표본 경계 종료와 기록 flush.
    Draining,
    /// 기록 commit과 자원 해제 완료.
    Closed,
}

/// 한 노드의 실행 관측.
#[derive(Debug, Clone, Default)]
pub struct NodeStats {
    /// 출력이 관측된 generation 수.
    pub generations: u64,
    /// 마지막 출력 generation.
    pub last_generation: Option<u64>,
    /// 마지막 generation의 첫 anchor.
    pub generation_first_time: Option<Rational>,
    /// 출력 또는 기록한 레코드 수.
    pub records: u64,
    /// 전체 채널이 유효한 레코드 수.
    pub valid_records: u64,
    /// 첫 레코드의 anchor 초.
    pub first_time: Option<Rational>,
    /// 마지막 레코드의 anchor 초.
    pub last_time: Option<Rational>,
    /// 첫 출력의 원천 anchor index.
    pub first_sequence: Option<u64>,
    /// 마지막 출력의 원천 anchor index.
    pub last_sequence: Option<u64>,
}

impl NodeStats {
    /// 게시하거나 기록한 데이터에서 실제 카운터를 갱신한다.
    pub fn observe(&mut self, frame: &Frame) {
        if self.last_generation != Some(frame.generation) {
            self.generations += 1;
            self.last_generation = Some(frame.generation);
            self.generation_first_time = Some(frame.time);
        }
        self.records += 1;
        self.valid_records += u64::from(frame.valid.iter().all(|&v| v == 1));
        self.first_time.get_or_insert(frame.time);
        self.first_sequence.get_or_insert(frame.sequence);
        self.last_time = Some(frame.time);
        self.last_sequence = Some(frame.sequence);
    }
}

/// 실행 중 최초로 확인한 실패와 소스 위치.
#[derive(Debug, Clone)]
pub struct RunError {
    /// E08xx 계획·자원 또는 E09xx 실행·저장 코드.
    pub code: &'static str,
    /// 실패 노드. 세션 공통 오류는 None.
    pub node: Option<usize>,
    /// 원본 소스 범위.
    pub span: Span,
    /// 원인과 다음 수정 행동.
    pub message: String,
}

/// 실행 종료 시 관측값과 최종 결과.
#[derive(Debug, Clone)]
pub struct RunReport {
    /// 종료 경계에서 합류를 기다리는 간선 레코드 수.
    pub pending_records: usize,
    /// 이번 실행을 시작한 입력 묶음 위치.
    pub input_offset: u64,
    /// 실행한 모듈·run 이름.
    pub entry: String,
    /// 처리 요청 입력 묶음 수.
    pub requested_samples: Option<u64>,
    /// 정상 EOF 수락 여부.
    pub end_of_input: bool,
    /// 입력 정책에 따라 수락한 Gap 수.
    pub gaps: u64,
    /// 입력 정책에 따라 집계한 원천 누락 index 수.
    pub missing_samples: u64,
    /// 실행 일정이 처리한 입력 묶음 수.
    pub completed_samples: u64,
    /// 데이터 버퍼·커널 상태·기록 staging 예약량.
    pub reserved_bytes: usize,
    /// 처리 루프의 실제 경과 초.
    pub elapsed_seconds: f64,
    /// 외부 stop 요청을 표본 경계에서 수락했는지 여부.
    pub stopped: bool,
    /// 수행한 상태 전이.
    pub states: Vec<SessionState>,
    /// 노드 순서의 출력 관측값.
    pub nodes: Vec<NodeStats>,
    /// 실패 원인.
    pub failure: Option<RunError>,
}
