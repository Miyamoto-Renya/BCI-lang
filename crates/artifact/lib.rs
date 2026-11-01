//! Artifact codec, manifest, 내용 식별자와 기록 컨테이너를 소유한다.
//!
//! 저장 형식의 판본과 복구 규칙은 이 패키지가 관리한다.
//! runtime과의 기록·재생 연결은 상위 조립 계층이 구성한다.

pub mod checkpoint;
mod csv;
pub mod descriptor;
pub mod evaluation;
pub mod input;
pub mod learning;
pub mod recording;
mod replay;
mod stream_codec;
