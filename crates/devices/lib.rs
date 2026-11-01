//! 장치 프로필, 구성 바인딩과 DeviceAdapter 경계의 소유 패키지다.
//!
//! 제조사 SDK와 디코더는 crates/adapter-이름 패키지에서 이 계약을 구현한다.
//! compiler와 runtime은 공통 descriptor와 인터페이스를 사용한다.

/// 장치 reader도 파일·합성과 같은 프레임·EOF 경계를 구현한다.
pub use bci_schema::input::{InputError, InputSource, SourceFactory, SourceSpec};
