use crate::Diagnostic;
use bci_schema::types::Span;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Kind {
    Ident(String),
    Number(String),
    Text(String),
    Symbol(String),
    End,
}
#[derive(Debug, Clone)]
pub(crate) struct Token {
    pub kind: Kind,
    pub span: Span,
}

pub(crate) fn lex(source: &str) -> Result<Vec<Token>, Diagnostic> {
    if source.len() > 1_048_576 {
        return Err(Diagnostic::new(
            "E0108",
            "소스 크기는 1 MiB 이하여야 한다.",
            Span::default(),
        ));
    }
    let bytes = source.as_bytes();
    let mut pos = 0;
    let mut tokens = Vec::new();
    while pos < bytes.len() {
        let start = pos;
        if bytes[pos].is_ascii_whitespace() {
            pos += 1;
            continue;
        }
        if source[pos..].starts_with("//") {
            while pos < bytes.len() && bytes[pos] != b'\n' {
                pos += 1;
            }
            continue;
        }
        if source[pos..].starts_with("/*") {
            pos += 2;
            let mut depth = 1;
            while pos < bytes.len() && depth > 0 {
                if source[pos..].starts_with("/*") {
                    depth += 1;
                    pos += 2;
                } else if source[pos..].starts_with("*/") {
                    depth -= 1;
                    pos += 2;
                } else {
                    pos += source[pos..].chars().next().map_or(1, char::len_utf8);
                }
            }
            if depth > 0 {
                return Err(Diagnostic::new(
                    "E0101",
                    "블록 주석의 닫는 */를 추가한다.",
                    Span { start, end: pos },
                ));
            }
            continue;
        }
        let kind = if bytes[pos].is_ascii_alphabetic() || bytes[pos] == b'_' {
            pos += 1;
            while pos < bytes.len() && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_') {
                pos += 1;
            }
            Kind::Ident(source[start..pos].into())
        } else if bytes[pos].is_ascii_digit() {
            while pos < bytes.len() && (bytes[pos].is_ascii_digit() || bytes[pos] == b'_') {
                pos += 1;
            }
            if pos + 1 < bytes.len() && bytes[pos] == b'.' && bytes[pos + 1].is_ascii_digit() {
                pos += 1;
                while pos < bytes.len() && (bytes[pos].is_ascii_digit() || bytes[pos] == b'_') {
                    pos += 1;
                }
            }
            if pos < bytes.len() && matches!(bytes[pos], b'e' | b'E') {
                pos += 1;
                if pos < bytes.len() && matches!(bytes[pos], b'+' | b'-') {
                    pos += 1;
                }
                let exponent = pos;
                while pos < bytes.len() && (bytes[pos].is_ascii_digit() || bytes[pos] == b'_') {
                    pos += 1;
                }
                if exponent == pos {
                    return Err(Diagnostic::new(
                        "E0101",
                        "숫자 지수에 정수를 지정한다.",
                        Span { start, end: pos },
                    ));
                }
            }
            for index in start..pos {
                if bytes[index] == b'_'
                    && (index == start
                        || index + 1 == pos
                        || !bytes[index - 1].is_ascii_digit()
                        || !bytes[index + 1].is_ascii_digit())
                {
                    return Err(Diagnostic::new(
                        "E0101",
                        "숫자 구분자 _는 두 숫자 사이에 배치한다.",
                        Span { start, end: pos },
                    ));
                }
            }
            Kind::Number(source[start..pos].into())
        } else if bytes[pos] == b'"' {
            pos += 1;
            let mut escaped = false;
            let mut closed = false;
            while pos < bytes.len() {
                let byte = bytes[pos];
                pos += 1;
                if !escaped && byte == b'"' {
                    closed = true;
                    break;
                }
                escaped = !escaped && byte == b'\\';
            }
            if !closed {
                return Err(Diagnostic::new(
                    "E0101",
                    "문자열의 닫는 따옴표를 추가한다.",
                    Span { start, end: pos },
                ));
            }
            let value = serde_json::from_str::<String>(&source[start..pos]).map_err(|error| {
                Diagnostic::new(
                    "E0101",
                    format!("문자열 escape를 확인한다: {error}"),
                    Span { start, end: pos },
                )
            })?;
            Kind::Text(value)
        } else if let Some(symbol) = ["|>", "->", "#{", "==", "!=", "<=", ">=", "&&", "||"]
            .into_iter()
            .find(|symbol| source[pos..].starts_with(symbol))
        {
            pos += symbol.len();
            Kind::Symbol(symbol.into())
        } else if b"(){}[];,:.<>!+-*/^".contains(&bytes[pos]) || bytes[pos] == b'=' {
            pos += 1;
            Kind::Symbol(source[start..pos].into())
        } else {
            let end = pos + source[pos..].chars().next().map_or(1, char::len_utf8);
            return Err(Diagnostic::new(
                "E0101",
                format!("등록한 구문 토큰을 사용한다: {}", &source[start..end]),
                Span { start, end },
            ));
        };
        tokens.push(Token {
            kind,
            span: Span { start, end: pos },
        });
    }
    tokens.push(Token {
        kind: Kind::End,
        span: Span {
            start: pos,
            end: pos,
        },
    });
    Ok(tokens)
}
