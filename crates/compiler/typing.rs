use crate::{
    Diagnostic,
    check::Checker,
    syntax::TypeExpr,
    value::{Environment, Value},
};
use bci_schema::{types::*, unit::Unit};

impl Checker<'_> {
    pub(crate) fn annotation(&mut self, value: &Value, ty: &TypeExpr) -> Result<(), Diagnostic> {
        if ty.name == "Product" {
            if let Value::Tuple(values) = value
                && values.len() == ty.args.len()
            {
                for (v, t) in values.iter().zip(&ty.args) {
                    self.annotation(v, t)?;
                }
                return Ok(());
            }
        } else if let Value::Stream(id) = value {
            let actual = self.graph.nodes[*id].output.clone().expect("stream output");
            let expected = self.stream_type(ty, Some(&actual))?;
            if let Some((code, message)) = actual.mismatch(&expected) {
                return Err(Diagnostic::new(code, message, ty.span));
            }
            return Ok(());
        } else if matches!(
            (ty.name.as_str(), value),
            ("bool", Value::Bool(_))
                | ("string", Value::Text(_))
                | ("f32" | "f64", Value::Number(_, _))
        ) && ty.args.is_empty()
        {
            return Ok(());
        }
        Err(Diagnostic::new(
            "E0201",
            format!(
                "타입 {}과 값 {}의 kind를 맞춘다.",
                ty.name,
                value.describe()
            ),
            ty.span,
        ))
    }

    pub(crate) fn stream_type(
        &mut self,
        ty: &TypeExpr,
        actual: Option<&StreamType>,
    ) -> Result<StreamType, Diagnostic> {
        let error = || {
            Diagnostic::new(
                "E0201",
                "Stream<Value,Schema,Clock,Rate,Quality> 타입을 요구한다.",
                ty.span,
            )
        };
        if ty.name != "Stream" || ty.args.len() != 5 {
            return Err(error());
        }
        let args = &ty.args;
        if args[1..]
            .iter()
            .any(|arg| !arg.args.is_empty() || arg.literal.is_some())
        {
            return Err(error());
        }
        let schema = match self.lookup(&args[1].name, &Environment::new(), args[1].span)? {
            Value::Schema(s) => s,
            _ => return Err(error()),
        };
        let clock = match self.lookup(&args[2].name, &Environment::new(), args[2].span)? {
            Value::Clock(c) => c,
            _ => return Err(error()),
        };
        let occurrence = if args[3].name == "Events" {
            Occurrence::Events
        } else {
            match self.lookup(&args[3].name, &Environment::new(), args[3].span)? {
                Value::Rate(r) => r,
                _ => return Err(error()),
            }
        };
        if args[4].name != "StandardQuality" || !args[4].args.is_empty() {
            return Err(Diagnostic::new(
                "E0206",
                "등록한 품질 어휘 StandardQuality를 사용한다.",
                args[4].span,
            ));
        }
        let shape = &args[0];
        if shape.args.len() < 2 {
            return Err(error());
        }
        if !shape.args[0].args.is_empty() || shape.args[0].literal.is_some() {
            return Err(error());
        }
        let dtype = match shape.args[0].name.as_str() {
            "f32" => DType::F32,
            "f64" => DType::F64,
            "i16" => DType::I16,
            "i32" => DType::I32,
            "bool" => DType::Bool,
            _ => return Err(error()),
        };
        let count = self.natural_type(&shape.args[1])?;
        if count != schema.channels.len() {
            return Err(Diagnostic::new(
                "E0204",
                "값의 채널 수와 schema의 채널 수를 맞춘다.",
                ty.span,
            ));
        }
        let value = match shape.name.as_str() {
            "Vec" if shape.args.len() == 2 => ValueType::Vector(dtype, count),
            "Cov" | "SPD" if shape.args.len() == 2 => {
                ValueType::Covariance(dtype, count, shape.name == "SPD")
            }
            "Epoch" | "Spectrum" if shape.args.len() == 3 || shape.args.len() == 4 => {
                let n = self.natural_type(&shape.args[2])?;
                let rate = if let Some(t) = shape.args.get(3) {
                    let (value, unit) = t.literal.as_ref().ok_or_else(error)?;
                    let hz: Unit = "Hz".parse().map_err(|_| error())?;
                    if !unit.same_dimension(&hz) || !value.is_positive() {
                        return Err(error());
                    }
                    value
                        .multiply(unit.scale)
                        .map_err(|e| Diagnostic::new("E0501", e, t.span))?
                } else if let Some(StreamType {
                    value: ValueType::Epoch(_, _, _, rate) | ValueType::Spectrum(_, _, _, rate),
                    ..
                }) = actual
                {
                    *rate
                } else {
                    return Err(Diagnostic::new(
                        "E0201",
                        format!(
                            "{} 매개변수에 내부 표본률을 네 번째 인수로 지정한다: {}<f64,C,N,250[Hz]>.",
                            shape.name, shape.name
                        ),
                        ty.span,
                    ));
                };
                if shape.name == "Spectrum" {
                    ValueType::Spectrum(dtype, count, n, rate)
                } else {
                    ValueType::Epoch(dtype, count, n, rate)
                }
            }
            _ => return Err(error()),
        };
        Ok(StreamType {
            value,
            schema,
            clock,
            occurrence,
            quality: "StandardQuality".into(),
        })
    }

    fn natural_type(&mut self, ty: &TypeExpr) -> Result<usize, Diagnostic> {
        let value = if let Some((number, unit)) = &ty.literal {
            Value::Number(*number, unit.clone())
        } else if ty.args.is_empty() {
            self.lookup(&ty.name, &Environment::new(), ty.span)?
        } else {
            return Err(Diagnostic::new(
                "E0201",
                "타입 크기는 정수 상수를 요구한다.",
                ty.span,
            ));
        };
        if let Value::Number(number, unit) = value
            && unit == Unit::scalar()
        {
            let n = number
                .natural()
                .map_err(|e| Diagnostic::new("E0501", e, ty.span))?;
            if (1..=65_536).contains(&n) {
                return Ok(n);
            }
        }
        Err(Diagnostic::new(
            "E0501",
            "타입 크기는 1..65536 범위의 무차원 정수 상수를 요구한다.",
            ty.span,
        ))
    }
}
