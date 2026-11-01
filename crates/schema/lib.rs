//! BCi 측정·시간·품질과 연산자 계약의 공통 값 타입을 소유한다.
//!
//! 언어·연산자 계약의 판본을 제공한다. 단위·스키마·효과와 registry 기술자는
//! 이 패키지의 도메인별 파일에 배치하고 실행 구현은 소비 패키지가 소유한다.

pub mod evaluation;
pub mod execution;
pub mod extension;
pub mod iir;
pub mod input;
pub mod kernel;
pub mod learning;
pub mod number;
pub mod registry;
pub mod resample;
pub mod selection;
pub mod spatial;
pub mod spectral;
pub mod types;
pub mod unit;

/// BCi 표면 언어와 의미 규격의 판본.
pub const LANGUAGE_VERSION: &str = "bci/0.1";

/// Rust 연산자 등록 계약의 판본.
pub const OPERATOR_CONTRACT_VERSION: &str = "bci-operator/0.1";
