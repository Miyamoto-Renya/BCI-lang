//! BCi 정적 검사·타입 설명·유한 실행 명령의 진입점.

mod execute;
mod learn;

use bci_extension::Extensions;
use std::{io::Read, path::Path, process::ExitCode};

/// 표준·사용자 정적 번들이 공유하는 CLI 진입점.
pub fn main(extensions: &Extensions) -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match args.as_slice() {
        [] => {
            help();
            ExitCode::SUCCESS
        }
        [flag] if flag == "--help" || flag == "-h" => {
            help();
            ExitCode::SUCCESS
        }
        [flag] if flag == "--version" || flag == "-V" => {
            println!(
                "bci {} ({})",
                env!("CARGO_PKG_VERSION"),
                bci_schema::LANGUAGE_VERSION
            );
            ExitCode::SUCCESS
        }
        [command] if command == "extensions" => {
            for spec in extensions.specifications() {
                println!(
                    "{}@{} [{}] quantity={}\n  contract={}\n  function={}\n  reference={}\n  Rust {} ({})",
                    spec.name,
                    spec.version,
                    spec.profile,
                    spec.quantity,
                    spec.contract_digest,
                    spec.function_digest,
                    spec.reference_digest,
                    spec.rust_symbol,
                    spec.rust_source
                );
            }
            println!(
                "등록한 사용자 연산자 {}개",
                extensions.specifications().len()
            );
            ExitCode::SUCCESS
        }
        [command, action] if command == "extensions" && action == "check" => {
            match extensions.verify_references() {
                Ok(count) => {
                    println!("확장 수치 참조 통과: {count}개 exact-bit 입력/출력");
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("E0510: {error}");
                    ExitCode::from(4)
                }
            }
        }
        [command, options @ ..] if command == "learn" => learn::run(options),
        [command, path] if command == "check" => process(Path::new(path), false, extensions),
        [command, path, options @ ..] if command == "run" => {
            execute::run(Path::new(path), options, extensions)
        }
        [command, path, flag, view]
            if command == "explain" && flag == "--view" && view == "types" =>
        {
            process(Path::new(path), true, extensions)
        }
        _ => {
            eprintln!(
                "명령 인수를 확인하세요. bci check 파일.bci, bci run 파일.bci 또는 bci explain 파일.bci --view types를 사용합니다."
            );
            ExitCode::from(2)
        }
    }
}

fn process(path: &Path, explain: bool, extensions: &Extensions) -> ExitCode {
    let compilation = match load(path, extensions) {
        Ok((_, compiled)) => compiled,
        Err(code) => return code,
    };
    if explain {
        print!("{}", compilation.explain_types());
    } else {
        println!(
            "{}: 정적 검사 통과 — graph 정의 {}개, run {}개, 노드 {}개, Pending {}개",
            path.display(),
            compilation.templates.len(),
            compilation.graphs.len(),
            compilation
                .graphs
                .iter()
                .map(|g| g.nodes.len())
                .sum::<usize>(),
            compilation.pending_count()
        );
        for graph in &compilation.graphs {
            for obligation in &graph.obligations {
                println!(
                    "  {} Pending #{} {}: {}",
                    graph.entry, obligation.id, obligation.kind, obligation.predicate
                );
            }
        }
    }
    ExitCode::SUCCESS
}

fn load(
    path: &Path,
    extensions: &Extensions,
) -> Result<(String, bci_compiler::Compilation), ExitCode> {
    let source = (|| -> std::io::Result<String> {
        let file = std::fs::File::open(path)?;
        let mut source = String::new();
        file.take(1_048_577).read_to_string(&mut source)?;
        Ok(source)
    })();
    let source = match source {
        Ok(source) => source,
        Err(error) => {
            eprintln!("{}: E0902: 소스 읽기 실패: {error}", path.display());
            return Err(ExitCode::from(5));
        }
    };
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    match bci_compiler::compile_with_extensions(&source, base, extensions.specifications()) {
        Ok(compilation) => Ok((source, compilation)),
        Err(diagnostics) => {
            let io_error = diagnostics.iter().any(|error| error.code == "E0902");
            for error in diagnostics {
                eprintln!("{}", error.render(&path.display().to_string(), &source));
            }
            Err(ExitCode::from(if io_error { 5 } else { 2 }))
        }
    }
}

fn help() {
    println!(
        "BCi 언어\n\n사용법:\n  bci check 파일.bci\n  bci explain 파일.bci --view types\n  bci run 파일.bci [--samples N] [--entry 이름] [--output 디렉터리]\n                    [--checkpoint-out 파일] [--resume 파일]\n  bci learn fit DATASET --output DIRECTORY [--protocol FILE]\n  bci learn select DATASET --candidate MODEL --candidate MODEL --output DIRECTORY [--protocol FILE]\n  bci learn evaluate MODEL DATASET --output FILE [--protocol FILE]\n  bci extensions [check]\n  bci --help\n  bci --version\n\ncheck는 소스의 타입·효과·연결과 후속 의무를 검사합니다.\nrun은 합성 입력의 기본 1000표본 또는 파일 EOF까지 처리합니다. --samples는 동기 또는 시각 병합 입력 묶음의 상한입니다.\nextensions는 번들의 사용자 함수를 표시하며 check는 수치 참조를 실행합니다.\n사용법: docs/compiler.md, docs/runtime.md, docs/inputs.md, docs/extensions.md, docs/checkpoints.md, docs/multi-input.md"
    );
}
