use crate::{Compilation, Diagnostic};
use bci_ir::{
    graph::TypedNode,
    plan::{ExecutionPlan, PlannedNode},
    value::ArgumentValue as Value,
};
use bci_schema::{
    input::SourceKind,
    kernel::KernelSpec,
    types::{DType, Occurrence, Span, ValueType},
    unit::Unit,
};

pub(crate) fn lower(
    compilation: &Compilation,
    entry: Option<&str>,
    samples: Option<u64>,
) -> Result<ExecutionPlan, Diagnostic> {
    let matching: Vec<_> = compilation
        .graphs
        .iter()
        .filter(|g| entry.is_none_or(|e| g.entry == e || g.entry.rsplit('.').next() == Some(e)))
        .collect();
    if matching.len() != 1 {
        return Err(Diagnostic::new(
            "E0801",
            "run을 하나 선택한다. --entry에 선언 이름 또는 모듈 전체 이름을 지정한다.",
            Span::default(),
        ));
    }
    if samples.is_some_and(|n| !(1..=1_000_000_000).contains(&n)) {
        return Err(Diagnostic::new(
            "E0801",
            "--samples는 1..1000000000 범위의 정수를 요구한다.",
            Span::default(),
        ));
    }
    let graph = matching[0];
    let mut budget = 16 * 1024 * 1024;
    if let Some(Value::Resource(resource)) = graph.configuration.get("resources") {
        if let Some(Value::Number(number, unit)) = resource.get("memory") {
            budget = number
                .multiply(unit.scale)
                .and_then(|n| n.natural())
                .map_err(|e| Diagnostic::new("E0801", e, Span::default()))?;
        }
        if !matches!(resource.get("queue_policy"), Some(Value::Enum(value)) if value == "Queue.Fault")
        {
            return Err(Diagnostic::new(
                "E0801",
                "순차 실행의 자원 초과 처리는 Queue.Fault를 사용한다.",
                Span::default(),
            ));
        }
    }
    let mut nodes: Vec<PlannedNode> = Vec::new();
    let mut sources = Vec::new();
    let mut reserved = 0usize;
    for node in &graph.nodes {
        let error = |message| Diagnostic::new("E0801", message, node.span);
        let input = node
            .inputs
            .first()
            .and_then(|id| graph.nodes.get(*id))
            .and_then(|n| n.output.as_ref());
        let shape = node.output.as_ref().map(|t| &t.value);
        if let Some(value) = shape {
            let dtype = match value {
                ValueType::Probability(_) | ValueType::Decision(_) => &DType::F64,
                ValueType::Vector(d, _)
                | ValueType::Epoch(d, ..)
                | ValueType::Spectrum(d, ..)
                | ValueType::Covariance(d, ..) => d,
            };
            if *dtype != DType::F64 {
                return Err(error("선택한 수치 커널은 f64 포트를 요구한다."));
            }
        }
        let channels = node.output.as_ref().map_or(0, |o| o.schema.channels.len());
        let kernel = match node.operator.as_str() {
            "std.model.predict" => {
                let Some(Value::Model(model)) = node.parameters.get("using") else {
                    return Err(error("예측 모델을 연결한다."));
                };
                KernelSpec::Predict {
                    model: model.clone(),
                }
            }
            "std.decision.threshold" => {
                let Some(ValueType::Probability(classes)) = input.map(|t| &t.value) else {
                    return Err(error("판정 확률 입력을 연결한다."));
                };
                KernelSpec::Threshold {
                    classes: classes.clone(),
                    minimum: number(node, "minimum")?,
                }
            }
            "std.signal.psd" => {
                let Some(ValueType::Epoch(_, _, samples, rate)) = input.map(|i| &i.value) else {
                    return Err(error("PSD Epoch 입력을 연결한다."));
                };
                let taper = match node.parameters.get("taper") {
                    Some(Value::Enum(v)) if v == "Taper.Rectangular" => {
                        bci_schema::spectral::Taper::Rectangular
                    }
                    Some(Value::Enum(v)) if v == "Taper.Hann" => bci_schema::spectral::Taper::Hann,
                    _ => return Err(error("PSD 창 함수를 지정한다.")),
                };
                KernelSpec::Psd {
                    channels,
                    samples: *samples,
                    rate: *rate,
                    taper,
                }
            }
            "std.signal.bandpower" => {
                let Some(ValueType::Spectrum(_, _, samples, rate)) = input.map(|i| &i.value) else {
                    return Err(error("bandpower Spectrum 입력을 연결한다."));
                };
                let low = exact_number(node, "low_hz")?.0;
                let high = exact_number(node, "high_hz")?.0;
                let include_high =
                    matches!(node.parameters.get("include_high"), Some(Value::Bool(true)));
                KernelSpec::BandPower {
                    channels,
                    samples: *samples,
                    rate: *rate,
                    band: bci_schema::spectral::Band {
                        low,
                        high,
                        include_high,
                    },
                }
            }
            "std.signal.log_power" => {
                let (floor, unit) = exact_number(node, "floor")?;
                let (reference, reference_unit) = exact_number(node, "reference")?;
                if unit != reference_unit {
                    return Err(error("로그 전력의 단위를 맞춘다."));
                }
                KernelSpec::LogPower {
                    channels,
                    floor,
                    reference,
                    unit,
                }
            }
            "std.signal.select" | "std.signal.reorder" => {
                let input = input.ok_or_else(|| error("채널 변환의 입력을 연결한다."))?;
                let (_, input_channels, width) = bci_schema::spatial::layout(&input.value)
                    .map_err(|e| Diagnostic::new("E0801", e, node.span))?;
                let Some(Value::Array(ids)) = node.parameters.get("channels") else {
                    return Err(error("출력 채널 ID 배열을 지정한다."));
                };
                let indices = ids
                    .iter()
                    .map(|id| match id {
                        Value::Text(id) => input
                            .schema
                            .channels
                            .iter()
                            .position(|c| c.id == *id)
                            .ok_or_else(|| error("선택한 입력 채널 ID를 확인한다.")),
                        _ => Err(error("채널 ID는 문자열을 요구한다.")),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                KernelSpec::ChannelMap {
                    input_channels,
                    indices,
                    width,
                }
            }
            "std.signal.rereference" => {
                let input = input.ok_or_else(|| error("재기준화 입력을 연결한다."))?;
                let (_, _, width) = bci_schema::spatial::layout(&input.value)
                    .map_err(|e| Diagnostic::new("E0801", e, node.span))?;
                let Some(Value::Array(values)) = node.parameters.get("weights") else {
                    return Err(error("재기준화 weights를 지정한다."));
                };
                let weights = values
                    .iter()
                    .map(|v| match v {
                        Value::Number(n, u) if *u == Unit::scalar() => Ok(*n),
                        _ => Err(error("재기준화 weights는 무차원 수치를 요구한다.")),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                KernelSpec::Rereference { weights, width }
            }
            "std.signal.mix" => {
                let Some(Value::Array(weights)) = node.parameters.get("weights") else {
                    return Err(error("mix weights를 지정한다."));
                };
                let weights = weights
                    .iter()
                    .map(|v| match v {
                        Value::Number(n, u) if *u == Unit::scalar() => Ok(n.to_f64()),
                        _ => Err(error("mix weights는 무차원 수치를 요구한다.")),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                KernelSpec::Mix { weights, channels }
            }
            "std.source.synthetic" | "std.source.csv" | "std.source.replay" => {
                sources.push(node.id);
                let Some(Value::Source(source)) = node.parameters.get("source") else {
                    return Err(error("입력 원천 기술자를 연결한다."));
                };
                KernelSpec::Source(source.clone())
            }
            "std.signal.convert_unit" | "std.signal.scale" | "std.source.assert_schema" => {
                if matches!(shape, Some(ValueType::Covariance(..))) {
                    return Err(Diagnostic::new(
                        "E0803",
                        "현재 선형 커널은 Vec·Epoch 입력을 처리한다. 공분산·SPD 행렬은 covariance·regularize 커널로 연결한다.",
                        node.span,
                    ));
                }
                let width = match shape {
                    Some(ValueType::Epoch(_, _, n, _)) => *n,
                    Some(ValueType::Covariance(_, c, _)) => *c,
                    _ => 1,
                };
                let factors = if node.operator == "std.signal.convert_unit" {
                    let input = input.ok_or_else(|| error("단위 변환의 입력 포트를 연결한다."))?;
                    let output = node
                        .output
                        .as_ref()
                        .ok_or_else(|| error("단위 변환의 출력 타입을 지정한다."))?;
                    input
                        .schema
                        .channels
                        .iter()
                        .zip(&output.schema.channels)
                        .map(|(a, b)| {
                            a.unit
                                .scale
                                .divide(b.unit.scale)
                                .map(|n| n.to_f64())
                                .map_err(|e| Diagnostic::new("E0501", e, node.span))
                        })
                        .collect::<Result<Vec<_>, _>>()?
                } else {
                    vec![
                        if node.operator == "std.signal.scale" {
                            number(node, "factor")?
                        } else {
                            1.0
                        };
                        channels
                    ]
                };
                KernelSpec::Scale { factors, width }
            }
            "std.signal.iir" => {
                let Some(Value::Iir(realization)) = node.parameters.get("realization") else {
                    return Err(error("IIR 실현을 지정한다."));
                };
                KernelSpec::Iir {
                    channels,
                    realization: realization.clone(),
                }
            }
            "std.signal.resample" => {
                let input = input.ok_or_else(|| error("재표본화 입력을 연결한다."))?;
                let Occurrence::Uniform {
                    rate,
                    phase: input_phase,
                } = input.occurrence
                else {
                    return Err(error("재표본화의 확정 입력 격자를 지정한다."));
                };
                let coefficients = match node.parameters.get("filter") {
                    Some(Value::Coefficients(v)) => v.iter().map(|n| n.to_f64()).collect(),
                    Some(Value::Array(v)) => v
                        .iter()
                        .map(|v| match v {
                            Value::Number(n, u) if *u == Unit::scalar() => Ok(n.to_f64()),
                            _ => Err(error("재표본화 filter에 무차원 계수를 지정한다.")),
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                    _ => return Err(error("재표본화 filter를 지정한다.")),
                };
                KernelSpec::Resample {
                    channels,
                    specification: Box::new(bci_schema::resample::Resampling {
                        up: natural(node, "up")?,
                        down: natural(node, "down")?,
                        coefficients,
                        rate,
                        input_phase,
                        phase: {
                            let (n, u) = exact_number(node, "phase")?;
                            n.multiply(u.scale)
                                .map_err(|e| Diagnostic::new("E0801", e, node.span))?
                        },
                    }),
                }
            }
            "std.signal.fir" => {
                let coefficients = match node.parameters.get("coeffs") {
                    Some(Value::Coefficients(values)) => {
                        values.iter().map(|n| n.to_f64()).collect()
                    }
                    Some(Value::Array(values)) => values
                        .iter()
                        .map(|v| match v {
                            Value::Number(n, u) if *u == Unit::scalar() => Ok(n.to_f64()),
                            _ => Err(error("FIR 계수는 무차원 수치 배열을 요구한다.")),
                        })
                        .collect::<Result<_, _>>()?,
                    _ => return Err(error("FIR의 계수를 지정한다.")),
                };
                KernelSpec::Fir {
                    channels,
                    coefficients,
                }
            }
            "std.signal.window" => KernelSpec::Window {
                channels,
                size: natural(node, "size")?,
                hop: natural(node, "hop")?,
                start: match node.parameters.get("start") {
                    Some(Value::Number(n, _)) => Some(
                        n.natural()
                            .map_err(|e| Diagnostic::new("E0501", e, node.span))?
                            as u64,
                    ),
                    _ => None,
                },
            },
            "std.signal.covariance" => {
                let Some(ValueType::Epoch(_, _, samples, _)) = input.map(|t| &t.value) else {
                    return Err(error("공분산 커널은 Epoch 포트를 요구한다."));
                };
                KernelSpec::Covariance {
                    channels,
                    samples: *samples,
                }
            }
            "std.signal.regularize" => KernelSpec::Regularize {
                channels,
                shrinkage: number(node, "shrinkage")?,
                floor: number(node, "floor")?,
            },
            "std.recording.write" => {
                let Some(Value::Text(destination)) = node.parameters.get("to") else {
                    return Err(error("기록 경로를 지정한다."));
                };
                KernelSpec::Record {
                    destination: destination.clone(),
                }
            }
            _ if matches!(node.parameters.get("extension"), Some(Value::Extension(_))) => {
                let Some(Value::Extension(operator)) = node.parameters.get("extension") else {
                    unreachable!()
                };
                KernelSpec::Extension {
                    operator: operator.clone(),
                    channels,
                }
            }
            name => {
                return Err(Diagnostic::new(
                    "E0803",
                    format!(
                        "현재 실행 번들에 {name} 커널을 연결해야 한다. 지원 연산자는 docs/runtime.md에 정의한다."
                    ),
                    node.span,
                ));
            }
        };
        let buffer_bytes = kernel
            .buffer_bytes()
            .map_err(|e| Diagnostic::new("E0801", e, node.span))?;
        let state_bytes = kernel
            .state_bytes()
            .map_err(|e| Diagnostic::new("E0801", e, node.span))?;
        reserved = reserved
            .checked_add(buffer_bytes)
            .and_then(|r| r.checked_add(state_bytes))
            .ok_or_else(|| error("전체 메모리 예약량 범위를 초과했다."))?;
        if reserved > budget {
            return Err(Diagnostic::new(
                "E0801",
                format!(
                    "실행 예약량 {reserved} bytes가 예산 {budget} bytes를 초과했다. 창·채널 크기 또는 memory 예산을 조정한다."
                ),
                node.span,
            ));
        }
        nodes.push(PlannedNode {
            id: node.id,
            operator: node.operator.clone(),
            span: node.span,
            inputs: node.inputs.clone(),
            output: node.output.clone(),
            kernel,
            buffer_bytes,
            state_bytes,
        });
    }
    if !(1..=64).contains(&sources.len())
        || !nodes
            .iter()
            .any(|n| matches!(n.kernel, KernelSpec::Record { .. }))
    {
        return Err(Diagnostic::new(
            "E0801",
            "순차 실행은 원천 1..64개와 기록 sink를 요구한다.",
            Span::default(),
        ));
    }
    if sources.len() > 1 {
        let first = nodes[sources[0]].output.as_ref().expect("source type");
        for &id in &sources {
            let ty = nodes[id].output.as_ref().expect("source type");
            if ty.clock != first.clock {
                return Err(Diagnostic::new(
                    "E0401",
                    "동기 원천들은 같은 clock ID를 요구한다.",
                    nodes[id].span,
                ));
            }
            if !matches!(ty.occurrence, Occurrence::Uniform { .. }) {
                return Err(Diagnostic::new(
                    "E0205",
                    "다중 원천은 각각 확정 Uniform rate·phase를 요구한다.",
                    nodes[id].span,
                ));
            }
        }
    }
    let samples = if samples.is_none()
        && sources.iter().any(|&id| matches!(&nodes[id].kernel,KernelSpec::Source(source) if source.kind==SourceKind::Synthetic))
    {
        Some(1000)
    } else {
        samples
    };
    let queues = bci_ir::queues::Queues::for_nodes(&nodes)
        .map_err(|e| Diagnostic::new("E0801", e, Span::default()))?;
    reserved = reserved
        .checked_add(queues.bytes)
        .filter(|&n| n <= budget)
        .ok_or_else(|| {
            Diagnostic::new(
                "E0801",
                "커널·간선 큐의 예약량이 memory 예산을 초과했다.",
                Span::default(),
            )
        })?;
    Ok(ExecutionPlan {
        queues,
        entry: graph.entry.clone(),
        schedule: (0..nodes.len()).collect(),
        nodes,
        sources,
        samples,
        memory_budget: budget,
        reserved_bytes: reserved,
        obligations: graph.obligations.clone(),
    })
}

fn number(node: &TypedNode, name: &str) -> Result<f64, Diagnostic> {
    match node.parameters.get(name) {
        Some(Value::Number(n, _)) => Ok(n.to_f64()),
        _ => Err(Diagnostic::new(
            "E0801",
            format!("{name} 수치 구성을 요구한다."),
            node.span,
        )),
    }
}
fn natural(node: &TypedNode, name: &str) -> Result<usize, Diagnostic> {
    match node.parameters.get(name) {
        Some(Value::Number(n, _)) => n
            .natural()
            .map_err(|e| Diagnostic::new("E0801", e, node.span)),
        _ => Err(Diagnostic::new(
            "E0801",
            format!("{name} 정수 구성을 요구한다."),
            node.span,
        )),
    }
}

fn exact_number(
    node: &TypedNode,
    name: &str,
) -> Result<(bci_schema::number::Rational, Unit), Diagnostic> {
    match node.parameters.get(name) {
        Some(Value::Number(n, u)) => Ok((*n, u.clone())),
        _ => Err(Diagnostic::new(
            "E0801",
            format!("{name} 수치를 지정한다."),
            node.span,
        )),
    }
}
