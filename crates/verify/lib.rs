//! 실행 계획의 토큰·용량·시간 증명서를 독립 검사하는 패키지다.
//!
//! 검증의 입력 구조는 bci-ir, 도메인 계약은 bci-schema가 제공한다.
//! VerifiedPlan 생성 권한은 이 패키지의 검증 경계가 소유한다.

mod sequential;
pub use sequential::{VerifiedPlan, verify};
