use bci_artifact::recording::RunWriter;
use bci_runtime::{
    ExecutionPlan, Frame, InputError, InputSource, Recorder, RunReport, SourceFactory, SourceKind,
    SourceSpec,
};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::atomic::AtomicBool,
};

struct DiskRecorder(RunWriter);
struct Sources;
impl SourceFactory for Sources {
    fn open(&self, spec: &SourceSpec) -> Result<Box<dyn InputSource>, InputError> {
        match spec.kind {
            SourceKind::Synthetic => bci_operators::SyntheticSources.open(spec),
            _ => bci_artifact::input::FileSources.open(spec),
        }
    }
}
impl Recorder for DiskRecorder {
    fn prepare(&mut self, plan: &ExecutionPlan) -> Result<(), String> {
        self.0.prepare(plan)
    }
    fn write(&mut self, sink: usize, source: usize, frame: &Frame) -> Result<(), String> {
        self.0.write(sink, source, frame)
    }
    fn finish(&mut self, report: &RunReport) -> Result<(), String> {
        self.0.finish(report)
    }
}

pub(crate) fn run(
    path: &Path,
    options: &[OsString],
    extensions: &bci_extension::Extensions,
) -> ExitCode {
    let mut samples = None;
    let mut resume = None;
    let mut checkpoint_out = None;
    let mut entry = None;
    let mut output = None;
    let mut flags = std::collections::BTreeSet::new();
    for option in options.chunks(2) {
        if option.len() != 2 || !flags.insert(option[0].clone()) {
            eprintln!(
                "실행 옵션은 --samples N, --entry 이름, --output 디렉터리, --resume 파일, --checkpoint-out 파일을 각각 한 번 지정합니다."
            );
            return ExitCode::from(2);
        }
        match option[0].to_str() {
            Some("--resume") => resume = Some(PathBuf::from(&option[1])),
            Some("--checkpoint-out") => checkpoint_out = Some(PathBuf::from(&option[1])),
            Some("--samples") => match option[1].to_str().and_then(|s| s.parse::<u64>().ok()) {
                Some(n) if (1..=1_000_000_000).contains(&n) => samples = Some(n),
                _ => {
                    eprintln!("--samples는 1..1000000000 범위의 정수를 요구합니다.");
                    return ExitCode::from(2);
                }
            },
            Some("--entry") => match option[1].to_str() {
                Some(name) => entry = Some(name),
                None => {
                    eprintln!("--entry는 UTF-8 run 이름을 요구합니다.");
                    return ExitCode::from(2);
                }
            },
            Some("--output") => output = Some(PathBuf::from(&option[1])),
            _ => {
                eprintln!("실행 옵션을 확인하세요. bci --help에서 지원 옵션을 조회합니다.");
                return ExitCode::from(2);
            }
        }
    }
    let (source, compilation) = match crate::load(path, extensions) {
        Ok(value) => value,
        Err(code) => return code,
    };
    let plan = match compilation.execution_plan(entry, samples) {
        Ok(plan) => plan,
        Err(error) => {
            eprintln!("{}", error.render(&path.display().to_string(), &source));
            return ExitCode::from(2);
        }
    };
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    let mut recorder = DiskRecorder(RunWriter::new(base, output.as_deref()));
    let saved = match resume
        .as_deref()
        .map(bci_artifact::checkpoint::load)
        .transpose()
    {
        Ok(saved) => saved,
        Err(error) => {
            eprintln!("{}: {}", error.code, error.message);
            return ExitCode::from(if error.code == "E0902" { 5 } else { 4 });
        }
    };
    let (report, checkpoint) = bci_runtime::run_with_checkpoint(
        &plan,
        &extensions.kernels(&bci_operators::StandardKernels),
        &Sources,
        &mut recorder,
        &AtomicBool::new(false),
        bci_runtime::CheckpointOptions {
            resume: saved.as_ref(),
            save: checkpoint_out.is_some(),
        },
    );
    let records: u64 = plan
        .nodes
        .iter()
        .filter(|n| n.output.is_none())
        .map(|n| report.nodes[n.id].records)
        .sum();
    if let Some(error) = &report.failure {
        let diagnostic = bci_compiler::Diagnostic::new(error.code, &error.message, error.span);
        eprintln!(
            "{}",
            diagnostic.render(&path.display().to_string(), &source)
        );
        eprintln!(
            "실행 종료: 입력 묶음 {}/{}개 처리, 기록 수락 {}개",
            report.completed_samples,
            report
                .requested_samples
                .map_or_else(|| "EOF".into(), |n| n.to_string()),
            records
        );
    } else {
        let throughput = if report.elapsed_seconds > 0.0 {
            report.completed_samples as f64 / report.elapsed_seconds
        } else {
            0.0
        };
        println!(
            "{}: 실행 완료 — 입력 묶음 {}개, 기록 {}개, 예약 {} bytes",
            plan.entry, report.completed_samples, records, report.reserved_bytes
        );
        if report.pending_records > 0 {
            println!(
                "  합류 대기: {}개 — checkpoint에 저장해 다음 입력과 이어 처리한다.",
                report.pending_records
            );
        }
        println!(
            "  입력 종료: {}, Gap {}개·누락 {}표본",
            if report.end_of_input {
                "EOF"
            } else {
                "표본 상한"
            },
            report.gaps,
            report.missing_samples
        );
        println!(
            "  논리 시간 실행: {:.6}초 경과, {:.1} 입력 묶음/초",
            report.elapsed_seconds, throughput
        );
        for obligation in &plan.obligations {
            let stats = &report.nodes[obligation.node];
            match obligation.kind.as_str() {
                "WindowPhase" => match stats.first_time {
                    Some(time) => println!("  WindowPhase Pass: 첫 창 anchor {time}s"),
                    None => println!("  WindowPhase Unobserved: 입력 종료 시 완성 창 0개"),
                },
                "NumericGuard" => {
                    println!("  NumericGuard: Cholesky 통과 {}개", stats.valid_records)
                }
                "Storage" => println!("  Storage Committed: 기록 {}개", stats.records),
                "Input" => println!(
                    "  Input {}: {}레코드 검사",
                    if report.end_of_input {
                        "Pass"
                    } else {
                        "Guarded"
                    },
                    stats.records
                ),
                _ => {}
            }
        }
    }
    for directory in recorder.0.directories() {
        println!("  기록: {}", directory.display());
    }
    if let (Some(path), Some(checkpoint)) = (checkpoint_out, checkpoint) {
        if let Err(error) = bci_artifact::checkpoint::save(&path, &checkpoint) {
            eprintln!("E0902: {error}");
            return ExitCode::from(5);
        }
        println!(
            "  checkpoint: {} (각 원천의 다음 레코드 위치 {})",
            path.display(),
            checkpoint.position
        );
    }
    if report.input_offset > 0 {
        println!("  복원 시작 위치: {}", report.input_offset);
    }
    match report.failure {
        Some(error) => ExitCode::from(if error.code == "E0902" { 5 } else { 4 }),
        None => ExitCode::SUCCESS,
    }
}
