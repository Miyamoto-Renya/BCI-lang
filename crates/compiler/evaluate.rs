use crate::{
    Diagnostic,
    check::Checker,
    syntax::*,
    value::{Environment, Value},
};
use bci_schema::{registry, types::Span};
use std::collections::BTreeMap;

impl Checker<'_> {
    pub fn eval(
        &mut self,
        expr: &Expr,
        env: &Environment,
        static_only: bool,
    ) -> Result<Value, Diagnostic> {
        if self.evaluation_depth >= 128 {
            return Err(Diagnostic::new(
                "E0108",
                "표현식 평가 깊이는 128 이하여야 한다.",
                expr.span,
            ));
        }
        self.evaluation_depth += 1;
        let result = self.evaluate(expr, env, static_only);
        self.evaluation_depth -= 1;
        result
    }

    fn evaluate(
        &mut self,
        expr: &Expr,
        env: &Environment,
        static_only: bool,
    ) -> Result<Value, Diagnostic> {
        let value = match &expr.kind {
            ExprKind::Number(n, u) => Value::Number(*n, u.clone()),
            ExprKind::Text(t) => Value::Text(t.clone()),
            ExprKind::Bool(b) => Value::Bool(*b),
            ExprKind::Path(name) => self.lookup(name, env, expr.span)?,
            ExprKind::Array(values) => {
                let values = values
                    .iter()
                    .map(|v| self.eval(v, env, static_only))
                    .collect::<Result<Vec<_>, _>>()?;
                if values
                    .windows(2)
                    .any(|v| std::mem::discriminant(&v[0]) != std::mem::discriminant(&v[1]))
                {
                    return Err(Diagnostic::new(
                        "E0201",
                        "배열 요소의 kind를 맞춘다.",
                        expr.span,
                    ));
                }
                Value::Array(values)
            }
            ExprKind::Tuple(values) => Value::Tuple(
                values
                    .iter()
                    .map(|v| self.eval(v, env, static_only))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            ExprKind::Record(fields) => {
                let mut values = BTreeMap::new();
                for (name, value) in fields {
                    if values.contains_key(name) {
                        return Err(Diagnostic::new(
                            "E0104",
                            format!("중복 레코드 필드: {name}"),
                            value.span,
                        ));
                    }
                    values.insert(name.clone(), self.eval(value, env, static_only)?);
                }
                Value::Record(values)
            }
            ExprKind::Field(value, name) => {
                let value = self.eval(value, env, static_only)?;
                self.field(value, name, expr.span)?
            }
            ExprKind::Unary(operator, rhs) => {
                match (operator.as_str(), self.eval(rhs, env, static_only)?) {
                    ("-", Value::Number(n, u)) => Value::Number(
                        n.negate()
                            .map_err(|e| Diagnostic::new("E0501", e, expr.span))?,
                        u,
                    ),
                    ("!", Value::Bool(value)) => Value::Bool(!value),
                    _ => {
                        return Err(Diagnostic::new(
                            "E0201",
                            "단항 연산자의 값 타입을 확인한다.",
                            expr.span,
                        ));
                    }
                }
            }
            ExprKind::Binary(operator, lhs, rhs) => {
                let left = self.eval(lhs, env, static_only)?;
                let right = self.eval(rhs, env, static_only)?;
                arithmetic(operator, left, right, expr.span)?
            }
            ExprKind::Call(target, args) => {
                let ExprKind::Path(name) = &target.kind else {
                    return Err(Diagnostic::new(
                        "E0103",
                        "호출 대상은 등록한 함수 또는 graph 이름을 요구한다.",
                        target.span,
                    ));
                };
                if env.contains_key(name)
                    || self.globals.contains_key(name)
                    || self.program.bindings.iter().any(|b| b.name == *name)
                {
                    return Err(Diagnostic::new(
                        "E0201",
                        format!("값 {name}을 호출 대상으로 사용할 수 있는 함수 이름으로 바꾼다."),
                        target.span,
                    ));
                }
                if let Some(graph) = self
                    .program
                    .graphs
                    .iter()
                    .find(|g| g.name == *name)
                    .cloned()
                {
                    if static_only {
                        return Err(Diagnostic::new(
                            "E0211",
                            "graph 호출은 실행 그래프 본문에서 구체화한다.",
                            expr.span,
                        ));
                    }
                    self.invoke(&graph, args, env, expr.span)?
                } else {
                    let canonical = if let Some((prefix, tail)) = name.split_once('.') {
                        self.imports.get(prefix).map(|p| format!("{p}.{tail}"))
                    } else {
                        None
                    }
                    .ok_or_else(|| {
                        Diagnostic::new(
                            "E0103",
                            format!("호출 모듈을 import한다: {name}"),
                            target.span,
                        )
                    })?;
                    if let Some(spec) = self
                        .extensions
                        .iter()
                        .find(|s| s.name == canonical)
                        .cloned()
                    {
                        if static_only {
                            return Err(Diagnostic::new(
                                "E0211",
                                "사용자 연산자는 run 또는 graph 본문에 연결한다.",
                                expr.span,
                            ));
                        }
                        let values =
                            self.arguments(args, &[("input".into(), true)], env, false, expr.span)?;
                        self.extension(&spec, values, expr.span)?
                    } else {
                        let signature = registry::signature(&canonical).ok_or_else(|| {
                            Diagnostic::new(
                                "E0103",
                                format!("등록한 호출 서명을 사용한다: {canonical}"),
                                target.span,
                            )
                        })?;
                        if static_only && !signature.descriptor {
                            return Err(Diagnostic::new(
                                "E0211",
                                "실행 연산자는 run 또는 graph 본문에 연결한다.",
                                expr.span,
                            ));
                        }
                        let parameters = signature
                            .parameters
                            .iter()
                            .map(|p| (p.name.to_string(), p.required))
                            .collect::<Vec<_>>();
                        let args = self.arguments(
                            args,
                            &parameters,
                            env,
                            static_only || signature.descriptor,
                            expr.span,
                        )?;
                        self.builtin(signature, args, expr.span)?
                    }
                }
            }
        };
        if static_only && !value.streams().is_empty() {
            return Err(Diagnostic::new(
                "E0211",
                "구성값에는 정적 값과 기술자를 사용한다.",
                expr.span,
            ));
        }
        Ok(value)
    }

    pub(crate) fn arguments(
        &mut self,
        args: &[Argument],
        params: &[(String, bool)],
        env: &Environment,
        static_only: bool,
        span: Span,
    ) -> Result<Environment, Diagnostic> {
        let mut values = Environment::new();
        let mut position = 0;
        for arg in args {
            let name = if let Some(name) = &arg.name {
                name.clone()
            } else {
                let param = params.get(position).ok_or_else(|| {
                    Diagnostic::new("E0104", "위치 인수 수가 서명보다 많다.", arg.value.span)
                })?;
                position += 1;
                param.0.clone()
            };
            if !params.iter().any(|p| p.0 == name) || values.contains_key(&name) {
                return Err(Diagnostic::new(
                    "E0104",
                    format!("중복 또는 등록 범위 밖의 인수: {name}"),
                    arg.value.span,
                ));
            }
            values.insert(name, self.eval(&arg.value, env, static_only)?);
        }
        for (name, required) in params {
            if *required && !values.contains_key(name) {
                return Err(Diagnostic::new(
                    "E0104",
                    format!("필수 인수 {name}을 지정한다."),
                    span,
                ));
            }
        }
        Ok(values)
    }
}

