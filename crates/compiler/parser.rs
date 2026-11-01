use crate::{
    Diagnostic,
    lexer::{Kind, Token, lex},
    syntax::*,
};
use bci_schema::{number::Rational, types::Span, unit::Unit};

pub(crate) fn parse(source: &str) -> Result<Program, Diagnostic> {
    Parser {
        source,
        tokens: lex(source)?,
        position: 0,
        depth: 0,
    }
    .program()
}

struct Parser<'a> {
    source: &'a str,
    tokens: Vec<Token>,
    position: usize,
    depth: usize,
}
impl Parser<'_> {
    fn current(&self) -> &Token {
        &self.tokens[self.position]
    }
    fn is(&self, text: &str) -> bool {
        matches!(&self.current().kind,Kind::Ident(s)|Kind::Symbol(s) if s==text)
    }
    fn eat(&mut self, text: &str) -> bool {
        if self.is(text) {
            self.position += 1;
            true
        } else {
            false
        }
    }
    fn error(&self, message: impl Into<String>) -> Diagnostic {
        Diagnostic::new("E0101", message, self.current().span)
    }
    fn expect(&mut self, text: &str) -> Result<(), Diagnostic> {
        if self.eat(text) {
            Ok(())
        } else {
            Err(self.error(format!("{text} 토큰을 요구한다.")))
        }
    }
    fn ident(&mut self) -> Result<String, Diagnostic> {
        if let Kind::Ident(s) = self.current().kind.clone() {
            self.position += 1;
            Ok(s)
        } else {
            Err(self.error("식별자를 요구한다."))
        }
    }
    fn text(&mut self) -> Result<String, Diagnostic> {
        if let Kind::Text(s) = self.current().kind.clone() {
            self.position += 1;
            Ok(s)
        } else {
            Err(self.error("문자열을 요구한다."))
        }
    }
    fn path(&mut self) -> Result<String, Diagnostic> {
        let mut name = self.ident()?;
        while self.eat(".") {
            name.push('.');
            name.push_str(&self.ident()?);
        }
        Ok(name)
    }
    fn end(&self, start: usize) -> Span {
        Span {
            start,
            end: self.tokens[self.position.saturating_sub(1)].span.end,
        }
    }
    fn enter(&mut self) -> Result<(), Diagnostic> {
        self.depth += 1;
        if self.depth > 128 {
            Err(self.error("구문 중첩 깊이는 128 이하여야 한다."))
        } else {
            Ok(())
        }
    }

    fn program(&mut self) -> Result<Program, Diagnostic> {
        self.expect("language")?;
        let language = self.text()?;
        self.expect(";")?;
        self.expect("module")?;
        let module = self.path()?;
        self.expect(";")?;
        let mut p = Program {
            language,
            module,
            imports: Vec::new(),
            bindings: Vec::new(),
            graphs: Vec::new(),
            runs: Vec::new(),
        };
        while self.current().kind != Kind::End {
            let start = self.current().span.start;
            let public = self.eat("pub");
            if self.eat("use") && !public {
                let path = self.path()?;
                let alias = if self.eat("as") {
                    self.ident()?
                } else {
                    path.rsplit('.').next().unwrap_or(&path).into()
                };
                self.expect(";")?;
                p.imports.push((path, alias, self.end(start)));
            } else if self.eat("graph") {
                let name = self.ident()?;
                self.expect("(")?;
                let mut params = Vec::new();
                if !self.eat(")") {
                    loop {
                        let param = self.ident()?;
                        self.expect(":")?;
                        let ty = self.ty()?;
                        params.push((param, ty));
                        if self.eat(")") {
                            break;
                        }
                        self.expect(",")?;
                        if self.eat(")") {
                            break;
                        }
                    }
                }
                self.expect("->")?;
                let output = self.ty()?;
                self.expect("!")?;
                let mode = self.ident()?;
                if !matches!(mode.as_str(), "live" | "record") {
                    return Err(self.error("graph 모드는 live 또는 record를 요구한다."));
                }
                let body = self.body(true)?;
                p.graphs.push(Graph {
                    name,
                    params,
                    output,
                    mode,
                    body,
                    span: self.end(start),
                });
            } else if self.eat("run") && !public {
                let name = self.ident()?;
                let body = self.body(false)?;
                p.runs.push(Run {
                    name,
                    body,
                    span: self.end(start),
                });
            } else {
                let kind = self.ident()?;
                if ![
                    "const", "schema", "clock", "device", "dataset", "model", "protocol",
                    "resource",
                ]
                .contains(&kind.as_str())
                {
                    return Err(self.error("최상위 선언 종류를 확인한다."));
                }
                p.bindings.push(self.binding(kind, start)?);
            }
        }
        Ok(p)
    }

    fn binding(&mut self, kind: String, start: usize) -> Result<Binding, Diagnostic> {
        let name = self.ident()?;
        let annotation = if self.eat(":") {
            Some(self.ty()?)
        } else {
            None
        };
        self.expect("=")?;
        let value = self.expression(0)?;
        self.expect(";")?;
        Ok(Binding {
            name,
            kind,
            annotation,
            value,
            span: self.end(start),
        })
    }

    fn body(&mut self, graph: bool) -> Result<Vec<Statement>, Diagnostic> {
        self.expect("{")?;
        let mut body = Vec::new();
        while !self.eat("}") {
            let start = self.current().span.start;
            if self.eat("let") {
                body.push(Statement::Let(Box::new(self.binding("let".into(), start)?)));
            } else if self.eat("return") {
                if !graph {
                    return Err(self.error("return은 graph 본문에서 사용한다."));
                }
                let value = self.expression(0)?;
                self.expect(";")?;
                self.expect("}")?;
                body.push(Statement::Return(value));
                return Ok(body);
            } else {
                if ["fn", "if", "match", "for", "while", "loop"]
                    .iter()
                    .any(|s| self.is(s))
                {
                    return Err(Diagnostic::new(
                        "E0112",
                        "일반 제어 흐름은 Rust 연산자에 작성한다.",
                        self.current().span,
                    ));
                }
                let value = self.expression(0)?;
                self.expect(";")?;
                body.push(Statement::Expression(value));
            }
        }
        if graph {
            return Err(self.error("graph 마지막에 return을 작성한다."));
        }
        Ok(body)
    }

    fn ty(&mut self) -> Result<TypeExpr, Diagnostic> {
        self.enter()?;
        let start = self.current().span.start;
        let result = if self.eat("(") {
            let mut args = vec![self.ty()?];
            self.expect(",")?;
            args.push(self.ty()?);
            while self.eat(",") {
                args.push(self.ty()?);
            }
            self.expect(")")?;
            TypeExpr {
                name: "Product".into(),
                args,
                literal: None,
                span: self.end(start),
            }
        } else if matches!(self.current().kind, Kind::Number(_)) {
            let expression = self.number()?;
            let ExprKind::Number(number, unit) = expression.kind else {
                unreachable!()
            };
            TypeExpr {
                name: String::new(),
                args: Vec::new(),
                literal: Some((number, unit)),
                span: expression.span,
            }
        } else {
            let name = self.path()?;
            let mut args = Vec::new();
            if self.eat("<") {
                args.push(self.ty()?);
                while self.eat(",") {
                    args.push(self.ty()?);
                }
                self.expect(">")?;
            }
            TypeExpr {
                name,
                args,
                literal: None,
                span: self.end(start),
            }
        };
        self.depth -= 1;
        Ok(result)
    }

    fn number(&mut self) -> Result<Expr, Diagnostic> {
        let token = self.current().clone();
        let Kind::Number(text) = token.kind else {
            return Err(self.error("숫자를 요구한다."));
        };
        self.position += 1;
        let number = text
            .parse::<Rational>()
            .map_err(|e| Diagnostic::new("E0501", e, token.span))?;
        let unit = if self.is("[") && self.current().span.start == token.span.end {
            self.position += 1;
            let start = self.current().span.start;
            while !self.is("]") {
                if self.current().kind == Kind::End {
                    return Err(self.error("단위의 닫는 ]를 추가한다."));
                }
                self.position += 1;
            }
            let end = self.current().span.start;
            self.expect("]")?;
            self.source[start..end]
                .parse::<Unit>()
                .map_err(|e| Diagnostic::new("E0203", e, self.end(token.span.start)))?
        } else {
            Unit::scalar()
        };
        Ok(Expr {
            kind: ExprKind::Number(number, unit),
            span: self.end(token.span.start),
        })
    }

    fn expression(&mut self, min: u8) -> Result<Expr, Diagnostic> {
        self.enter()?;
        let mut left = self.atom()?;
        while let Kind::Symbol(symbol) = &self.current().kind {
            let operator = symbol.clone();
            let precedence = match operator.as_str() {
                "|>" => 1,
                "||" => 2,
                "&&" => 3,
                "==" | "!=" => 4,
                "<" | "<=" | ">" | ">=" => 5,
                "+" | "-" => 6,
                "*" | "/" => 7,
                _ => break,
            };
            if precedence < min {
                break;
            }
            self.position += 1;
            let right = self.expression(precedence + 1)?;
            let span = Span {
                start: left.span.start,
                end: right.span.end,
            };
            if operator == "|>" {
                let ExprKind::Call(target, mut args) = right.kind else {
                    return Err(Diagnostic::new(
                        "E0101",
                        "pipe 오른쪽에는 연산자 호출을 작성한다.",
                        right.span,
                    ));
                };
                args.insert(
                    0,
                    Argument {
                        name: None,
                        value: left,
                    },
                );
                left = Expr {
                    kind: ExprKind::Call(target, args),
                    span,
                };
            } else {
                left = Expr {
                    kind: ExprKind::Binary(operator, Box::new(left), Box::new(right)),
                    span,
                };
            }
            check_expression_depth(&left)?;
        }
        self.depth -= 1;
        Ok(left)
    }

    fn atom(&mut self) -> Result<Expr, Diagnostic> {
        let token = self.current().clone();
        let start = token.span.start;
        let mut value = match token.kind {
            Kind::Number(_) => self.number()?,
            Kind::Text(text) => {
                self.position += 1;
                Expr {
                    kind: ExprKind::Text(text),
                    span: token.span,
                }
            }
            Kind::Ident(ref name) if matches!(name.as_str(), "true" | "false") => {
                self.position += 1;
                Expr {
                    kind: ExprKind::Bool(name == "true"),
                    span: token.span,
                }
            }
            Kind::Ident(_) => {
                let name = self.path()?;
                Expr {
                    kind: ExprKind::Path(name),
                    span: self.end(start),
                }
            }
            _ if self.eat("-") || self.eat("!") => {
                let Kind::Symbol(operator) = token.kind else {
                    unreachable!()
                };
                let rhs = self.expression(8)?;
                Expr {
                    kind: ExprKind::Unary(operator, Box::new(rhs)),
                    span: self.end(start),
                }
            }
            _ if self.eat("[") => {
                let args = self.list("]")?;
                Expr {
                    kind: ExprKind::Array(args),
                    span: self.end(start),
                }
            }
            _ if self.eat("#{") => {
                let mut fields = Vec::new();
                if !self.eat("}") {
                    loop {
                        let name = self.ident()?;
                        self.expect(":")?;
                        let value = self.expression(0)?;
                        fields.push((name, value));
                        if self.eat("}") {
                            break;
                        }
                        self.expect(",")?;
                        if self.eat("}") {
                            break;
                        }
                    }
                }
                Expr {
                    kind: ExprKind::Record(fields),
                    span: self.end(start),
                }
            }
            _ if self.eat("(") => {
                let first = self.expression(0)?;
                if self.eat(",") {
                    let mut args = vec![first, self.expression(0)?];
                    while self.eat(",") {
                        args.push(self.expression(0)?);
                    }
                    self.expect(")")?;
                    Expr {
                        kind: ExprKind::Tuple(args),
                        span: self.end(start),
                    }
                } else {
                    self.expect(")")?;
                    first
                }
            }
            _ => return Err(self.error("값·이름·호출 표현식을 요구한다.")),
        };
        check_expression_depth(&value)?;
        loop {
            if self.eat("(") {
                let mut args = Vec::new();
                let mut named = false;
                if !self.eat(")") {
                    loop {
                        let name = if matches!(self.current().kind, Kind::Ident(_))
                            && self
                                .tokens
                                .get(self.position + 1)
                                .is_some_and(|t| matches!(&t.kind,Kind::Symbol(s) if s==":"))
                        {
                            let name = self.ident()?;
                            self.expect(":")?;
                            named = true;
                            Some(name)
                        } else {
                            if named {
                                return Err(self.error("위치 인수는 이름 인수 앞에 배치한다."));
                            }
                            None
                        };
                        args.push(Argument {
                            name,
                            value: self.expression(0)?,
                        });
                        if self.eat(")") {
                            break;
                        }
                        self.expect(",")?;
                        if self.eat(")") {
                            break;
                        }
                    }
                }
                value = Expr {
                    kind: ExprKind::Call(Box::new(value), args),
                    span: self.end(start),
                };
            } else if self.eat(".") {
                let field = self.ident()?;
                value = Expr {
                    kind: ExprKind::Field(Box::new(value), field),
                    span: self.end(start),
                };
            } else {
                break;
            }
            check_expression_depth(&value)?;
        }
        Ok(value)
    }

    fn list(&mut self, close: &str) -> Result<Vec<Expr>, Diagnostic> {
        let mut values = Vec::new();
        if self.eat(close) {
            return Ok(values);
        }
        loop {
            values.push(self.expression(0)?);
            if self.eat(close) {
                break;
            }
            self.expect(",")?;
            if self.eat(close) {
                break;
            }
        }
        Ok(values)
    }
}

fn check_expression_depth(expression: &Expr) -> Result<(), Diagnostic> {
    let mut pending = vec![(expression, 1)];
    while let Some((expr, depth)) = pending.pop() {
        if depth > 128 {
            return Err(Diagnostic::new(
                "E0108",
                "표현식 트리 깊이는 128 이하여야 한다.",
                expr.span,
            ));
        }
        match &expr.kind {
            ExprKind::Unary(_, child) | ExprKind::Field(child, _) => {
                pending.push((child, depth + 1))
            }
            ExprKind::Binary(_, lhs, rhs) => {
                pending.push((lhs, depth + 1));
                pending.push((rhs, depth + 1));
            }
            ExprKind::Call(target, args) => {
                pending.push((target, depth + 1));
                pending.extend(args.iter().map(|arg| (&arg.value, depth + 1)));
            }
            ExprKind::Array(items) | ExprKind::Tuple(items) => {
                pending.extend(items.iter().map(|item| (item, depth + 1)))
            }
            ExprKind::Record(items) => {
                pending.extend(items.iter().map(|(_, item)| (item, depth + 1)))
            }
            _ => {}
        }
    }
    Ok(())
}
