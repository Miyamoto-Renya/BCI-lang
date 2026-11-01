use crate::{
    Diagnostic,
    syntax::*,
    value::{Environment, Value},
};
use bci_ir::graph::{NodeId, Obligation, TypedGraph, TypedNode};
use bci_schema::{LANGUAGE_VERSION, registry, types::*};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub(crate) struct Checker<'a> {
    pub program: &'a Program,
    pub base: &'a Path,
    pub extensions: &'a [bci_schema::extension::OperatorSpec],
    pub(crate) globals: Environment,
    pub(crate) imports: BTreeMap<String, String>,
    resolving: Vec<String>,
    calls: Vec<String>,
    pub(crate) evaluation_depth: usize,
    pub graph: TypedGraph,
    pub limit: Mode,
    pub configured: bool,
    pub in_template: bool,
}

pub(crate) fn check(
    program: &Program,
    base: &Path,
    extensions: &[bci_schema::extension::OperatorSpec],
) -> Result<(Vec<TypedGraph>, Vec<TypedGraph>), Vec<Diagnostic>> {
    let mut checker = Checker::new(program, base, extensions);
    checker.names().map_err(|error| vec![error])?;
    for binding in &program.bindings {
        checker
            .global(&binding.name, binding.span)
            .map_err(|error| vec![error])?;
    }
    let mut templates = Vec::new();
    let mut graphs = Vec::new();
    let mut errors = Vec::new();
    for graph in &program.graphs {
        checker.reset(
            &graph.name,
            if graph.mode == "live" {
                Mode::Live
            } else {
                Mode::Record
            },
        );
        checker.in_template = true;
        match checker.template(graph) {
            Ok(()) => templates.push(checker.graph.clone()),
            Err(error) => errors.push(error),
        }
    }
    for run in &program.runs {
        checker.reset(&run.name, Mode::Live);
        let result = checker
            .body(&run.body, &mut Environment::new(), false)
            .and_then(|_| checker.finish(&[]));
        match result {
            Ok(()) => graphs.push(checker.graph.clone()),
            Err(error) => errors.push(error),
        }
    }
    if errors.is_empty() {
        Ok((graphs, templates))
    } else {
        Err(errors)
    }
}

impl<'a> Checker<'a> {
    fn new(
        program: &'a Program,
        base: &'a Path,
        extensions: &'a [bci_schema::extension::OperatorSpec],
    ) -> Self {
        Self {
            program,
            base,
            extensions,
            globals: Environment::new(),
            imports: BTreeMap::new(),
            resolving: Vec::new(),
            calls: Vec::new(),
            evaluation_depth: 0,
            graph: TypedGraph {
                entry: program.module.clone(),
                configuration: BTreeMap::new(),
                nodes: Vec::new(),
                bindings: BTreeMap::new(),
                obligations: Vec::new(),
                effect: Effect::new(Mode::Live),
            },
            limit: Mode::Live,
            configured: false,
            in_template: false,
        }
    }

    fn reset(&mut self, name: &str, mode: Mode) {
        self.graph = TypedGraph {
            entry: format!("{}.{}", self.program.module, name),
            configuration: BTreeMap::new(),
            nodes: Vec::new(),
            bindings: BTreeMap::new(),
            obligations: Vec::new(),
            effect: Effect::new(Mode::Live),
        };
        self.limit = mode;
        self.configured = false;
        self.in_template = false;
        self.calls.clear();
        self.evaluation_depth = 0;
    }

