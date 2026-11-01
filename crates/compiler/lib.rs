//! 소스 해석, 이름·타입·효과 검사와 TypedGraph 구체화를 수행한다.
//!
//! 구문 처리, 선언 검사, 정적 평가, 타입 해석과 연산자 규칙을 루트의 형제 파일로 구성한다.
//! CLI는 공개 compile API를 사용하고, 장치 바인딩과 실행 계획 생성은 이 그래프를 확장한다.

mod check;
mod diagnostic;
mod evaluate;
mod lexer;
mod operators;
mod parser;
mod plan;
mod syntax;
mod typing;
mod value;

use bci_ir::graph::TypedGraph;
pub use diagnostic::Diagnostic;
use std::{fmt::Write, path::Path};

/// 한 소스 모듈의 정적 검사와 그래프 구체화 결과.
#[derive(Debug)]
pub struct Compilation {
    /// 소스 모듈 이름.
    pub module: String,
    /// run별 실행 그래프.
    pub graphs: Vec<TypedGraph>,
    /// 매개변수 포트로 독립 검사한 graph 정의.
    pub templates: Vec<TypedGraph>,
}

impl Compilation {
    /// run을 원천 레코드 상한 또는 파일 EOF까지 처리하는 실행 계획으로 낮춘다.
    pub fn execution_plan(
        &self,
        entry: Option<&str>,
        samples: Option<u64>,
    ) -> Result<bci_ir::plan::ExecutionPlan, Diagnostic> {
        plan::lower(self, entry, samples)
    }
    /// run 그래프에 남은 bind/run 의무의 수.
    pub fn pending_count(&self) -> usize {
        self.graphs.iter().map(|g| g.obligations.len()).sum()
    }

    /// 각 포트의 타입·효과·연결과 추가 확인 의무를 설명한다.
    pub fn explain_types(&self) -> String {
        let mut text = String::new();
        for (kind, graphs) in [("graph 정의", &self.templates), ("run", &self.graphs)] {
            for graph in graphs {
                let _ = writeln!(text, "{kind} {} [{}]", graph.entry, graph.effect.mode);
                for (name, value) in &graph.configuration {
                    let _ = writeln!(text, "  configure {name}: {}", value.describe());
                }
                for node in &graph.nodes {
                    let inputs = node
                        .inputs
                        .iter()
                        .map(|id| format!("%{id}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let _ = writeln!(
                        text,
                        "  %{} = {}({inputs}) [{}]",
                        node.id, node.operator, node.effect.mode
                    );
                    if let Some(output) = &node.output {
                        let _ = writeln!(text, "    {output}");
                    }
                    for (name, value) in &node.parameters {
                        let _ = writeln!(text, "    {name}: {}", value.describe());
                    }
                }
                for (name, node) in &graph.bindings {
                    let _ = writeln!(text, "  {name} -> %{node}");
                }
                for obligation in &graph.obligations {
                    let _ = writeln!(
                        text,
                        "  Pending #{} %{} {}: {}",
                        obligation.id, obligation.node, obligation.kind, obligation.predicate
                    );
                }
            }
        }
        text
    }
}

/// 소스를 검사하고 artifact 경로를 base에 상대적으로 해석한다.
pub fn compile(source: &str, base: &Path) -> Result<Compilation, Vec<Diagnostic>> {
    compile_with_extensions(source, base, &[])
}

/// 정적 번들의 확장 선언을 더해 이름·타입을 검사한다.
pub fn compile_with_extensions(
    source: &str,
    base: &Path,
    extensions: &[bci_schema::extension::OperatorSpec],
) -> Result<Compilation, Vec<Diagnostic>> {
    bci_schema::extension::validate_registry(extensions).map_err(|e| {
        vec![Diagnostic::new(
            "E0510",
            e,
            bci_schema::types::Span::default(),
        )]
    })?;
    let program = parser::parse(source).map_err(|error| vec![error])?;
    let (graphs, templates) = check::check(&program, base, extensions)?;
    Ok(Compilation {
        module: program.module,
        graphs,
        templates,
    })
}
