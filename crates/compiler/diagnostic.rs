//! 소스 범위가 붙은 컴파일 진단.

use bci_schema::types::Span;

/// 컴파일 오류와 수정 행동.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    /// 설계 규격의 진단 코드.
    pub code: String,
    /// 원인과 요구 조건.
    pub message: String,
    /// 원본 UTF-8 byte 범위.
    pub span: Span,
}

impl Diagnostic {
    /// 컴파일·실행 오류를 공통 소스 위치 진단으로 구성한다.
    pub fn new(code: &str, message: impl Into<String>, span: Span) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            span,
        }
    }

    /// 파일·줄·문자 열과 해당 소스 줄을 출력한다.
    pub fn render(&self, path: &str, source: &str) -> String {
        let start = self.span.start.min(source.len());
        let start = source.floor_char_boundary(start);
        let prefix = &source[..start];
        let line = prefix.bytes().filter(|&b| b == b'\n').count() + 1;
        let line_start = prefix.rfind('\n').map_or(0, |n| n + 1);
        let column = source[line_start..start].chars().count() + 1;
        let line_end = source[start..]
            .find('\n')
            .map_or(source.len(), |n| start + n);
        format!(
            "{path}:{line}:{column}: {}: {}\n  {}\n  {}^",
            self.code,
            self.message,
            &source[line_start..line_end],
            " ".repeat(column - 1)
        )
    }
}