    fn names(&mut self) -> Result<(), Diagnostic> {
        if self.program.language != LANGUAGE_VERSION {
            return Err(Diagnostic::new(
                "E0109",
                format!("언어 판본은 {LANGUAGE_VERSION}을 요구한다."),
                Span::default(),
            ));
        }
        let mut names = BTreeSet::new();
        for (module, alias, span) in &self.program.imports {
            if prelude_root(alias) || !names.insert(alias.clone()) {
                return Err(Diagnostic::new(
                    "E0102",
                    format!("import 별칭 충돌: {alias}"),
                    *span,
                ));
            }
            if !registry::contains_module(module)
                && !self
                    .extensions
                    .iter()
                    .any(|s| s.name.rsplit_once('.').is_some_and(|(m, _)| m == module))
            {
                return Err(Diagnostic::new(
                    "E0103",
                    format!("등록한 모듈을 import한다: {module}"),
                    *span,
                ));
            }
            self.imports.insert(alias.clone(), module.clone());
        }
        for binding in &self.program.bindings {
            self.reserve(&mut names, &binding.name, binding.span)?;
        }
        for graph in &self.program.graphs {
            self.reserve(&mut names, &graph.name, graph.span)?;
            let mut parameters = BTreeSet::new();
            for (name, ty) in &graph.params {
                self.reserve(&mut parameters, name, ty.span)?;
                if self.imports.contains_key(name) {
                    return Err(Diagnostic::new(
                        "E0102",
                        "매개변수 이름이 import 별칭과 충돌한다.",
                        ty.span,
                    ));
                }
            }
        }
        for run in &self.program.runs {
            self.reserve(&mut names, &run.name, run.span)?;
        }
        Ok(())
    }

    fn reserve(
        &self,
        names: &mut BTreeSet<String>,
        name: &str,
        span: Span,
    ) -> Result<(), Diagnostic> {
        if [
            "language", "module", "use", "as", "pub", "const", "schema", "clock", "device",
            "dataset", "model", "protocol", "resource", "graph", "run", "let", "return", "live",
            "record", "true", "false",
        ]
        .contains(&name)
            || prelude_root(name)
            || !names.insert(name.into())
        {
            return Err(Diagnostic::new(
                "E0102",
                format!("이름 충돌 또는 예약된 이름: {name}. 고유한 이름을 지정한다."),
                span,
            ));
        }
        Ok(())
    }

    fn global(&mut self, name: &str, span: Span) -> Result<Value, Diagnostic> {
        if let Some(value) = self.globals.get(name) {
            return Ok(value.clone());
        }
        if self.resolving.iter().any(|n| n == name) {
            return Err(Diagnostic::new(
                "E0107",
                format!("선언 의존 순환: {} -> {name}", self.resolving.join(" -> ")),
                span,
            ));
        }
        if self.resolving.len() > 128 {
            return Err(Diagnostic::new(
                "E0108",
                "상수 의존 깊이는 128 이하여야 한다.",
                span,
            ));
        }
        let binding = self
            .program
            .bindings
            .iter()
            .find(|b| b.name == name)
            .cloned()
            .ok_or_else(|| {
                Diagnostic::new(
                    "E0103",
                    format!("이름을 선언하거나 import한다: {name}"),
                    span,
                )
            })?;
        self.resolving.push(name.into());
        let result = self.eval(&binding.value, &Environment::new(), true);
        self.resolving.pop();
        let value = result?;
        let kind_valid = match binding.kind.as_str() {
            "const" => true,
            "schema" => matches!(value, Value::Schema(_)),
            "clock" => matches!(value, Value::Clock(_)),
            "device" => matches!(value, Value::Device { .. }),
            "resource" => matches!(value, Value::Resource(_)),
            _ => false,
        };
        if !kind_valid {
            return Err(Diagnostic::new(
                "E0201",
                format!(
                    "{} 선언에 맞는 기술자를 요구한다: {}",
                    binding.kind,
                    value.describe()
                ),
                binding.span,
            ));
        }
        if let Some(annotation) = &binding.annotation {
            self.annotation(&value, annotation)?;
        }
        self.globals.insert(name.into(), value.clone());
        Ok(value)
    }

    pub fn identity(&self, span: Span) -> String {
        if let Some(name) = self.resolving.last() {
            format!("{}.{}", self.program.module, name)
        } else {
            format!(
                "{}@{}:{}",
                self.graph.entry,
                span.start,
                self.graph.nodes.len()
            )
        }
    }

    pub fn lookup(
        &mut self,
        name: &str,
        env: &Environment,
        span: Span,
    ) -> Result<Value, Diagnostic> {
        if let Some(value) = env.get(name) {
            return Ok(value.clone());
        }
        if known_enum(name) {
            return Ok(Value::Enum(name.into()));
        }
        if let Some((root, rest)) = name.split_once('.') {
            let mut value = self.lookup(root, env, span)?;
            for field in rest.split('.') {
                value = self.field(value, field, span)?;
            }
            return Ok(value);
        }
        self.global(name, span)
    }

