//! 사용자 연산자의 context, buffer view, 상태 codec과 등록 API를 소유한다.
//!
//! 연산자는 이 SDK와 bci-schema의 계약에 연결한다.
//! 실행기는 SDK의 인터페이스로 호출 문맥과 예약 자원을 제공한다.

use bci_schema::{execution::Frame, kernel::KernelSpec};

mod codec;
mod pointwise;
mod stateful;
pub use bci_extension_macros::operator;
pub use bci_schema::extension::{OperatorSpec, POINTWISE_PROFILE, STATEFUL_PROFILE, StateContract};
pub use codec::{StateReader, StateWriter};
pub use pointwise::{ExtensionKernels, Extensions, PointwiseRegistration};
pub use stateful::{ResetCause, SampleView, StateFactory, StatefulOperator, StatefulRegistration};

/// 순차 실행기가 전달하는 원천 tick.
#[derive(Debug, Clone, Copy)]
pub struct Tick {
    /// 원천 표본 index.
    pub index: u64,
}

/// 이번 호출에 연결된 입력 포트만 읽는 불변 view.
#[derive(Clone, Copy)]
pub struct Inputs<'a> {
    frames: &'a [Frame],
    ports: &'a [usize],
}
impl<'a> Inputs<'a> {
    /// 검증한 포트 ID를 슬롯에 연결한다.
    pub fn new(frames: &'a [Frame], ports: &'a [usize]) -> Self {
        Self { frames, ports }
    }
    /// 연결한 입력 수.
    pub fn len(&self) -> usize {
        self.ports.len()
    }
    /// 입력 공집합 여부.
    pub fn is_empty(&self) -> bool {
        self.ports.is_empty()
    }
    /// 호출 인수 순서의 포트를 조회한다.
    pub fn get(&self, index: usize) -> Option<&'a Frame> {
        self.ports.get(index).and_then(|&id| self.frames.get(id))
    }
}

/// 준비 단계에서 상태를 예약한 수치 커널.
pub trait Kernel {
    /// 현재 입력에서 생성한 추가 출력을 차례로 게시한다. false에서 입력 경계가 완료된다.
    fn drain(&mut self, _output: &mut Frame) -> Result<bool, String> {
        Ok(false)
    }

    /// 전체 입력 포트가 준비된 tick을 처리한다. 기본 구현은 단일 입력으로 연결한다.
    fn process_inputs(
        &mut self,
        inputs: Inputs<'_>,
        tick: Tick,
        output: &mut Frame,
    ) -> Result<bool, String> {
        if inputs.len() != 1 {
            return Err("이 커널은 입력 포트 하나를 요구한다.".into());
        }
        self.process(inputs.get(0), tick, output)
    }
    /// 예약 상한 안에 복원용 상태를 저장한다.
    fn checkpoint(&self, _output: &mut StateWriter) -> Result<(), String> {
        Err("커널에 checkpoint codec을 연결한다.".into())
    }
    /// 새 인스턴스에 저장 상태를 복원한다.
    fn restore(&mut self, _input: &mut StateReader<'_>) -> Result<(), String> {
        Err("커널에 restore codec을 연결한다.".into())
    }
    /// 예약 출력 슬롯을 채우고 이번 tick의 게시 여부를 반환한다.
    fn process(
        &mut self,
        input: Option<&Frame>,
        tick: Tick,
        output: &mut Frame,
    ) -> Result<bool, String>;
}

/// CLI·실행 bundle이 런타임에 주입하는 커널 생성 경계.
pub trait KernelFactory {
    /// 정적 번들에 실제 연결한 사용자 계약을 조회한다.
    fn extension_contract(&self, _name: &str) -> Option<&OperatorSpec> {
        None
    }
    /// 구체 구성의 상태·scratch를 예약한다.
    fn create(&self, specification: &KernelSpec) -> Result<Box<dyn Kernel>, String>;
}
