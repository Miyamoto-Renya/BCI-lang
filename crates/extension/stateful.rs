use crate::{Kernel, OperatorSpec, StateReader, StateWriter, Tick};
use bci_schema::execution::{Frame, Origin};

/// 상태 초기화의 의미 경계.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetCause {
    /// 입력 generation 또는 연속 index 변경.
    Gap,
    /// 호스트의 명시적 초기화 요청.
    Explicit,
}

/// 한 호출 동안 사용하는 동일 채널 벡터와 품질.
pub struct SampleView<'a> {
    /// 유한 입력 값.
    pub values: &'a [f64],
    /// 채널별 유효성. 무효 채널의 누적 갱신은 건너뛴다.
    pub valid: &'a [u8],
}

/// 단일 입력·동일 격자 상태 연산자의 실행·복원 계약.
pub trait StatefulOperator: Sized + 'static {
    /// 고정 채널 수에 맞춰 상태를 예약한다.
    fn prepare(channels: usize) -> Result<Self, String>;
    /// 객체와 소유 버퍼 capacity의 전체 bytes.
    fn state_bytes(&self) -> usize;
    /// 입력 한 레코드를 처리한다. 출력·scratch 크기는 준비 시 고정한다.
    fn step(
        &mut self,
        input: SampleView<'_>,
        output: &mut [f64],
        scratch: &mut [u8],
    ) -> Result<(), String>;
    /// 경계에서 누적 상태를 초기화한다.
    fn reset(&mut self, cause: ResetCause) -> Result<(), String>;
    /// 판본이 고정된 상태 payload를 쓴다.
    fn checkpoint(&self, output: &mut StateWriter) -> Result<(), String>;
    /// 새 객체에 payload를 읽고 상태 불변식을 검사한다.
    fn restore(&mut self, input: &mut StateReader<'_>) -> Result<(), String>;
}

/// 상태 타입의 factory·선언·수치 참조 연결.
pub type StateFactory = fn(&OperatorSpec, usize) -> Result<Box<dyn Kernel>, String>;

/// 상태 타입의 factory·선언·수치 참조 연결.
pub struct StatefulRegistration {
    /// compiler와 runtime이 공유하는 선언.
    pub spec: OperatorSpec,
    /// 구체 채널 수로 wrapper를 준비한다.
    pub create: StateFactory,
    /// 순서 있는 단일 채널 exact-bit corpus와 저장·복원 검사를 실행한다.
    pub verify: fn(&OperatorSpec) -> Result<usize, String>,
}
impl StatefulRegistration {
    /// 매크로가 고른 상태 타입의 wrapper를 생성한다.
    pub fn create<T: StatefulOperator>(
        spec: &OperatorSpec,
        channels: usize,
    ) -> Result<Box<dyn Kernel>, String> {
        let contract = spec
            .state
            .as_ref()
            .ok_or("stateful 자원 계약을 연결한다.")?;
        let bound = contract.memory(channels)?;
        let scratch_size = contract
            .scratch_bytes
            .checked_mul(channels)
            .ok_or("scratch 범위를 초과했다.")?;
        let state = T::prepare(channels)?;
        if state.state_bytes() > bound - scratch_size {
            return Err("사용자 상태가 예약량을 초과했다.".into());
        }
        let mut scratch = Vec::new();
        scratch
            .try_reserve_exact(scratch_size)
            .map_err(|e| e.to_string())?;
        scratch.resize(scratch_size, 0);
        Ok(Box::new(Stateful {
            state,
            scratch,
            bound: bound - scratch_size,
            payload_limit: contract.checkpoint_limit(channels)?,
            channels,
            previous: None,
            first: None,
            origin: None,
            faulted: false,
        }))
    }

    /// 작성자가 제공한 참조 순서와 중간 저장·복원·초기화 결과를 비교한다.
    pub fn verify<T: StatefulOperator>(
        spec: &OperatorSpec,
        reference: &[(u64, u64)],
    ) -> Result<usize, String> {
        if reference.is_empty() || reference.len() > 4096 {
            return Err("상태 reference는 1..4096쌍을 요구한다.".into());
        }
        let mut state = Self::create::<T>(spec, 1)?;
        let mut input = Frame::allocate(1, 1)?;
        input.valid.fill(1);
        let mut output = Frame::allocate(1, 1)?;
        for (index, &(value, expected)) in reference.iter().enumerate() {
            input.values[0] = f64::from_bits(value);
            input.origin = if index == 0 {
                Origin::Synthetic
            } else {
                Origin::Measurement
            };
            input.sequence = index as u64;
            state.process(
                Some(&input),
                Tick {
                    index: index as u64,
                },
                &mut output,
            )?;
            if output.values[0].to_bits() != expected {
                return Err(format!(
                    "{}: 상태 reference {index} 결과가 기대 bit와 다르다.",
                    spec.name
                ));
            }
            let expected_origin = if index == 0 {
                Origin::Synthetic
            } else {
                Origin::Mixed
            };
            if output.origin != expected_origin {
                return Err("상태 reference의 누적 origin을 기여 입력과 맞춘다.".into());
            }
            let mut writer =
                StateWriter::new(spec.state.as_ref().expect("state").checkpoint_limit(1)? + 64)?;
            state.checkpoint(&mut writer)?;
            let bytes = writer.finish();
            let mut restored = Self::create::<T>(spec, 1)?;
            let mut reader = StateReader::new(&bytes);
            restored.restore(&mut reader)?;
            reader.finish()?;
            state = restored;
        }
        input.generation = 1;
        input.origin = Origin::Measurement;
        input.sequence += 1;
        input.values[0] = f64::from_bits(reference[0].0);
        state.process(
            Some(&input),
            Tick {
                index: input.sequence,
            },
            &mut output,
        )?;
        if output.values[0].to_bits() != reference[0].1 || output.origin != Origin::Measurement {
            return Err("Gap 초기화 뒤 첫 reference 결과를 맞춘다.".into());
        }
        Ok(reference.len())
    }
}

