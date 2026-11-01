use crate::{
    Diagnostic,
    check::Checker,
    value::{Environment, Value},
};
use bci_artifact::descriptor;
use bci_schema::input::{GapPolicy, InvalidPolicy, SourceKind, SourceSpec, row_bound};
use bci_schema::{number::Rational, registry::Signature, types::*, unit::Unit};
use std::collections::{BTreeMap, BTreeSet};

struct Args {
    values: Environment,
    span: Span,
}
impl Args {
    fn get(&self, name: &str) -> Result<&Value, Diagnostic> {
        self.values
            .get(name)
            .ok_or_else(|| self.err("E0104", format!("필수 인수 {name}을 지정한다.")))
    }
    fn err(&self, code: &str, message: impl Into<String>) -> Diagnostic {
        Diagnostic::new(code, message, self.span)
    }
    fn string(&self, name: &str) -> Result<String, Diagnostic> {
        match self.get(name)? {
            Value::Text(s) => Ok(s.clone()),
            v => Err(self.err(
                "E0201",
                format!("{name}은 문자열을 요구한다: {}", v.describe()),
            )),
        }
    }
    fn number(&self, name: &str) -> Result<(Rational, Unit), Diagnostic> {
        match self.get(name)? {
            Value::Number(n, u) => Ok((*n, u.clone())),
            _ => Err(self.err("E0201", format!("{name}은 수치 인수를 요구한다."))),
        }
    }
    fn scalar(&self, name: &str) -> Result<Rational, Diagnostic> {
        let (n, u) = self.number(name)?;
        if u != Unit::scalar() {
            return Err(self.err("E0203", format!("{name}은 무차원 값을 요구한다.")));
        }
        Ok(n)
    }
    fn natural(&self, name: &str) -> Result<usize, Diagnostic> {
        let n = self
            .scalar(name)?
            .natural()
            .map_err(|e| self.err("E0501", e))?;
        if n == 0 || n > 65_536 {
            return Err(self.err("E0501", format!("{name}은 1..65536 범위를 요구한다.")));
        }
        Ok(n)
    }
    fn quantity(&self, name: &str, unit: &str, positive: bool) -> Result<Rational, Diagnostic> {
        let (n, u) = self.number(name)?;
        let target: Unit = unit.parse().map_err(|e| self.err("E0203", e))?;
        if !u.same_dimension(&target) {
            return Err(self.err("E0203", format!("{name}은 {unit} 차원을 요구한다.")));
        }
        let n = n
            .multiply(u.scale)
            .and_then(|n| n.divide(target.scale))
            .map_err(|e| self.err("E0501", e))?;
        if positive && !n.is_positive() {
            return Err(self.err("E0501", format!("{name}은 양수를 요구한다.")));
        }
        Ok(n)
    }
    fn schema(&self, name: &str) -> Result<ChannelSchema, Diagnostic> {
        match self.get(name)? {
            Value::Schema(s) => Ok(s.clone()),
            _ => Err(self.err("E0201", format!("{name}은 채널 schema를 요구한다."))),
        }
    }
    fn clock(&self, name: &str) -> Result<String, Diagnostic> {
        match self.get(name)? {
            Value::Clock(s) => Ok(s.clone()),
            _ => Err(self.err("E0201", format!("{name}은 clock 기술자를 요구한다."))),
        }
    }
    fn strings(&self, name: &str) -> Result<Vec<String>, Diagnostic> {
        match self.get(name)? {
            Value::Array(values) => values
                .iter()
                .map(|v| match v {
                    Value::Text(s) => Ok(s.clone()),
                    _ => Err(self.err("E0201", format!("{name}은 문자열 배열을 요구한다."))),
                })
                .collect(),
            _ => Err(self.err("E0201", format!("{name}은 배열을 요구한다."))),
        }
    }
    fn coefficients(&self, name: &str) -> Result<usize, Diagnostic> {
        let length = match self.get(name)? {
            Value::Coefficients(values) => values.len(),
            Value::Array(values) => {
                for value in values {
                    if !matches!(value,Value::Number(_,u) if *u==Unit::scalar()) {
                        return Err(self.err("E0203", "필터 계수는 무차원 수치 배열을 요구한다."));
                    }
                }
                values.len()
            }
            _ => {
                return Err(self.err(
                    "E0201",
                    format!("{name}은 계수 artifact 또는 배열을 요구한다."),
                ));
            }
        };
        if length == 0 || length > 65_536 {
            return Err(self.err("E0501", "계수 수는 1..65536 범위를 요구한다."));
        }
        Ok(length)
    }
    fn enumeration(&self, name: &str, allowed: &[&str]) -> Result<(), Diagnostic> {
        if let Some(value) = self.values.get(name)
            && !matches!(value,Value::Enum(v) if allowed.contains(&v.as_str()))
        {
            return Err(self.err(
                "E0501",
                format!("{name}은 {} 값을 요구한다.", allowed.join(", ")),
            ));
        }
        Ok(())
    }
}

