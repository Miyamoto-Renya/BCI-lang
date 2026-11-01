//! SOS IIR의 표본률·안정성·초기화 계약.
use crate::{
    number::Rational,
    types::{DType, Occurrence, StreamType, ValueType},
};
use sha2::{Digest, Sha256};

/// 단위원 경계에서 f64 Schur 부등식에 적용하는 최소 여유.
pub const SCHUR_MARGIN: f64 = 1e-12;

/// a0=1로 정규화한 순서 있는 SOS 필터.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Realization {
    /// 계수를 설계한 입력 발생률 Hz.
    pub rate: Rational,
    /// 초기화 이후 무효로 표시할 연속 유효 입력 수.
    pub warmup: usize,
    /// 각 절의 b0,b1,b2,a1,a2 순서 계수.
    pub sections: Vec<[Rational; 5]>,
}
impl Realization {
    /// 정확한 Schur 조건과 실행 f64 계수의 경계 여유를 검사한다.
    pub fn validate(&self) -> Result<(), String> {
        if !self.rate.is_positive()
            || self.warmup > 1_000_000
            || !(1..=64).contains(&self.sections.len())
        {
            return Err("SOS는 양의 표본률·1..64절·0..1000000 warmup을 요구한다.".into());
        }
        for (index, s) in self.sections.iter().enumerate() {
            let a1 = s[3];
            let a2 = s[4];
            let conditions = [
                Rational::ONE.checked_add(a1)?.checked_add(a2)?,
                Rational::ONE.checked_add(a1.negate()?)?.checked_add(a2)?,
                Rational::ONE.checked_add(a2.negate()?)?,
                Rational::ONE.checked_add(a2)?,
            ];
            let a1 = a1.to_f64();
            let a2 = a2.to_f64();
            if conditions.iter().any(|c| !c.is_positive())
                || s.iter().any(|n| !n.to_f64().is_finite())
                || [1.0 + a1 + a2, 1.0 - a1 + a2, 1.0 - a2, 1.0 + a2]
                    .iter()
                    .any(|&v| v <= SCHUR_MARGIN)
            {
                return Err(format!(
                    "SOS 절 {index}의 극점 안정성과 f64 Schur 여유 > {SCHUR_MARGIN}을 요구한다."
                ));
            }
        }
        Ok(())
    }
    /// 입력 벡터·발생률을 고정 실현과 대조한다.
    pub fn accepts(&self, input: &StreamType) -> Result<(), String> {
        self.validate()?;
        if !matches!(input.value, ValueType::Vector(DType::F64, _)) {
            return Err("IIR는 Vec<f64,C> 입력을 요구한다.".into());
        }
        let rate = match input.occurrence {
            Occurrence::Uniform { rate, .. } | Occurrence::PendingUniform { rate, .. } => rate,
            _ => return Err("IIR는 균일 입력 발생률을 요구한다.".into()),
        };
        if rate != self.rate {
            return Err(format!(
                "IIR 설계 표본률 {}Hz와 입력 {rate}Hz를 맞춘다.",
                self.rate
            ));
        }
        Ok(())
    }
    /// 경로와 십진 표기 차이를 정규화한 실행 계약 digest.
    pub fn digest(&self) -> String {
        let mut hash = Sha256::new();
        hash.update(b"bci-iir-sos-df2t/0.1");
        for field in std::iter::once(self.rate.to_string())
            .chain(std::iter::once(self.warmup.to_string()))
            .chain(self.sections.iter().flatten().map(ToString::to_string))
        {
            hash.update((field.len() as u64).to_le_bytes());
            hash.update(field.as_bytes());
        }
        format!("{:x}", hash.finalize())
    }
    /// f64 계수·채널 지연 상태·warmup 카운터와 제어 구조 예약량.
    pub fn state_bytes(&self, channels: usize) -> Result<usize, String> {
        self.sections
            .len()
            .checked_mul(40)
            .and_then(|n| {
                channels
                    .checked_mul(self.sections.len())
                    .and_then(|v| v.checked_mul(16))
                    .and_then(|v| n.checked_add(v))
            })
            .and_then(|n| channels.checked_mul(8).and_then(|v| n.checked_add(v)))
            .and_then(|n| n.checked_add(1024))
            .ok_or_else(|| "IIR 상태 예약량 범위를 초과했다.".into())
    }
}
