//! 인과 polyphase FIR 재표본화의 정확한 격자와 예약 계약.
use crate::{
    number::Rational,
    types::{DType, Occurrence, StreamType, ValueType},
};

/// 고속 격자의 계수와 입출력 위상.
#[derive(Debug, Clone, PartialEq)]
pub struct Resampling {
    /// 서로소 보간 배수, 1..1024.
    pub up: usize,
    /// 서로소 감산 배수, 1..1024.
    pub down: usize,
    /// `h[0]`부터 시작하는 인과 고속 격자 FIR 계수.
    pub coefficients: Vec<f64>,
    /// 입력 발생률 Hz.
    pub rate: Rational,
    /// 입력의 확정 위상 초.
    pub input_phase: Rational,
    /// 출력의 확정 위상 초.
    pub phase: Rational,
}
impl Resampling {
    /// 입력 격자에서 출력 시작점까지의 고속 격자 index.
    pub fn offset(&self) -> Result<u64, String> {
        let mut a = self.up;
        let mut b = self.down;
        while b != 0 {
            (a, b) = (b, a % b);
        }
        if !(1..=1024).contains(&self.up)
            || !(1..=1024).contains(&self.down)
            || a != 1
            || !self.rate.is_positive()
            || !(1..=65536).contains(&self.coefficients.len())
            || self.coefficients.iter().any(|v| !v.is_finite())
        {
            return Err(
                "재표본화는 서로소 up·down 1..1024, 양의 표본률, 유한 계수 1..65536개를 요구한다."
                    .into(),
            );
        }
        let offset = self
            .phase
            .checked_add(self.input_phase.negate()?)?
            .multiply(self.rate)?
            .multiply(Rational::integer(self.up as i64))?
            .natural()?;
        if offset >= self.down {
            return Err(
                "재표본화 phase는 입력 위상부터 출력 한 주기 안의 고속 격자점으로 지정한다.".into(),
            );
        }
        Ok(offset as u64)
    }
    /// 타입·고속 격자·인과 support를 검사하고 출력 타입을 구성한다.
    pub fn output(&self, input: &StreamType) -> Result<StreamType, String> {
        self.offset()?;
        if !matches!(input.value, ValueType::Vector(DType::F64, _))
            || input.occurrence
                != (Occurrence::Uniform {
                    rate: self.rate,
                    phase: self.input_phase,
                })
        {
            return Err("재표본화 입력은 확정 Uniform 격자의 f64 Vec를 요구한다.".into());
        }
        let mut output = input.clone();
        output.occurrence = Occurrence::Uniform {
            rate: self.output_rate()?,
            phase: self.phase,
        };
        Ok(output)
    }
    /// 출력 표본률.
    pub fn output_rate(&self) -> Result<Rational, String> {
        self.rate
            .multiply(Rational::new(self.up as i128, self.down as i128)?)
    }
    /// 입력 이력의 고정 슬롯 수.
    pub fn history(&self) -> usize {
        self.coefficients.len().div_ceil(self.up) + 1
    }
    /// 입력 한 개가 생성하는 출력 수 상한.
    pub fn burst(&self) -> usize {
        self.up.div_ceil(self.down)
    }
    /// Frame 이력과 계수·제어 상태의 예약량.
    pub fn memory(&self, channels: usize) -> Result<usize, String> {
        self.offset()?;
        channels
            .checked_mul(9)
            .and_then(|v| v.checked_add(std::mem::size_of::<crate::execution::Frame>() + 128))
            .and_then(|v| v.checked_mul(self.history()))
            .and_then(|v| v.checked_add(self.coefficients.len() * 8 + 1024))
            .ok_or_else(|| "재표본화 상태 예약량 범위를 초과했다.".into())
    }
}
