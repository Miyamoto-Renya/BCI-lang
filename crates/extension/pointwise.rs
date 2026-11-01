use crate::{Kernel, KernelFactory, OperatorSpec, Tick};
use bci_schema::{execution::Frame, extension::validate_registry, kernel::KernelSpec};

/// 매크로가 연결하는 계약·함수·정확한 bit 참조 집합.
pub struct PointwiseRegistration {
    /// compiler와 runtime이 대조하는 계약.
    pub spec: OperatorSpec,
    /// 유효 원소마다 호출할 함수.
    pub function: fn(f64) -> f64,
    /// 입력 bit·기대 출력 bit 쌍.
    pub reference: &'static [(u64, u64)],
}

impl PointwiseRegistration {
    fn verify(&self) -> Result<(), String> {
        if self.reference.is_empty() || self.reference.len() > 4096 {
            return Err(format!(
                "{}: reference를 1..4096개 지정한다.",
                self.spec.name
            ));
        }
        for &(input, expected) in self.reference {
            if !f64::from_bits(input).is_finite() || !f64::from_bits(expected).is_finite() {
                return Err(format!(
                    "{}: reference는 유한 f64 bit를 요구한다.",
                    self.spec.name
                ));
            }
            let actual = std::panic::catch_unwind(|| (self.function)(f64::from_bits(input)))
                .map_err(|_| {
                    format!(
                        "{}: reference 입력 {input:016x}에서 panic, {}",
                        self.spec.name, self.spec.rust_source
                    )
                })?;
            if actual.to_bits() != expected {
                return Err(format!(
                    "{}: reference 입력 {input:016x}, 기대 {expected:016x}, 관측 {:016x}, {}",
                    self.spec.name,
                    actual.to_bits(),
                    self.spec.rust_source
                ));
            }
        }
        Ok(())
    }
}

/// 한 실행 파일에 정적으로 연결한 사용자 연산자 집합.
pub struct Extensions {
    registrations: Vec<PointwiseRegistration>,
    states: Vec<crate::StatefulRegistration>,
    specifications: Vec<OperatorSpec>,
}

impl Extensions {
    /// 표준 커널만 연결하는 빈 registry.
    pub fn empty() -> Self {
        Self {
            registrations: Vec::new(),
            states: Vec::new(),
            specifications: Vec::new(),
        }
    }

    /// 선언의 유일성과 profile을 검사해 registry를 구성한다.
    pub fn new(registrations: Vec<PointwiseRegistration>) -> Result<Self, String> {
        if registrations.iter().any(|r| r.spec.state.is_some()) {
            return Err("pointwise registration에는 무상태 계약을 연결한다.".into());
        }
        let specifications = registrations
            .iter()
            .map(|r| r.spec.clone())
            .collect::<Vec<_>>();
        validate_registry(&specifications)?;
        Ok(Self {
            registrations,
            states: Vec::new(),
            specifications,
        })
    }

    /// 상태 연산자를 더하고 전체 namespace의 유일성을 검사한다.
    pub fn with_stateful(
        mut self,
        states: Vec<crate::StatefulRegistration>,
    ) -> Result<Self, String> {
        if states.iter().any(|s| s.spec.state.is_none()) {
            return Err("상태 registration에 stateful 계약을 연결한다.".into());
        }
        self.specifications
            .extend(states.iter().map(|r| r.spec.clone()));
        validate_registry(&self.specifications)?;
        self.states.extend(states);
        Ok(self)
    }

    /// compiler에 전달할 선언 데이터.
    pub fn specifications(&self) -> &[OperatorSpec] {
        &self.specifications
    }

    /// 번들의 모든 exact-bit reference를 실제 함수에 적용한다.
    pub fn verify_references(&self) -> Result<usize, String> {
        let mut count = 0;
        for registration in &self.registrations {
            registration.verify()?;
            count += registration.reference.len();
        }
        for registration in &self.states {
            count += std::panic::catch_unwind(|| (registration.verify)(&registration.spec))
                .map_err(|_| {
                    format!(
                        "{}: 상태 reference에서 panic이 발생했다.",
                        registration.spec.name
                    )
                })??;
        }
        Ok(count)
    }

    /// 사용자 함수와 표준 커널 factory를 하나의 실행 경계로 묶는다.
    pub fn kernels<'a>(&'a self, standard: &'a dyn KernelFactory) -> ExtensionKernels<'a> {
        ExtensionKernels {
            extensions: self,
            standard,
        }
    }
}

/// 사용자 연산자는 registry로, 표준 연산자는 주입한 factory로 생성한다.
pub struct ExtensionKernels<'a> {
    extensions: &'a Extensions,
    standard: &'a dyn KernelFactory,
}

impl KernelFactory for ExtensionKernels<'_> {
    fn extension_contract(&self, name: &str) -> Option<&OperatorSpec> {
        self.extensions
            .specifications
            .iter()
            .find(|s| s.name == name)
    }

    fn create(&self, spec: &KernelSpec) -> Result<Box<dyn Kernel>, String> {
        let KernelSpec::Extension { operator, channels } = spec else {
            return self.standard.create(spec);
        };
        if operator.state.is_some() {
            let registered = self
                .extensions
                .states
                .iter()
                .find(|r| r.spec == **operator)
                .ok_or("상태 계약을 실행 번들과 맞춘다.")?;
            (registered.verify)(operator)?;
            return (registered.create)(operator, *channels);
        }
        let registered = self
            .extensions
            .registrations
            .iter()
            .find(|r| r.spec == **operator)
            .ok_or_else(|| {
                format!(
                    "{}: 계획과 같은 계약·함수를 번들에 등록한다.",
                    operator.name
                )
            })?;
        registered.verify()?;
        Ok(Box::new(Pointwise {
            function: registered.function,
            channels: *channels,
        }))
    }
}

struct Pointwise {
    function: fn(f64) -> f64,
    channels: usize,
}
impl Kernel for Pointwise {
    fn checkpoint(&self, _: &mut crate::StateWriter) -> Result<(), String> {
        Ok(())
    }
    fn restore(&mut self, input: &mut crate::StateReader<'_>) -> Result<(), String> {
        input.finish()
    }
    fn process(
        &mut self,
        input: Option<&Frame>,
        _: Tick,
        output: &mut Frame,
    ) -> Result<bool, String> {
        let input = input.ok_or_else(|| "pointwise 입력 포트를 연결한다.".to_string())?;
        if [
            input.values.len(),
            input.valid.len(),
            output.values.len(),
            output.valid.len(),
        ]
        .iter()
        .any(|&n| n != self.channels)
        {
            return Err("pointwise의 입력·출력 배열을 계약 채널 수와 맞춘다.".into());
        }
        output.inherit(input);
        for ((to, &from), &valid) in output
            .values
            .iter_mut()
            .zip(&input.values)
            .zip(&input.valid)
        {
            *to = if valid == 1 {
                (self.function)(from)
            } else {
                from
            };
        }
        Ok(true)
    }
}