impl Checker<'_> {
    pub(crate) fn extension(
        &mut self,
        spec: &bci_schema::extension::OperatorSpec,
        values: Environment,
        span: Span,
    ) -> Result<Value, Diagnostic> {
        let args = Args { values, span };
        let (input, output) = self.stream(args.get("input")?, span)?;
        if !matches!(output.value, ValueType::Vector(DType::F64, _)) {
            return Err(args.err(
                "E0201",
                format!("{}는 Vec<f64,C> 입력을 요구한다.", spec.name),
            ));
        }
        if !spec.accepts(&output) {
            return Err(args.err(
                "E0204",
                format!(
                    "{}의 모든 채널에 측정량 {}을 요구한다.",
                    spec.name, spec.quantity
                ),
            ));
        }
        let mut parameters = args.values;
        parameters.insert("extension".into(), Value::Extension(Box::new(spec.clone())));
        let id = self.node(
            &spec.name,
            vec![input],
            Some(output),
            parameters,
            Effect::new(Mode::Live),
            span,
        )?;
        Ok(Value::Stream(id))
    }

    pub(crate) fn builtin(
        &mut self,
        signature: &Signature,
        values: Environment,
        span: Span,
    ) -> Result<Value, Diagnostic> {
        let mut args = Args { values, span };
        let name = signature.name;
        if signature.descriptor {
            return self.descriptor(name, &args);
        }
        if name == "std.runtime.configure" {
            if self.in_template || self.configured || !self.graph.nodes.is_empty() {
                return Err(args.err(
                    "E0302",
                    "runtime.configure는 run의 첫 연산자로 한 번 호출한다.",
                ));
            }
            args.enumeration("mode", &["Mode.Live", "Mode.Record"])?;
            if let Some(resource) = args.values.get("resources")
                && !matches!(resource, Value::Resource(_))
            {
                return Err(args.err("E0201", "resources에 resource 기술자를 지정한다."));
            }
            self.limit = if matches!(args.get("mode")?,Value::Enum(s) if s=="Mode.Record") {
                Mode::Record
            } else {
                Mode::Live
            };
            self.configured = true;
            self.graph.configuration = args.values.clone();
            return Ok(Value::Void);
        }
        let mut effect = Effect::new(signature.mode);
        if name == "std.source.replay" {
            args.enumeration("gap", &["Gap.Reset", "Gap.Fault"])?;
            let path = std::path::absolute(self.base.join(args.string("path")?))
                .map_err(|e| args.err("E0902", e.to_string()))?;
            let port = args
                .scalar("port")?
                .natural()
                .map_err(|e| args.err("E0501", e))?;
            let mut spec = bci_artifact::input::inspect_replay(&path, port)
                .map_err(|e| args.err(e.code, e.message))?;
            if let SourceKind::Replay { gap, .. } = &mut spec.kind {
                *gap = if matches!(args.values.get("gap"),Some(Value::Enum(v)) if v=="Gap.Fault") {
                    GapPolicy::Fault
                } else {
                    GapPolicy::Reset
                };
            }
            if args.values.contains_key("expect")
                && args.schema("expect")?.channels != spec.stream.schema.channels
            {
                return Err(args.err("E0204", "재생 포트의 채널·단위·기준을 expect와 맞춘다."));
            }
            if args.values.contains_key("clock") && args.clock("clock")? != spec.stream.clock {
                return Err(args.err("E0401", "재생 포트의 원래 clock ID를 사용한다."));
            }
            if args.values.contains_key("rate") {
                let rate = match spec.stream.occurrence {
                    Occurrence::Uniform { rate, .. } | Occurrence::PendingUniform { rate, .. } => {
                        rate
                    }
                    _ => return Err(args.err("E0205", "재생 포트에 Uniform 격자를 요구한다.")),
                };
                if rate != args.quantity("rate", "Hz", true)? {
                    return Err(args.err("E0205", "재생 포트의 발생률을 rate와 맞춘다."));
                }
            }
            let output = spec.stream.clone();
            let mut values = args.values.clone();
            values.insert("source".into(), Value::Source(Box::new(spec)));
            let id = self.node(name, Vec::new(), Some(output), values, effect, span)?;
            self.obligation(
                id,
                "Input",
                "기록 header·포트·시각·품질과 EOF commit 관측값을 검사한다.",
                span,
            );
            return Ok(Value::Stream(id));
        }
        if matches!(
            name,
            "std.source.synthetic" | "std.source.csv" | "std.source.acquire"
        ) {
            let schema = args.schema(if name != "std.source.acquire" {
                "schema"
            } else {
                "expect"
            })?;
            let rate = args.quantity("rate", "Hz", true)?;
            let mut binding = None;
            let clock = if name != "std.source.acquire" {
                args.clock("clock")?
            } else {
                let Value::Device {
                    identity,
                    profile,
                    config,
                } = args.get("device")?
                else {
                    return Err(args.err("E0201", "source.acquire에 device 기술자를 지정한다."));
                };
                if schema
                    .channels
                    .iter()
                    .any(|channel| channel.quantity != "electrical.voltage")
                {
                    return Err(args.err(
                        "E0204",
                        "현재 acquire 계약은 electrical.voltage 채널 묶음을 요구한다.",
                    ));
                }
                if let Some(channels) = config.get("channels")
                    && !matches!(channels, Value::Number(n,u) if *u == Unit::scalar() && n.natural().ok() == Some(schema.channels.len()))
                {
                    return Err(args.err("E0204", "장치 설정의 channels와 기대 schema를 맞춘다."));
                }
                if let Some(value) = config.get("output_rate") {
                    let mut v = BTreeMap::new();
                    v.insert("rate".into(), value.clone());
                    if (Args { values: v, span }).quantity("rate", "Hz", true)? != rate {
                        return Err(
                            args.err("E0205", "장치 설정 output_rate와 source의 rate를 맞춘다.")
                        );
                    }
                }
                effect.capabilities.insert("AcquireElectrical".into());
                binding = Some(profile.clone());
                format!("{identity}.clock")
            };
            let phase = if args.values.contains_key("phase") {
                args.quantity("phase", "s", false)?
            } else {
                Rational::ZERO
            };
            let output = StreamType {
                value: ValueType::Vector(DType::F64, schema.channels.len()),
                schema,
                clock,
                occurrence: Occurrence::Uniform { rate, phase },
                quality: "StandardQuality".into(),
            };
            let mut values = args.values.clone();
            if name != "std.source.acquire" {
                let kind = if name == "std.source.csv" {
                    args.enumeration("gap", &["Gap.Reset", "Gap.Fault"])?;
                    args.enumeration("invalid", &["Invalid.Mark", "Invalid.Fault"])?;
                    SourceKind::Csv {
                        path: std::path::absolute(self.base.join(args.string("path")?))
                            .map_err(|e| args.err("E0902", e.to_string()))?,
                        gap: if matches!(args.values.get("gap"),Some(Value::Enum(v)) if v=="Gap.Fault")
                        {
                            GapPolicy::Fault
                        } else {
                            GapPolicy::Reset
                        },
                        invalid: if matches!(args.values.get("invalid"),Some(Value::Enum(v)) if v=="Invalid.Mark")
                        {
                            InvalidPolicy::Mark
                        } else {
                            InvalidPolicy::Fault
                        },
                    }
                } else {
                    SourceKind::Synthetic
                };
                let c = output.schema.channels.len();
                let spec = SourceSpec {
                    row_bytes: if kind == SourceKind::Synthetic {
                        0
                    } else {
                        row_bound(c, c).map_err(|e| args.err("E0801", e))?
                    },
                    kind,
                    stream: output.clone(),
                };
                if name == "std.source.csv" {
                    bci_artifact::input::inspect_csv(&spec)
                        .map_err(|e| args.err(e.code, e.message))?;
                }
                values.insert("source".into(), Value::Source(Box::new(spec)));
            }
            let id = self.node(name, Vec::new(), Some(output), values, effect, span)?;
            if name == "std.source.csv" {
                self.obligation(
                    id,
                    "Input",
                    "CSV header·시간 격자·Gap·무효 값 정책을 검사한다.",
                    span,
                );
            }
            if let Some(profile) = binding {
                self.obligation(
                    id,
                    "Binding",
                    format!("프로필 {profile}의 실제 schema·rate·기능·접근 증거를 조회한다."),
                    span,
                );
            }
            return Ok(Value::Stream(id));
        }
        if name == "std.signal.mix" {
            let Value::Tuple(values) = args.get("input")? else {
                return Err(args.err("E0201", "mix 입력은 Stream 튜플을 요구한다."));
            };
            if !(2..=64).contains(&values.len()) {
                return Err(args.err("E0501", "mix 입력 포트 수는 2..64 범위를 요구한다."));
            }
            let mut ports = Vec::new();
            let (_, output) = self.stream(&values[0], span)?;
            if !matches!(output.value, ValueType::Vector(DType::F64, _)) {
                return Err(args.err("E0201", "mix는 Vec<f64,C> 입력을 요구한다."));
            }
            for value in values {
                let (id, ty) = self.stream(value, span)?;
                if let Some((code, message)) = ty.mismatch(&output) {
                    return Err(args.err(code, message));
                }
                ports.push(id);
            }
            let Value::Array(weights) = args.get("weights")? else {
                return Err(args.err("E0201", "mix weights는 무차원 수치 배열을 요구한다."));
            };
            if weights.len() != ports.len() {
                return Err(args.err("E0501", "입력마다 weight를 하나 지정한다."));
            }
            for weight in weights {
                match weight {
                    Value::Number(n, u) if *u == Unit::scalar() && n.to_f64().is_finite() => {}
                    _ => return Err(args.err("E0203", "mix weight는 유한 무차원 수치를 요구한다.")),
                }
            }
            let id = self.node(name, ports, Some(output), args.values.clone(), effect, span)?;
            return Ok(Value::Stream(id));
        }
        if name == "std.recording.write" {
            let input = args.get("input")?;
            let ids = input.streams();
            fn all_streams(value: &Value) -> bool {
                match value {
                    Value::Stream(_) => true,
                    Value::Tuple(values) => !values.is_empty() && values.iter().all(all_streams),
                    _ => false,
                }
            }
            if ids.is_empty() || !all_streams(input) {
                return Err(args.err(
                    "E0201",
                    "recording.write는 Stream 또는 Stream 튜플을 요구한다.",
                ));
            }
            let destination = args.string("to")?;
            if destination.is_empty() {
                return Err(args.err("E0501", "기록 경로에 비어 있지 않은 문자열을 지정한다."));
            }
            effect.writes.insert(destination.clone());
            let id = self.node(name, ids, None, args.values.clone(), effect, span)?;
            self.obligation(
                id,
                "Storage",
                format!("기록 경로 {destination}의 용량·처리율·열기 조건을 확인한다."),
                span,
            );
            return Ok(Value::Void);
        }
        let (input, mut output) = self.stream(args.get("input")?, span)?;
        let mut pending = Vec::new();
        if matches!(output.value, ValueType::Spectrum(..)) && name != "std.signal.bandpower" {
            return Err(args.err("E0201", "Spectrum의 전력 특징은 bandpower로 추출한다. 채널·단위·시계 변환은 PSD 입력에 연결한다."));
        }
        if matches!(output.value, ValueType::Probability(_)) && name != "std.decision.threshold"
            || matches!(output.value, ValueType::Decision(_))
        {
            return Err(args.err(
                "E0201",
                "Probability는 decision.threshold에, Decision은 기록 sink에 연결한다.",
            ));
        }
        match name {
            "std.model.predict" => {
                let Value::Model(model) = args.get("using")? else {
                    return Err(args.err("E0703", "using에 model.load 기술자를 지정한다."));
                };
                output = model.output(&output).map_err(|e| args.err("E0703", e))?;
            }
            "std.decision.threshold" => {
                output = bci_schema::learning::decision(&output, args.scalar("minimum")?.to_f64())
                    .map_err(|e| args.err("E0703", e))?;
            }
            "std.signal.psd" => {
                args.enumeration("normalization", &["Psd.OneSidedDensity"])?;
                let taper = match args.get("taper")? {
                    Value::Enum(v) if v == "Taper.Rectangular" => {
                        bci_schema::spectral::Taper::Rectangular
                    }
                    Value::Enum(v) if v == "Taper.Hann" => bci_schema::spectral::Taper::Hann,
                    _ => {
                        return Err(args.err(
                            "E0501",
                            "PSD taper는 Taper.Rectangular 또는 Taper.Hann을 요구한다.",
                        ));
                    }
                };
                output =
                    bci_schema::spectral::psd(&output, taper).map_err(|e| args.err("E0501", e))?;
            }
            "std.signal.bandpower" => {
                let Value::Record(values) = args.get("band")? else {
                    return Err(args.err("E0201", "band는 low·high Hz 레코드를 요구한다."));
                };
                if values.len() != 2 || !values.contains_key("low") || !values.contains_key("high")
                {
                    return Err(args.err("E0501", "band의 필드는 low·high를 지정한다."));
                }
                let bounds = Args {
                    values: values.clone(),
                    span,
                };
                let low = bounds.quantity("low", "Hz", false)?;
                let high = bounds.quantity("high", "Hz", false)?;
                let include_high = match args.values.get("include_high") {
                    None => false,
                    Some(Value::Bool(v)) => *v,
                    _ => return Err(args.err("E0201", "include_high는 bool을 요구한다.")),
                };
                output = bci_schema::spectral::bandpower(
                    &output,
                    &bci_schema::spectral::Band {
                        low,
                        high,
                        include_high,
                    },
                )
                .map_err(|e| args.err("E0501", e))?;
                args.values
                    .insert("low_hz".into(), Value::Number(low, Unit::scalar()));
                args.values
                    .insert("high_hz".into(), Value::Number(high, Unit::scalar()));
                args.values
                    .insert("include_high".into(), Value::Bool(include_high));
            }
            "std.signal.log_power" => {
                let (floor, unit) = args.number("floor")?;
                let (reference, reference_unit) = args.number("reference")?;
                if unit != reference_unit {
                    return Err(args.err("E0203", "floor·reference의 단위 배율을 맞춘다."));
                }
                output = bci_schema::spectral::log_power(&output, floor, reference, &unit)
                    .map_err(|e| args.err("E0501", e))?;
            }
            "std.signal.convert_unit" => {
                require_float(&output, &args)?;
                let unit: Unit = args
                    .string("to")?
                    .parse()
                    .map_err(|e| args.err("E0203", e))?;
                for channel in &mut output.schema.channels {
                    if !channel.unit.same_dimension(&unit) {
                        return Err(args.err("E0203", "단위 변환의 입력·출력 물리 차원을 맞춘다."));
                    }
                    channel.unit = unit.clone();
                }
            }
            "std.signal.scale" => {
                args.scalar("factor")?;
                require_float(&output, &args)?;
            }
            "std.signal.iir" => {
                let Value::Iir(realization) = args.get("realization")? else {
                    return Err(
                        args.err("E0201", "IIR realization에 signal.sos 기술자를 지정한다.")
                    );
                };
                realization
                    .accepts(&output)
                    .map_err(|e| args.err("E0501", e))?;
            }
            "std.signal.fir" | "std.signal.filtfilt" => {
                require_vector(&output, &args)?;
                require_float(&output, &args)?;
                rate_of(&output, &args)?;
                let taps = args.coefficients("coeffs")?;
                args.enumeration("warmup", &["Warmup.Full"])?;
                if name.ends_with("filtfilt") {
                    pending.push((
                        "RecordEnd",
                        format!("기록 구간·종료 사건과 {taps}계수 필터의 경계 초기값을 확인한다."),
                    ));
                }
            }
            "std.signal.window" => {
                let (dtype, count) = require_vector(&output, &args)?;
                let rate = rate_of(&output, &args)?;
                let size = args.natural("size")?;
                let hop = args.natural("hop")?;
                args.enumeration("anchor", &["Anchor.End"])?;
                args.enumeration("quality", &["Quality.RequireAll"])?;
                args.enumeration("tail", &["Tail.Discard"])?;
                let out_rate = rate
                    .divide(Rational::integer(hop as i64))
                    .map_err(|e| args.err("E0501", e))?;
                let phase = match args.values.get("start") {
                    Some(Value::Number(index, unit)) if *unit == Unit::scalar() => {
                        let start = index.natural().map_err(|e| args.err("E0501", e))?;
                        let last = start
                            .checked_add(size - 1)
                            .and_then(|n| i64::try_from(n).ok())
                            .ok_or_else(|| args.err("E0501", "창 index 범위를 초과했다."))?;
                        let Occurrence::Uniform { phase, .. } = &output.occurrence else {
                            return Err(
                                args.err("E0205", "고정 index 창은 확정한 입력 위상을 요구한다.")
                            );
                        };
                        Occurrence::Uniform {
                            rate: out_rate,
                            phase: phase
                                .checked_add(
                                    Rational::integer(last)
                                        .divide(rate)
                                        .map_err(|e| args.err("E0501", e))?,
                                )
                                .map_err(|e| args.err("E0501", e))?,
                        }
                    }
                    None | Some(Value::Enum(_)) => {
                        args.enumeration("start", &["WindowStart.FirstValid"])?;
                        let key = format!("{}:window{}", self.graph.entry, self.graph.nodes.len());
                        pending.push((
                            "WindowPhase",
                            format!("{key}: 첫 연속 {size}개 valid 표본으로 시작 위상을 확정한다."),
                        ));
                        Occurrence::PendingUniform {
                            rate: out_rate,
                            phase_key: key,
                        }
                    }
                    _ => {
                        return Err(args.err(
                            "E0201",
                            "start는 정수 index 또는 WindowStart.FirstValid를 요구한다.",
                        ));
                    }
                };
                output.value = ValueType::Epoch(dtype, count, size, rate);
                output.occurrence = phase;
            }
            "std.signal.covariance" => {
                require_float(&output, &args)?;
                let ValueType::Epoch(dtype, count, size, _) = output.value.clone() else {
                    return Err(args.err("E0201", "covariance는 Epoch 입력을 요구한다."));
                };
                if size < 2 {
                    return Err(args.err("E0501", "공분산의 표본 수는 2 이상이어야 한다."));
                }
                let first = &output.schema.channels[0];
                if output
                    .schema
                    .channels
                    .iter()
                    .any(|c| c.unit != first.unit || c.quantity != first.quantity)
                {
                    return Err(args.err("E0203", "공분산 입력의 측정량과 단위를 맞춘다."));
                }
                for channel in &mut output.schema.channels {
                    channel.unit = channel.unit.power(2).map_err(|e| args.err("E0203", e))?;
                    channel.quantity = format!("covariance({})", channel.quantity);
                }
                output.value = ValueType::Covariance(dtype, count, false);
            }
            "std.signal.regularize" => {
                let ValueType::Covariance(dtype, count, false) = output.value.clone() else {
                    return Err(args.err("E0201", "regularize는 Cov 입력을 요구한다."));
                };
                let lambda = args.scalar("shrinkage")?;
                if !lambda.is_positive()
                    || lambda
                        .compare(Rational::ONE)
                        .map_err(|e| args.err("E0501", e))?
                        .is_gt()
                {
                    return Err(args.err("E0501", "shrinkage는 0 초과 1 이하를 요구한다."));
                }
                let (floor, unit) = args.number("floor")?;
                if !floor.is_positive() {
                    return Err(args.err("E0501", "공분산 floor는 양수를 요구한다."));
                }
                if output.schema.channels.iter().any(|c| c.unit != unit) {
                    return Err(args.err("E0203", "floor와 공분산의 단위를 맞춘다."));
                }
                output.value = ValueType::Covariance(dtype, count, true);
                pending.push((
                    "NumericGuard",
                    "출력 Cholesky 검사 통과 후 SPD refinement를 게시한다.".into(),
                ));
            }
            "std.signal.resample" => {
                let Occurrence::Uniform {
                    rate,
                    phase: input_phase,
                } = output.occurrence
                else {
                    return Err(args.err("E0205", "재표본화 입력에 확정 Uniform 격자를 연결한다."));
                };
                args.coefficients("filter")?;
                let coefficients = match args.get("filter")? {
                    Value::Coefficients(v) => v.iter().map(|n| n.to_f64()).collect(),
                    Value::Array(v) => v
                        .iter()
                        .map(|v| match v {
                            Value::Number(n, _) => n.to_f64(),
                            _ => unreachable!(),
                        })
                        .collect(),
                    _ => unreachable!(),
                };
                let specification = bci_schema::resample::Resampling {
                    up: args.natural("up")?,
                    down: args.natural("down")?,
                    coefficients,
                    rate,
                    input_phase,
                    phase: args.quantity("phase", "s", false)?,
                };
                output = specification
                    .output(&output)
                    .map_err(|e| args.err("E0501", e))?;
                pending.push((
                    "Resampling",
                    "인과 polyphase FIR의 정확한 격자·support·초기화 경계를 실행에서 검사한다."
                        .into(),
                ));
            }

            "std.signal.rereference" => {
                let weights = match args.get("weights")? {
                    Value::Array(values) => values
                        .iter()
                        .map(|v| match v {
                            Value::Number(n, u) if *u == Unit::scalar() => Ok(*n),
                            _ => Err(args
                                .err("E0203", "재기준화 weights는 무차원 수치 배열을 요구한다.")),
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                    _ => return Err(args.err("E0201", "재기준화 weights 배열을 지정한다.")),
                };
                output = bci_schema::spatial::rereference(&output, &weights)
                    .map_err(|e| args.err("E0501", e))?;
            }
            "std.signal.select" | "std.signal.reorder" => {
                bci_schema::spatial::layout(&output.value).map_err(|e| args.err("E0201", e))?;
                let indices = args
                    .strings("channels")?
                    .iter()
                    .map(|id| {
                        output
                            .schema
                            .channels
                            .iter()
                            .position(|c| c.id == *id)
                            .ok_or_else(|| args.err("E0204", format!("입력 채널 {id}를 확인한다.")))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                output = bci_schema::spatial::select(&output, &indices, name.ends_with("reorder"))
                    .map_err(|e| args.err("E0204", e))?;
            }
            "std.source.assert_schema" => {
                let schema = args.schema("expect")?;
                let rate = args.quantity("rate", "Hz", true)?;
                let mut expected = output.clone();
                expected.schema = schema;
                if let Some((code, message)) = output.mismatch(&expected) {
                    return Err(args.err(code, message));
                }
                if rate_of(&output, &args)? != rate {
                    return Err(args.err(
                        "E0205",
                        format!("발생률 불일치: 입력 {}; 요구 {rate}Hz.", output.occurrence),
                    ));
                }
            }
            "std.time.align" => {
                let target = args.clock("to")?;
                let Value::Mapping {
                    path,
                    from,
                    to,
                    slope,
                    offset,
                } = args.get("mapping")?
                else {
                    return Err(args.err("E0201", "mapping 기술자를 지정한다."));
                };
                if output.clock != *from || target != *to {
                    return Err(args.err(
                        "E0401",
                        format!(
                            "mapping 양 끝 {from} -> {to}와 포트 {} -> {target}를 맞춘다.",
                            output.clock
                        ),
                    ));
                }
                output.clock = target;
                output.occurrence = match &output.occurrence {
                    Occurrence::Uniform { rate, phase } => Occurrence::Uniform {
                        rate: rate.divide(*slope).map_err(|e| args.err("E0501", e))?,
                        phase: phase
                            .multiply(*slope)
                            .and_then(|p| p.checked_add(*offset))
                            .map_err(|e| args.err("E0501", e))?,
                    },
                    Occurrence::PendingUniform { rate, phase_key } => Occurrence::PendingUniform {
                        rate: rate.divide(*slope).map_err(|e| args.err("E0501", e))?,
                        phase_key: format!("{phase_key}@{path}"),
                    },
                    Occurrence::Events => Occurrence::Events,
                };
                if let ValueType::Epoch(_, _, _, rate) = &mut output.value {
                    *rate = rate.divide(*slope).map_err(|e| args.err("E0501", e))?;
                }
                pending.push((
                    "ClockMapping",
                    format!("{path}: epoch·가용 시각·유효 구간·오차 증거를 검증한다."),
                ));
            }
            _ => return Err(args.err("E0103", format!("의미 규칙을 등록한다: {name}"))),
        }
        let id = self.node(
            name,
            vec![input],
            Some(output),
            args.values.clone(),
            effect,
            span,
        )?;
        for (kind, predicate) in pending {
            self.obligation(id, kind, predicate, span);
        }
        Ok(Value::Stream(id))
    }

    fn descriptor(&mut self, name: &str, args: &Args) -> Result<Value, Diagnostic> {
        match name {
            "std.schema.electrical" => {
                let ids = args.strings("channels")?;
                let unique: BTreeSet<_> = ids.iter().collect();
                if ids.is_empty()
                    || ids.len() > 65_536
                    || ids.iter().any(String::is_empty)
                    || unique.len() != ids.len()
                {
                    return Err(args.err(
                        "E0204",
                        "채널은 고유한 비어 있지 않은 ID를 사용하고 수는 1..65536이어야 한다.",
                    ));
                }
                let unit: Unit = args
                    .string("unit")?
                    .parse()
                    .map_err(|e| args.err("E0203", e))?;
                if !unit.same_dimension(&"V".parse().map_err(|e| args.err("E0203", e))?) {
                    return Err(args.err("E0203", "electrical schema는 전압 단위를 요구한다."));
                }
                let reference = args.string("reference")?;
                if reference.is_empty() {
                    return Err(args.err("E0204", "전압 기준 ID를 지정한다."));
                }
                let name = if args.values.contains_key("name") {
                    args.string("name")?
                } else {
                    self.identity(args.span)
                };
                Ok(Value::Schema(ChannelSchema {
                    name,
                    channels: ids
                        .into_iter()
                        .map(|id| Channel {
                            id,
                            quantity: "electrical.voltage".into(),
                            unit: unit.clone(),
                            reference: reference.clone(),
                        })
                        .collect(),
                }))
            }
            "std.schema.load" => Ok(Value::Schema(
                descriptor::load_schema(&self.base.join(args.string("path")?))
                    .map_err(|e| args.err("E0902", e))?,
            )),
            "std.time.session_clock" => Ok(Value::Clock(if args.values.contains_key("name") {
                args.string("name")?
            } else {
                self.identity(args.span)
            })),
            "std.time.uniform" => Ok(Value::Rate(Occurrence::Uniform {
                rate: args.quantity("rate", "Hz", true)?,
                phase: args.quantity("phase", "s", false)?,
            })),
            "std.time.device_clock" => {
                let Value::Device { identity, .. } = args.get("device")? else {
                    return Err(args.err("E0201", "device 기술자를 지정한다."));
                };
                Ok(Value::Clock(format!("{identity}.clock")))
            }
            "std.device.bind" => {
                let profile = args.string("profile")?;
                let Value::Record(config) = args.get("config")? else {
                    return Err(args.err("E0201", "config에 설정 레코드를 지정한다."));
                };
                if profile.is_empty() {
                    return Err(args.err("E0501", "profile 경로를 지정한다."));
                }
                Ok(Value::Device {
                    identity: self.identity(args.span),
                    profile,
                    config: config.clone(),
                })
            }
            "std.time.mapping" => {
                let path = args.string("path")?;
                let mapping = descriptor::load_mapping(&self.base.join(&path))
                    .map_err(|e| args.err("E0902", e))?;
                Ok(Value::Mapping {
                    path,
                    from: mapping.from,
                    to: mapping.to,
                    slope: mapping.slope,
                    offset: mapping.offset,
                })
            }
            "std.model.load" => Ok(Value::Model(Box::new(
                bci_artifact::learning::load_model(&self.base.join(args.string("path")?))
                    .map_err(|e| args.err("E0703", e))?,
            ))),
            "std.signal.sos" => Ok(Value::Iir(Box::new(
                descriptor::load_sos(&self.base.join(args.string("path")?))
                    .map_err(|e| args.err("E0501", e))?,
            ))),
            "std.signal.coefficients" => Ok(Value::Coefficients(
                descriptor::load_coefficients(&self.base.join(args.string("path")?))
                    .map_err(|e| args.err("E0902", e))?,
            )),
            "std.runtime.budget" => {
                args.quantity("memory", "byte", true)?;
                args.enumeration("queue_policy", &["Queue.Fault", "Queue.DropOldest"])?;
                if args.values.contains_key("profile") {
                    args.string("profile")?;
                }
                Ok(Value::Resource(args.values.clone()))
            }
            _ => Err(args.err("E0103", format!("구성 생성자를 등록한다: {name}"))),
        }
    }
}

fn require_vector(output: &StreamType, args: &Args) -> Result<(DType, usize), Diagnostic> {
    match &output.value {
        ValueType::Vector(dtype, count) => Ok((dtype.clone(), *count)),
        _ => Err(args.err("E0201", "표본 벡터 Vec 입력을 요구한다.")),
    }
}
fn require_float(output: &StreamType, args: &Args) -> Result<(), Diagnostic> {
    let dtype = match &output.value {
        ValueType::Probability(_) | ValueType::Decision(_) => &DType::F64,
        ValueType::Vector(d, _)
        | ValueType::Epoch(d, _, _, _)
        | ValueType::Spectrum(d, ..)
        | ValueType::Covariance(d, _, _) => d,
    };
    if !matches!(dtype, DType::F32 | DType::F64) {
        return Err(args.err("E0201", "f32 또는 f64 수치 입력을 요구한다."));
    }
    Ok(())
}
fn rate_of(output: &StreamType, args: &Args) -> Result<Rational, Diagnostic> {
    match output.occurrence {
        Occurrence::Uniform { rate, .. } | Occurrence::PendingUniform { rate, .. } => Ok(rate),
        _ => Err(args.err("E0205", "균일 발생 규칙을 요구한다.")),
    }
}