    pub(crate) fn field(&self, value: Value, name: &str, span: Span) -> Result<Value, Diagnostic> {
        match value {
            Value::Record(fields) => fields.get(name).cloned().ok_or_else(|| {
                Diagnostic::new("E0103", format!("레코드 필드 {name}를 정의한다."), span)
            }),
            _ => Err(Diagnostic::new(
                "E0201",
                "필드 접근은 레코드 값에 적용한다.",
                span,
            )),
        }
    }

    fn body(
        &mut self,
        body: &[Statement],
        env: &mut Environment,
        graph: bool,
    ) -> Result<Value, Diagnostic> {
        for statement in body {
            match statement {
                Statement::Let(binding) => {
                    let mut names = env.keys().cloned().collect();
                    self.reserve(&mut names, &binding.name, binding.span)?;
                    if self.imports.contains_key(&binding.name) {
                        return Err(Diagnostic::new(
                            "E0102",
                            "지역 이름이 import 별칭과 충돌한다.",
                            binding.span,
                        ));
                    }
                    let value = self.eval(&binding.value, env, false)?;
                    if matches!(value, Value::Void) {
                        return Err(Diagnostic::new(
                            "E0201",
                            "let에는 값 또는 stream 출력을 연결한다.",
                            binding.span,
                        ));
                    }
                    if let Some(annotation) = &binding.annotation {
                        self.annotation(&value, annotation)?;
                    }
                    if let Value::Stream(id) = &value {
                        let prefix = self.calls.join(".");
                        self.graph.bindings.insert(
                            if prefix.is_empty() {
                                binding.name.clone()
                            } else {
                                format!("{prefix}.{}#{}", binding.name, id)
                            },
                            *id,
                        );
                    }
                    env.insert(binding.name.clone(), value);
                }
                Statement::Expression(expr) => {
                    let value = self.eval(expr, env, false)?;
                    if !matches!(value, Value::Void) {
                        return Err(Diagnostic::new(
                            "E0110",
                            "표현식의 결과를 let 또는 sink에 연결한다.",
                            expr.span,
                        ));
                    }
                }
                Statement::Return(expr) => {
                    if graph {
                        return self.eval(expr, env, false);
                    }
                    return Err(Diagnostic::new(
                        "E0101",
                        "return은 graph 본문에서 사용한다.",
                        expr.span,
                    ));
                }
            }
        }
        Ok(Value::Void)
    }

    pub(crate) fn invoke(
        &mut self,
        graph: &Graph,
        args: &[Argument],
        env: &Environment,
        span: Span,
    ) -> Result<Value, Diagnostic> {
        if self.calls.contains(&graph.name) || self.calls.len() >= 64 {
            return Err(Diagnostic::new(
                "E0107",
                format!(
                    "graph 호출 순환 또는 깊이 초과: {} -> {}",
                    self.calls.join(" -> "),
                    graph.name
                ),
                span,
            ));
        }
        let params = graph
            .params
            .iter()
            .map(|(n, _)| (n.clone(), true))
            .collect::<Vec<_>>();
        let mut local = self.arguments(args, &params, env, false, span)?;
        for (name, ty) in &graph.params {
            self.annotation(local.get(name).expect("validated argument"), ty)
                .map_err(|mut e| {
                    e.span = span;
                    e
                })?;
        }
        if graph.mode == "live"
            && local
                .values()
                .flat_map(Value::streams)
                .any(|id| self.graph.nodes[id].effect.mode == Mode::Record)
        {
            return Err(Diagnostic::new(
                "E0301",
                "Live graph 입력에 Record 효과가 연결되었다. graph의 효과 선언과 입력 경로를 맞춘다.",
                span,
            ));
        }
        self.calls.push(graph.name.clone());
        let outer = self.limit;
        self.limit = if graph.mode == "live" {
            Mode::Live
        } else {
            Mode::Record
        };
        let result = self.body(&graph.body, &mut local, true).and_then(|value| {
            self.annotation(&value, &graph.output)?;
            Ok(value)
        });
        self.limit = outer;
        self.calls.pop();
        let value = result?;
        if outer == Mode::Live
            && (graph.mode == "record"
                || value
                    .streams()
                    .iter()
                    .any(|&id| self.graph.nodes[id].effect.mode == Mode::Record))
        {
            return Err(Diagnostic::new(
                "E0301",
                "Live 경로에 Record graph가 연결되었다. 실행 모드와 호출 경로를 맞춘다.",
                span,
            ));
        }
        Ok(value)
    }

