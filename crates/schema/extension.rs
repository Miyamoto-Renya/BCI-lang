//! 정적 번들이 공유하는 사용자 연산자·상태 자원 계약.

use crate::types::{DType, StreamType, ValueType};

/// 유효 f64 벡터 원소를 같은 측정·시간·품질 타입으로 변환하는 profile.
pub const POINTWISE_PROFILE: &str = "std.contracts.pointwise_f64/0.1";
/// 상태를 가진 동일 격자 f64 벡터 profile.
pub const STATEFUL_PROFILE: &str = "std.contracts.stateful_f64/0.1";

/// 채널 수에 따라 구체화하는 상태·scratch·codec 상한.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateContract {
    /// 상태 codec 판본.
    pub codec: String,
    /// 상태 객체와 고정 소유 메모리 상한.
    pub fixed_bytes: usize,
    /// 채널당 소유 메모리 상한.
    pub channel_bytes: usize,
    /// 채널당 임시 공간 상한.
    pub scratch_bytes: usize,
    /// checkpoint 고정 payload 상한.
    pub checkpoint_bytes: usize,
    /// checkpoint 채널당 payload 상한.
    pub checkpoint_channel_bytes: usize,
}
impl StateContract {
    /// 실행 시 소유 상태와 scratch의 합을 계산한다.
    pub fn memory(&self, channels: usize) -> Result<usize, String> {
        self.channel_bytes
            .checked_add(self.scratch_bytes)
            .and_then(|n| n.checked_mul(channels))
            .and_then(|n| n.checked_add(self.fixed_bytes))
            .filter(|&n| n <= 64 * 1024 * 1024)
            .ok_or_else(|| "상태·scratch 합은 노드당 64 MiB 이하여야 한다.".into())
    }
    /// 사용자 codec의 출력 byte 상한.
    pub fn checkpoint_limit(&self, channels: usize) -> Result<usize, String> {
        self.checkpoint_channel_bytes
            .checked_mul(channels)
            .and_then(|n| n.checked_add(self.checkpoint_bytes))
            .filter(|&n| n <= 16 * 1024 * 1024)
            .ok_or_else(|| "checkpoint payload 상한은 16 MiB다.".into())
    }
}

/// 컴파일·실행·기록이 대조하는 불변 연산자 정체성.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperatorSpec {
    /// stateful profile의 자원·codec 계약. pointwise는 None이다.
    pub state: Option<StateContract>,
    /// BCi 정규 호출 이름.
    pub name: String,
    /// 작성자가 선언한 세 자리 버전.
    pub version: String,
    /// 적용할 SDK profile.
    pub profile: String,
    /// 모든 입력 채널에 요구하는 측정량.
    pub quantity: String,
    /// 계약 파일 원문 SHA-256.
    pub contract_digest: String,
    /// 매크로가 받은 Rust 함수 토큰의 SHA-256.
    pub function_digest: String,
    /// exact-bit reference 파일 원문 SHA-256.
    pub reference_digest: String,
    /// Rust 함수의 모듈 경로.
    pub rust_symbol: String,
    /// Rust 소스 파일·행·열.
    pub rust_source: String,
}

impl OperatorSpec {
    /// registry에 넣기 전에 이름·profile·digest를 검사한다.
    pub fn validate(&self) -> Result<(), String> {
        let identifier = |s: &str| {
            let mut chars = s.chars();
            chars
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        };
        if self.name.len() > 256
            || self.name.split('.').count() < 2
            || !self.name.split('.').all(identifier)
            || self.name.starts_with("std.")
        {
            return Err("확장 이름은 사용자 namespace의 점으로 구분한 식별자로 지정한다.".into());
        }
        let version: Vec<_> = self.version.split('.').collect();
        if version.len() != 3
            || version.iter().any(|s| s.parse::<u32>().is_err())
            || self.version.len() > 32
            || self.profile
                != if self.state.is_some() {
                    STATEFUL_PROFILE
                } else {
                    POINTWISE_PROFILE
                }
        {
            return Err(
                "연산자 kind에 대응하는 profile과 major.minor.patch 버전을 요구한다.".into(),
            );
        }
        if let Some(state) = &self.state {
            if state.codec.is_empty() || state.codec.len() > 128 {
                return Err("state codec은 1..128 bytes 판본 이름을 요구한다.".into());
            }
            state.memory(1)?;
            state.checkpoint_limit(1)?;
        }
        if [&self.quantity, &self.rust_symbol, &self.rust_source]
            .iter()
            .any(|s| s.is_empty() || s.len() > 4096)
        {
            return Err("측정량·Rust symbol·소스 위치는 1..4096 bytes로 지정한다.".into());
        }
        for digest in [
            &self.contract_digest,
            &self.function_digest,
            &self.reference_digest,
        ] {
            if digest.len() != 64
                || !digest
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            {
                return Err("계약·함수·reference digest는 SHA-256 소문자 hex로 지정한다.".into());
            }
        }
        Ok(())
    }

    /// 사용자 profile의 입력 타입·측정량과 동일 출력 조건을 검사한다.
    pub fn accepts(&self, input: &StreamType) -> bool {
        matches!(input.value, ValueType::Vector(DType::F64, n) if n > 0 && n == input.schema.channels.len())
            && input
                .schema
                .channels
                .iter()
                .all(|c| c.quantity == self.quantity)
    }
}

/// 표준 namespace 보호와 확장 이름의 유일성을 검사한다.
pub fn validate_registry(specs: &[OperatorSpec]) -> Result<(), String> {
    if specs.len() > 4096 {
        return Err("정적 번들당 확장 연산자 상한은 4096개다.".into());
    }
    let mut names = std::collections::BTreeSet::new();
    for spec in specs {
        spec.validate()?;
        if !names.insert(&spec.name) {
            return Err(format!("확장 이름 {}을 한 번 등록한다.", spec.name));
        }
    }
    Ok(())
}