struct Stateful<T> {
    state: T,
    scratch: Vec<u8>,
    bound: usize,
    payload_limit: usize,
    channels: usize,
    previous: Option<(u64, u64)>,
    first: Option<u64>,
    origin: Option<Origin>,
    faulted: bool,
}
impl<T: StatefulOperator> Kernel for Stateful<T> {
    fn process(
        &mut self,
        input: Option<&Frame>,
        _: Tick,
        output: &mut Frame,
    ) -> Result<bool, String> {
        if self.faulted {
            return Err("Fault 뒤에는 새 객체를 준비하거나 checkpoint를 복원한다.".into());
        }
        self.faulted = true;
        let input = input.ok_or("stateful 입력을 연결한다.")?;
        if [
            input.values.len(),
            input.valid.len(),
            output.values.len(),
            output.valid.len(),
        ]
        .iter()
        .any(|&n| n != self.channels)
        {
            return Err("stateful 배열 크기를 계약 채널 수와 맞춘다.".into());
        }
        if input.gap.is_some()
            || self.previous.is_some_and(|(g, n)| {
                g != input.generation || n.checked_add(1) != Some(input.sequence)
            })
        {
            self.state.reset(ResetCause::Gap)?;
            self.first = None;
            self.origin = None;
            self.scratch.fill(0);
        }
        let first = self
            .first
            .map_or(input.source_first, |n| n.min(input.source_first));
        output.inherit(input);
        output.values.copy_from_slice(&input.values);
        self.scratch.fill(0);
        self.state.step(
            SampleView {
                values: &input.values,
                valid: &input.valid,
            },
            &mut output.values,
            &mut self.scratch,
        )?;
        if self.state.state_bytes() > self.bound {
            return Err("사용자 상태가 예약량을 초과했다.".into());
        }
        for (c, &valid) in input.valid.iter().enumerate() {
            if valid == 0 {
                output.values[c] = input.values[c];
            }
        }
        if output.values.iter().any(|v| !v.is_finite()) {
            return Err("상태 연산자 출력은 유한 f64를 요구한다.".into());
        }
        output.source_first = first;
        output.origin = self
            .origin
            .map_or(input.origin, |o| o.combine(input.origin));
        self.origin = Some(output.origin);
        self.previous = Some((input.generation, input.sequence));
        self.first = Some(first);
        self.faulted = false;
        Ok(true)
    }
    fn checkpoint(&self, output: &mut StateWriter) -> Result<(), String> {
        if self.faulted {
            return Err("완료한 호출 경계의 상태를 저장한다.".into());
        }
        let mut payload = StateWriter::new(self.payload_limit)?;
        self.state.checkpoint(&mut payload)?;
        output.optional(self.previous.map(|p| p.0))?;
        output.optional(self.previous.map(|p| p.1))?;
        output.optional(self.first)?;
        output.bytes(&[self.origin.map_or(255, |o| o as u8)])?;
        output.bytes(&payload.finish())
    }
    fn restore(&mut self, input: &mut StateReader<'_>) -> Result<(), String> {
        self.faulted = true;
        let generation = input.optional()?;
        let sequence = input.optional()?;
        let first = input.optional()?;
        let code = input.bytes(1)?[0];
        let origin = if code == 255 {
            None
        } else {
            Some(Origin::from_code(code)?)
        };
        if generation.is_some() != sequence.is_some()
            || first.is_some() != sequence.is_some()
            || origin.is_some() != sequence.is_some()
            || first.zip(sequence).is_some_and(|(f, n)| f > n)
        {
            return Err("wrapper의 generation·원천 범위 상태를 맞춘다.".into());
        }
        self.state.restore(input)?;
        input.finish()?;
        if self.state.state_bytes() > self.bound {
            return Err("복원 상태가 예약량을 초과했다.".into());
        }
        self.previous = generation.zip(sequence);
        self.first = first;
        self.origin = origin;
        self.scratch.fill(0);
        self.faulted = false;
        Ok(())
    }
}