    fn template(&mut self, graph: &Graph) -> Result<(), Diagnostic> {
        let mut env = Environment::new();
        for (name, ty) in &graph.params {
            let output = self.stream_type(ty, None)?;
            let id = self.node(
                "graph.input",
                Vec::new(),
                Some(output),
                Environment::new(),
                Effect::new(Mode::Live),
                ty.span,
            )?;
            env.insert(name.clone(), Value::Stream(id));
        }
        self.calls.push(graph.name.clone());
        let result = self.body(&graph.body, &mut env, true)?;
        self.annotation(&result, &graph.output)?;
        self.finish(&result.streams())
    }

    fn finish(&self, returns: &[NodeId]) -> Result<(), Diagnostic> {
        let mut consumed: BTreeSet<_> = self
            .graph
            .nodes
            .iter()
            .flat_map(|node| node.inputs.iter().copied())
            .collect();
        consumed.extend(returns);
        for node in &self.graph.nodes {
            if node.output.is_some() && !consumed.contains(&node.id) {
                return Err(Diagnostic::new(
                    "E0110",
                    format!(
                        "%{} {}의 출력을 sink 또는 graph 반환에 연결한다.",
                        node.id, node.operator
                    ),
                    node.span,
                ));
            }
        }
        Ok(())
    }

    pub fn node(
        &mut self,
        operator: &str,
        inputs: Vec<NodeId>,
        output: Option<StreamType>,
        args: Environment,
        mut effect: Effect,
        span: Span,
    ) -> Result<NodeId, Diagnostic> {
        if self.graph.nodes.len() >= 65_536 {
            return Err(Diagnostic::new(
                "E0108",
                "구체화 그래프는 65536 노드 이하여야 한다.",
                span,
            ));
        }
        for id in &inputs {
            effect.combine(&self.graph.nodes[*id].effect);
        }
        if self.limit == Mode::Live && effect.mode == Mode::Record {
            return Err(Diagnostic::new(
                "E0301",
                format!(
                    "Live 경로에 Record 효과가 연결되었다: {operator}. 실행 모드와 연산자를 맞춘다."
                ),
                span,
            ));
        }
        self.graph.effect.combine(&effect);
        let id = self.graph.nodes.len();
        self.graph.nodes.push(TypedNode {
            id,
            operator: operator.into(),
            span,
            inputs,
            output,
            parameters: args
                .into_iter()
                .filter(|(_, v)| v.streams().is_empty())
                .collect(),
            effect,
        });
        Ok(id)
    }

    pub fn obligation(
        &mut self,
        node: NodeId,
        kind: &str,
        predicate: impl Into<String>,
        span: Span,
    ) {
        self.graph.obligations.push(Obligation {
            id: self.graph.obligations.len(),
            node,
            kind: kind.into(),
            predicate: predicate.into(),
            span,
        });
    }

    pub fn stream(&self, value: &Value, span: Span) -> Result<(NodeId, StreamType), Diagnostic> {
        if let Value::Stream(id) = value
            && let Some(ty) = &self.graph.nodes[*id].output
        {
            return Ok((*id, ty.clone()));
        }
        Err(Diagnostic::new(
            "E0201",
            format!("Stream 입력을 요구한다: {}", value.describe()),
            span,
        ))
    }
}

fn prelude_root(name: &str) -> bool {
    [
        "Mode",
        "Warmup",
        "Taper",
        "Psd",
        "WindowStart",
        "Anchor",
        "Quality",
        "Tail",
        "Queue",
        "Gap",
        "Invalid",
        "StandardQuality",
        "Events",
    ]
    .contains(&name)
}
fn known_enum(name: &str) -> bool {
    [
        "Mode.Live",
        "Mode.Record",
        "Warmup.Full",
        "Taper.Rectangular",
        "Taper.Hann",
        "Psd.OneSidedDensity",
        "WindowStart.FirstValid",
        "Anchor.End",
        "Quality.RequireAll",
        "Tail.Discard",
        "Queue.Fault",
        "Queue.DropOldest",
        "Gap.Reset",
        "Gap.Fault",
        "Invalid.Mark",
        "Invalid.Fault",
    ]
    .contains(&name)
}
