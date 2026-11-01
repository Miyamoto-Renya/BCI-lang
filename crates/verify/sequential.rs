use bci_ir::plan::{ExecutionPlan, PlannedNode};
use bci_schema::{
    execution::RunError,
    input::{SourceKind, row_bound},
    kernel::KernelSpec,
    types::{DType, Occurrence, Span, ValueType},
};

/// 의존 순서·단일 슬롯 수명·구체 커널·메모리·의무 연결을 검사한 계획.
pub struct VerifiedPlan<'a>(&'a ExecutionPlan);

impl<'a> VerifiedPlan<'a> {
    /// 검사한 불변 계획을 조회한다.
    pub fn plan(&self) -> &'a ExecutionPlan {
        self.0
    }
}

/// 동기 원천 묶음과 순차 실행 계약을 독립 검사한다.
pub fn verify(plan: &ExecutionPlan) -> Result<VerifiedPlan<'_>, RunError> {
    let fault = |node: Option<&PlannedNode>, message: String| RunError {
        code: "E0801",
        node: node.map(|n| n.id),
        span: node.map_or(Span::default(), |n| n.span),
        message,
    };
    if plan
        .samples
        .is_some_and(|n| !(1..=1_000_000_000).contains(&n))
        || plan.nodes.is_empty()
        || plan.nodes.len() > 65_536
    {
        return Err(fault(
            None,
            "표본 수와 노드 수를 실행 범위에 맞춘다.".into(),
        ));
    }
    if plan.schedule != (0..plan.nodes.len()).collect::<Vec<_>>() {
        return Err(fault(
            None,
            "순차 일정은 각 노드를 ID 의존 순서로 정확히 한 번 방문해야 한다.".into(),
        ));
    }
    let mut sources = Vec::new();
    let mut sinks = 0;
    let mut total = 0usize;
    let mut consumed = vec![false; plan.nodes.len()];
    for (index, node) in plan.nodes.iter().enumerate() {
        let invalid = |message: &str| fault(Some(node), message.into());
        if node.id != index
            || node
                .inputs
                .iter()
                .any(|&id| id >= index || plan.nodes[id].output.is_none())
        {
            return Err(invalid("입력은 앞선 노드의 출력 슬롯을 참조해야 한다."));
        }
        for &input in &node.inputs {
            consumed[input] = true;
        }
        let (values, channels) = node.kernel.layout().map_err(|e| fault(Some(node), e))?;
        if let Some(output) = &node.output {
            bci_schema::learning::typed_stream(output).map_err(|e| fault(Some(node), e))?;
            if channels == 0 || channels > 65_536 || output.schema.channels.len() != channels {
                return Err(invalid("출력 채널과 커널 배열 크기를 맞춘다."));
            }
            let (dtype, shape_channels, count) = match output.value {
                ValueType::Probability(ref classes)
                    if bci_schema::learning::classes_valid(classes) =>
                {
                    (&DType::F64, classes.len(), Some(classes.len()))
                }
                ValueType::Decision(ref classes)
                    if bci_schema::learning::classes_valid(classes) =>
                {
                    (&DType::F64, 2, Some(2))
                }
                ValueType::Probability(_) | ValueType::Decision(_) => {
                    return Err(invalid("확률·판정 클래스 순서를 확인한다."));
                }
                ValueType::Spectrum(ref d, c, n, rate) => {
                    if !(1..=bci_schema::spectral::MAX_DFT_SAMPLES).contains(&n)
                        || !rate.is_positive()
                    {
                        return Err(invalid("PSD의 DFT 표본 수와 양의 내부 격자를 확인한다."));
                    }
                    (d, c, c.checked_mul(n / 2 + 1))
                }
                ValueType::Vector(ref d, c) => (d, c, Some(c)),
                ValueType::Epoch(ref d, c, _, _) | ValueType::Covariance(ref d, c, _) => (
                    d,
                    c,
                    c.checked_mul(match output.value {
                        ValueType::Epoch(_, _, n, _) => n,
                        _ => c,
                    }),
                ),
            };
            if *dtype != DType::F64 || shape_channels != channels || count != Some(values) {
                return Err(invalid("출력 타입과 f64 커널 layout을 맞춘다."));
            }
        } else if !matches!(node.kernel, KernelSpec::Record { .. }) {
            return Err(invalid("수치 노드의 출력 타입을 지정한다."));
        }
        let input = node
            .inputs
            .first()
            .and_then(|&id| plan.nodes[id].output.as_ref());
        let output = node.output.as_ref();
        match &node.kernel {
            KernelSpec::Source(spec) => {
                let kind_valid = match &spec.kind {
                    SourceKind::Synthetic => {
                        node.operator == "std.source.synthetic"
                            && spec.row_bytes == 0
                            && plan.samples.is_some()
                    }
                    SourceKind::Csv { path, .. } => {
                        node.operator == "std.source.csv"
                            && !path.as_os_str().is_empty()
                            && spec.row_bytes
                                == row_bound(values, channels).map_err(|e| fault(Some(node), e))?
                    }
                    SourceKind::Replay { path, .. } => {
                        node.operator == "std.source.replay"
                            && !path.as_os_str().is_empty()
                            && spec.row_bytes
                                >= row_bound(values, channels).map_err(|e| fault(Some(node), e))?
                    }
                };
                let rate_valid = match spec.stream.occurrence {
                    Occurrence::Uniform { rate, .. } | Occurrence::PendingUniform { rate, .. } => {
                        rate.is_positive()
                    }
                    _ => false,
                };
                let shape_valid = matches!(spec.kind, SourceKind::Replay { .. })
                    || (matches!(spec.stream.value, ValueType::Vector(DType::F64, _))
                        && matches!(spec.stream.occurrence, Occurrence::Uniform { .. }));
                if !kind_valid
                    || !shape_valid
                    || !node.inputs.is_empty()
                    || !rate_valid
                    || output != Some(&spec.stream)
                {
                    return Err(invalid(
                        "입력 원천의 reader·타입·양의 발생률 계약을 맞춘다.",
                    ));
                }
                sources.push(index);
            }
            KernelSpec::Mix { weights, .. } => {
                let input = input.ok_or_else(|| invalid("mix 입력을 연결한다."))?;
                let output = output.ok_or_else(|| invalid("mix 출력을 지정한다."))?;
                if node.operator != "std.signal.mix"
                    || !(2..=64).contains(&weights.len())
                    || weights.len() != node.inputs.len()
                    || weights.iter().any(|v| !v.is_finite())
                    || !matches!(input.value,ValueType::Vector(DType::F64,c) if c==channels)
                    || input.mismatch(output).is_some()
                    || node.inputs.iter().any(|&id| {
                        plan.nodes[id]
                            .output
                            .as_ref()
                            .is_none_or(|t| t.mismatch(output).is_some())
                    })
                {
                    return Err(invalid(
                        "mix의 동일 포트 타입·입력 수·유한 weights 계약을 맞춘다.",
                    ));
                }
            }
            KernelSpec::Record { destination } => {
                sinks += 1;
                if node.operator != "std.recording.write"
                    || node.inputs.is_empty()
                    || node.output.is_some()
                    || destination.is_empty()
                {
                    return Err(invalid("기록 sink의 입력·경로·출력 계약을 맞춘다."));
                }
            }
            kernel => {
                if node.inputs.len() != 1 {
                    return Err(invalid("수치 커널에 입력 포트 하나를 연결한다."));
                }
                let input = input.ok_or_else(|| invalid("입력 타입을 지정한다."))?;
                let output = output.ok_or_else(|| invalid("출력 타입을 지정한다."))?;
                if input.clock != output.clock || input.quality != output.quality {
                    return Err(invalid("표준 순차 커널의 clock·품질 어휘를 보존한다."));
                }
                let vector =
                    matches!(input.value, ValueType::Vector(DType::F64,c) if c == channels);
                let valid = match kernel {
                    KernelSpec::Predict { model } => {
                        node.operator == "std.model.predict"
                            && model.output(input).is_ok_and(|t| t == *output)
                    }
                    KernelSpec::Threshold { classes, minimum } => {
                        node.operator == "std.decision.threshold"
                            && input.value == ValueType::Probability(classes.clone())
                            && bci_schema::learning::decision(input, *minimum)
                                .is_ok_and(|t| t == *output)
                    }
                    KernelSpec::Psd {
                        channels: c,
                        samples,
                        rate,
                        taper,
                    } => {
                        node.operator == "std.signal.psd"
                            && input.value == ValueType::Epoch(DType::F64, *c, *samples, *rate)
                            && bci_schema::spectral::psd(input, *taper).is_ok_and(|t| t == *output)
                    }
                    KernelSpec::BandPower {
                        channels: c,
                        samples,
                        rate,
                        band,
                    } => {
                        node.operator == "std.signal.bandpower"
                            && input.value == ValueType::Spectrum(DType::F64, *c, *samples, *rate)
                            && bci_schema::spectral::bandpower(input, band)
                                .is_ok_and(|t| t == *output)
                    }
                    KernelSpec::LogPower {
                        floor,
                        reference,
                        unit,
                        ..
                    } => {
                        node.operator == "std.signal.log_power"
                            && bci_schema::spectral::log_power(input, *floor, *reference, unit)
                                .is_ok_and(|t| t == *output)
                    }
                    KernelSpec::ChannelMap {
                        input_channels,
                        indices,
                        width,
                    } => {
                        ["std.signal.select", "std.signal.reorder"]
                            .contains(&node.operator.as_str())
                            && bci_schema::spatial::layout(&input.value).is_ok_and(|(d, c, n)| {
                                d == DType::F64 && c == *input_channels && n == *width
                            })
                            && bci_schema::spatial::select(
                                input,
                                indices,
                                node.operator.ends_with("reorder"),
                            )
                            .is_ok_and(|expected| expected == *output)
                    }
                    KernelSpec::Rereference { weights, width } => {
                        node.operator == "std.signal.rereference"
                            && bci_schema::spatial::layout(&input.value)
                                .is_ok_and(|(d, _, n)| d == DType::F64 && n == *width)
                            && bci_schema::spatial::rereference(input, weights)
                                .is_ok_and(|expected| expected == *output)
                    }
                    KernelSpec::Extension { operator, .. } => {
                        operator.validate().is_ok()
                            && node.operator == operator.name
                            && operator.accepts(input)
                            && input == output
                    }
                    KernelSpec::Scale { factors, width } => {
                        let same_channels = input.schema.channels == output.schema.channels;
                        let conversion = input
                            .schema
                            .channels
                            .iter()
                            .zip(&output.schema.channels)
                            .zip(factors)
                            .all(|((a, b), factor)| {
                                a.id == b.id
                                    && a.quantity == b.quantity
                                    && a.reference == b.reference
                                    && a.unit.same_dimension(&b.unit)
                                    && a.unit
                                        .scale
                                        .divide(b.unit.scale)
                                        .is_ok_and(|r| r.to_f64() == *factor)
                            });
                        [
                            "std.signal.scale",
                            "std.signal.convert_unit",
                            "std.source.assert_schema",
                        ]
                        .contains(&node.operator.as_str())
                            && factors.iter().all(|v| v.is_finite())
                            && *width > 0
                            && matches!(input.value, ValueType::Vector(..) | ValueType::Epoch(..))
                            && input.value == output.value
                            && input.occurrence == output.occurrence
                            && match node.operator.as_str() {
                                "std.signal.convert_unit" => conversion,
                                "std.source.assert_schema" => {
                                    same_channels && factors.iter().all(|v| *v == 1.0)
                                }
                                _ => same_channels && factors.windows(2).all(|v| v[0] == v[1]),
                            }
                    }
                    KernelSpec::Resample { specification, .. } => {
                        node.operator == "std.signal.resample"
                            && vector
                            && specification.output(input).is_ok_and(|t| t == *output)
                    }
                    KernelSpec::Iir { realization, .. } => {
                        node.operator == "std.signal.iir"
                            && vector
                            && input == output
                            && realization.accepts(input).is_ok()
                    }
                    KernelSpec::Fir { coefficients, .. } => {
                        node.operator == "std.signal.fir"
                            && vector
                            && (1..=65_536).contains(&coefficients.len())
                            && coefficients.iter().all(|v| v.is_finite())
                            && input == output
                    }
                    KernelSpec::Window {
                        size, hop, start, ..
                    } => {
                        let rate = match input.occurrence {
                            Occurrence::Uniform { rate, .. } => Some(rate),
                            _ => None,
                        };
                        let expected = rate.and_then(|rate| {
                            rate.divide(bci_schema::number::Rational::new(*hop as i128, 1).ok()?)
                                .ok()
                        });
                        let output_rate = match output.occurrence {
                            Occurrence::Uniform { rate, .. }
                            | Occurrence::PendingUniform { rate, .. } => Some(rate),
                            _ => None,
                        };
                        let phase_matches = match (start, &input.occurrence, &output.occurrence) {
                            (
                                Some(start),
                                Occurrence::Uniform { rate, phase },
                                Occurrence::Uniform { phase: out, .. },
                            ) => {
                                start
                                    .checked_add(*size as u64)
                                    .and_then(|end| end.checked_sub(1))
                                    .and_then(|last| {
                                        bci_schema::number::Rational::new(last as i128, 1).ok()
                                    })
                                    .and_then(|last| last.divide(*rate).ok())
                                    .and_then(|offset| phase.checked_add(offset).ok())
                                    == Some(*out)
                            }
                            (None, _, Occurrence::PendingUniform { phase_key, .. }) => {
                                !phase_key.is_empty()
                            }
                            _ => false,
                        };
                        node.operator == "std.signal.window"
                            && vector
                            && (1..=65_536).contains(size)
                            && (1..=65_536).contains(hop)
                            && expected.is_some()
                            && expected == output_rate
                            && phase_matches
                            && input.schema.channels == output.schema.channels
                            && start.is_none_or(|s| s.checked_add(*size as u64).is_some())
                            && matches!(output.value, ValueType::Epoch(DType::F64,c,n,r) if c==channels && n==*size && Some(r)==rate)
                    }
                    KernelSpec::Covariance { samples, .. } => {
                        node.operator == "std.signal.covariance"
                            && *samples >= 2
                            && matches!(input.value,ValueType::Epoch(DType::F64,c,n,_) if c==channels && n==*samples)
                            && output.value == ValueType::Covariance(DType::F64, channels, false)
                            && input.occurrence == output.occurrence
                            && input
                                .schema
                                .channels
                                .iter()
                                .zip(&output.schema.channels)
                                .all(|(a, b)| {
                                    a.id == b.id
                                        && a.reference == b.reference
                                        && b.quantity == format!("covariance({})", a.quantity)
                                        && a.unit.power(2).is_ok_and(|u| u == b.unit)
                                })
                            && input
                                .schema
                                .channels
                                .windows(2)
                                .all(|v| v[0].unit == v[1].unit && v[0].quantity == v[1].quantity)
                    }
                    KernelSpec::Regularize {
                        shrinkage, floor, ..
                    } => {
                        node.operator == "std.signal.regularize"
                            && shrinkage.is_finite()
                            && *shrinkage > 0.0
                            && *shrinkage <= 1.0
                            && floor.is_finite()
                            && *floor > 0.0
                            && input.value == ValueType::Covariance(DType::F64, channels, false)
                            && output.value == ValueType::Covariance(DType::F64, channels, true)
                            && input.occurrence == output.occurrence
                            && input.schema.channels == output.schema.channels
                    }
                    _ => false,
                };
                if !valid {
                    return Err(invalid("커널의 이름·입력·출력·매개변수 계약을 맞춘다."));
                }
            }
        }
        let buffer = node
            .kernel
            .buffer_bytes()
            .map_err(|e| fault(Some(node), e))?;
        let state = node
            .kernel
            .state_bytes()
            .map_err(|e| fault(Some(node), e))?;
        if buffer != node.buffer_bytes || state != node.state_bytes {
            return Err(invalid("커널 계약에서 메모리 예약량을 다시 계산한다."));
        }
        total = total
            .checked_add(buffer)
            .and_then(|v| v.checked_add(state))
            .ok_or_else(|| invalid("메모리 합의 범위를 초과했다."))?;
    }
    let queues = bci_ir::queues::Queues::for_nodes(&plan.nodes).map_err(|e| fault(None, e))?;
    if queues != plan.queues {
        return Err(fault(None, "간선 큐 용량을 burst·발생률과 맞춘다.".into()));
    }
    total = total
        .checked_add(queues.bytes)
        .ok_or_else(|| fault(None, "큐 예약량 합의 범위를 초과했다.".into()))?;
    if sources != plan.sources
        || !(1..=64).contains(&sources.len())
        || sinks == 0
        || consumed
            .iter()
            .zip(&plan.nodes)
            .any(|(used, n)| !used && n.output.is_some())
        || total != plan.reserved_bytes
        || total > plan.memory_budget
    {
        return Err(fault(
            None,
            "원천·출력 소비 관계·전체 예약량과 예산을 맞춘다.".into(),
        ));
    }
    if sources.len() > 1 {
        let first = plan.nodes[sources[0]]
            .output
            .as_ref()
            .expect("verified source");
        for &id in &sources {
            let ty = plan.nodes[id].output.as_ref().expect("verified source");
            if ty.clock != first.clock || !matches!(ty.occurrence, Occurrence::Uniform { .. }) {
                return Err(fault(
                    Some(&plan.nodes[id]),
                    "다중 원천에 같은 clock과 각각의 확정 rate·phase를 지정한다.".into(),
                ));
            }
        }
    }
    let mut guards = std::collections::BTreeSet::new();
    let mut obligation_ids = std::collections::BTreeSet::new();
    for obligation in &plan.obligations {
        let node = plan
            .nodes
            .get(obligation.node)
            .ok_or_else(|| fault(None, "의무의 소유 노드를 연결한다.".into()))?;
        if !guards.insert((obligation.node, obligation.kind.as_str()))
            || !obligation_ids.insert(obligation.id)
            || !matches!(
                (obligation.kind.as_str(), &node.kernel),
                ("Resampling", KernelSpec::Resample { .. })
                    | ("WindowPhase", KernelSpec::Window { start: None, .. })
                    | ("NumericGuard", KernelSpec::Regularize { .. })
                    | ("Storage", KernelSpec::Record { .. })
                    | ("Input", KernelSpec::Source(_))
            )
        {
            return Err(fault(
                Some(node),
                format!("{} 의무를 해소하는 실행 경계를 연결한다.", obligation.kind),
            ));
        }
    }
    for node in &plan.nodes {
        let required = match node.kernel {
            KernelSpec::Source(ref source) if source.kind != SourceKind::Synthetic => Some("Input"),
            KernelSpec::Resample { .. } => Some("Resampling"),
            KernelSpec::Window { start: None, .. } => Some("WindowPhase"),
            KernelSpec::Regularize { .. } => Some("NumericGuard"),
            KernelSpec::Record { .. } => Some("Storage"),
            _ => None,
        };
        if let Some(kind) = required
            && !guards.contains(&(node.id, kind))
        {
            return Err(fault(
                Some(node),
                format!("{kind} 의무를 커널에 하나 연결한다."),
            ));
        }
    }
    Ok(VerifiedPlan(plan))
}