fn arithmetic(operator: &str, left: Value, right: Value, span: Span) -> Result<Value, Diagnostic> {
    let err = |message| Diagnostic::new("E0501", message, span);
    match (left, right) {
        (Value::Number(a, u), Value::Number(b, v)) => {
            if !["*", "/"].contains(&operator) && u != v {
                return Err(Diagnostic::new("E0203", "상수 연산의 단위를 맞춘다.", span));
            }
            let value = match operator {
                "+" => Value::Number(a.checked_add(b).map_err(err)?, u),
                "-" => Value::Number(a.checked_add(b.negate().map_err(err)?).map_err(err)?, u),
                "*" => Value::Number(a.multiply(b).map_err(err)?, u.multiply(&v).map_err(err)?),
                "/" => Value::Number(
                    a.divide(b).map_err(err)?,
                    u.multiply(&v.power(-1).map_err(err)?).map_err(err)?,
                ),
                "==" => Value::Bool(a == b),
                "!=" => Value::Bool(a != b),
                "<" | "<=" | ">" | ">=" => {
                    let order = a.compare(b).map_err(err)?;
                    Value::Bool(match operator {
                        "<" => order.is_lt(),
                        "<=" => order.is_le(),
                        ">" => order.is_gt(),
                        _ => order.is_ge(),
                    })
                }
                _ => {
                    return Err(Diagnostic::new(
                        "E0201",
                        "수치에 적용하는 산술·비교 연산을 사용한다.",
                        span,
                    ));
                }
            };
            Ok(value)
        }
        (Value::Bool(a), Value::Bool(b)) => match operator {
            "&&" => Ok(Value::Bool(a && b)),
            "||" => Ok(Value::Bool(a || b)),
            "==" => Ok(Value::Bool(a == b)),
            "!=" => Ok(Value::Bool(a != b)),
            _ => Err(Diagnostic::new(
                "E0201",
                "bool에 논리·동등 연산을 적용한다.",
                span,
            )),
        },
        (Value::Text(a), Value::Text(b)) if operator == "==" || operator == "!=" => {
            Ok(Value::Bool(if operator == "==" { a == b } else { a != b }))
        }
        _ => Err(Diagnostic::new(
            "E0211",
            "상수 산술에 맞는 값 타입을 사용하고 stream 계산은 연산자로 연결한다.",
            span,
        )),
    }
}
