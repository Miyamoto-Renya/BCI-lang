//! 실행 데이터와 커널 정체성을 JSON Lines·종료 manifest에 기록한다.

use bci_ir::plan::ExecutionPlan;
use bci_schema::{
    execution::{Frame, RunError, RunReport},
    input::SourceKind,
    kernel::{KernelSpec, RECORD_BUFFER_BYTES},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static SERIAL: AtomicU64 = AtomicU64::new(0);

struct Sink {
    writer: Option<BufWriter<File>>,
    path: PathBuf,
    records: u64,
}

/// 하나의 run이 소유하는 기록 파일 집합.
pub struct RunWriter {
    base: PathBuf,
    output: Option<PathBuf>,
    sinks: BTreeMap<usize, Sink>,
    directories: Vec<PathBuf>,
    manifest: Value,
}

impl RunWriter {
    /// to 경로 기준 디렉터리와 CLI 출력 경로 재지정을 설정한다.
    pub fn new(base: &Path, output: Option<&Path>) -> Self {
        Self {
            base: base.to_owned(),
            output: output.map(Path::to_owned),
            sinks: BTreeMap::new(),
            directories: Vec::new(),
            manifest: Value::Null,
        }
    }

    /// 새 run 디렉터리, 데이터 파일과 준비 manifest를 만든다.
    pub fn prepare(&mut self, plan: &ExecutionPlan) -> Result<(), String> {
        if !self.directories.is_empty() {
            return Err("RunWriter 인스턴스마다 한 run을 준비한다.".into());
        }
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let run_id = format!(
            "run-{stamp}-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        );
        let mut groups = BTreeMap::new();
        self.manifest = manifest(plan, &run_id);
        self.manifest["executable_sha256"] = json!(executable_digest()?);
        for node in &plan.nodes {
            if let KernelSpec::Record { destination } = &node.kernel {
                let parent = self
                    .output
                    .clone()
                    .unwrap_or_else(|| self.base.join(destination));
                let directory = if let Some(dir) = groups.get(&parent) {
                    PathBuf::clone(dir)
                } else {
                    fs::create_dir_all(&parent).map_err(|e| {
                        format!(
                            "{} 생성 실패: {e}. 출력 경로와 쓰기 권한을 확인한다.",
                            parent.display()
                        )
                    })?;
                    let path = parent.join(&run_id);
                    fs::create_dir(&path)
                        .map_err(|e| format!("{} 생성 실패: {e}", path.display()))?;
                    self.directories.push(path.clone());
                    groups.insert(parent, path.clone());
                    path
                };
                let path = directory.join(format!("sink-{}.jsonl", node.id));
                let file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .map_err(|e| format!("{} 열기 실패: {e}", path.display()))?;
                let mut writer = BufWriter::with_capacity(RECORD_BUFFER_BYTES, file);
                let header = json!({"kind":"header","format":"bci-recording/0.6","entry":plan.entry,"sink":node.id,"ports":node.inputs,"origin":"PerRecord","layout":"channel-major","epoch":0});
                serde_json::to_writer(&mut writer, &header).map_err(|e| e.to_string())?;
                writeln!(writer)
                    .and_then(|_| writer.flush())
                    .and_then(|_| writer.get_ref().sync_all())
                    .map_err(|e| format!("{} 준비 기록 실패: {e}", path.display()))?;
                self.sinks.insert(
                    node.id,
                    Sink {
                        writer: Some(writer),
                        path,
                        records: 0,
                    },
                );
            }
        }
        self.manifest["status"] = json!("Preparing");
        for dir in &self.directories {
            commit_manifest(dir, &self.manifest)?;
        }
        Ok(())
    }

    /// 이미 예약한 writer에 데이터 한 개를 스트리밍 직렬화한다.
    pub fn write(&mut self, sink: usize, source: usize, frame: &Frame) -> Result<(), String> {
        let sink = self
            .sinks
            .get_mut(&sink)
            .ok_or("기록 sink를 먼저 준비한다.")?;
        let writer = sink.writer.as_mut().ok_or("열린 기록 파일을 요구한다.")?;
        let write = (|| -> Result<(), Box<dyn std::error::Error>> {
            write!(
                writer,
                "{{\"kind\":\"data\",\"port\":{source},\"sequence\":{},\"time_seconds\":\"{}\",\"source_range\":[{},{}],\"generation\":{},\"origin\":\"{:?}\",\"gap\":",
                frame.sequence,
                frame.time,
                frame.source_first,
                frame.source_last,
                frame.generation,
                frame.origin
            )?;
            serde_json::to_writer(&mut *writer, &frame.gap.map(|g| [g.first, g.last]))?;
            write!(writer, ",\"flags\":{},\"valid\":", frame.flags)?;
            serde_json::to_writer(&mut *writer, &frame.valid)?;
            write!(writer, ",\"values\":")?;
            serde_json::to_writer(&mut *writer, &frame.values)?;
            writeln!(writer, "}}")?;
            Ok(())
        })();
        write.map_err(|e| {
            format!(
                "{} 기록 실패: {e}. 출력 공간과 파일 시스템 상태를 확인한다.",
                sink.path.display()
            )
        })?;
        sink.records += 1;
        Ok(())
    }

    /// 모든 데이터를 sync한 후 최종 관측·의무 판정 manifest를 게시한다.
    pub fn finish(&mut self, report: &RunReport) -> Result<(), String> {
        let mut failure = None;
        for sink in self.sinks.values_mut() {
            if let Some(mut writer) = sink.writer.take()
                && let Err(error) = writer.flush().and_then(|_| writer.get_ref().sync_all())
            {
                failure.get_or_insert_with(|| {
                    format!(
                        "{} commit 실패: {error}. 출력 공간과 파일 시스템 상태를 확인한다.",
                        sink.path.display()
                    )
                });
            }
        }
        let mut report = report.clone();
        if let Some(message) = &failure {
            report.failure.get_or_insert(RunError {
                code: "E0902",
                node: None,
                span: Default::default(),
                message: message.clone(),
            });
        }
        self.manifest["status"] = json!(if report.failure.is_some() {
            "Faulted"
        } else if report.stopped {
            "Stopped"
        } else {
            "Closed"
        });
        self.manifest["report"] = report_json(&report);
        if let Some(obligations) = self.manifest["obligations"].as_array_mut() {
            for obligation in obligations {
                let node = obligation["node"].as_u64().unwrap_or(u64::MAX) as usize;
                let Some(stats) = report.nodes.get(node) else {
                    continue;
                };
                let failed = report
                    .failure
                    .as_ref()
                    .is_some_and(|e| e.node == Some(node));
                obligation["evidence"] = match obligation["kind"].as_str() {
                    Some("WindowPhase") => {
                        json!({"status":if stats.first_time.is_some(){"Pass"}else{"Unobserved"},"anchor_seconds":stats.first_time.map(|t|t.to_string()),"first_sequence":stats.first_sequence,"generations":stats.generations,"last_generation":stats.last_generation,"last_generation_anchor":stats.generation_first_time.map(|t|t.to_string())})
                    }
                    Some("Resampling") => {
                        json!({"status":if failed{"Fail"}else{"Guarded"},"records":stats.records,"grid":"exact_rational","support":"causal_polyphase","initial_state":"zero_invalid"})
                    }
                    Some("NumericGuard") => {
                        json!({"status":if failed{"Fail"}else{"Guarded"},"cholesky_passes":stats.valid_records})
                    }
                    Some("Storage") => {
                        let storage_failed = failure.is_some()
                            || report.failure.as_ref().is_some_and(|e| {
                                e.code == "E0902" && (e.node.is_none() || e.node == Some(node))
                            });
                        json!({"status":if storage_failed{"Fail"}else{"Committed"},"records":self.sinks.get(&node).map_or(0,|s|s.records),"backpressure":"synchronous","capacity_monitor":"write/flush/sync errors"})
                    }
                    Some("Input") => {
                        json!({"status":if failed {"Fail"}else if report.end_of_input {"Pass"}else{"Guarded"},"records":stats.records,"gaps":report.gaps,"missing_samples":report.missing_samples})
                    }
                    _ => json!({"status":"Pending"}),
                };
            }
        }
        for directory in &self.directories {
            if let Err(error) = commit_manifest(directory, &self.manifest) {
                failure.get_or_insert(error);
            }
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// 이번 실행이 생성한 기록 디렉터리.
    pub fn directories(&self) -> &[PathBuf] {
        &self.directories
    }
}

fn commit_manifest(directory: &Path, manifest: &Value) -> Result<(), String> {
    let pending = directory.join(format!(
        "manifest-{}.tmp",
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&pending)
        .map_err(|e| e.to_string())?;
    serde_json::to_writer_pretty(&mut file, manifest).map_err(|e| e.to_string())?;
    file.write_all(b"\n")
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    fs::rename(&pending, directory.join("manifest.json"))
        .map_err(|e| format!("{} manifest commit 실패: {e}", directory.display()))?;
    #[cfg(unix)]
    File::open(directory)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn manifest(plan: &ExecutionPlan, run_id: &str) -> Value {
    let mut source_ports: Vec<Vec<usize>> = Vec::new();
    for node in &plan.nodes {
        let sources = if matches!(node.kernel, KernelSpec::Source(_)) {
            vec![node.id]
        } else {
            node.inputs
                .iter()
                .flat_map(|&id| source_ports[id].iter().copied())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect()
        };
        source_ports.push(sources);
    }
    json!({
        "format":"bci-run/0.6","run_id":run_id,"entry":plan.entry,
        "sources":plan.sources,"input_policy":if plan.synchronous_inputs(){"SynchronousExact"}else{"UniformTimeMerge"},
        "bundle":"bci-static-f64/0.10","standard_kernels":"bci-standard-f64/0.8","timing":"logical-unpaced","origin":"PerRecord","epoch":0,
        "queue_policy":"Fault","queue_capacities":plan.queues.capacities,"queue_reserved_bytes":plan.queues.bytes,
        "samples":plan.samples,"memory_budget":plan.memory_budget,"reserved_bytes":plan.reserved_bytes,"schedule":plan.schedule,
        "nodes":plan.nodes.iter().map(|node|{
            let output=node.output.as_ref().map(crate::stream_codec::encode);
            json!({"id":node.id,"operator":node.operator,"inputs":node.inputs,"source_ports":source_ports[node.id],"source_span":[node.span.start,node.span.end],"kernel":kernel_json(&node.kernel),"output":output,"buffer_bytes":node.buffer_bytes,"state_bytes":node.state_bytes})
        }).collect::<Vec<_>>(),
        "obligations":plan.obligations.iter().map(|o|json!({"id":o.id,"node":o.node,"kind":o.kind,"predicate":o.predicate})).collect::<Vec<_>>()
    })
}

/// 실제 실행 파일 전체의 SHA-256을 계산한다.
pub fn executable_digest() -> Result<String, String> {
    let path = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65_536];
    loop {
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn kernel_json(kernel: &KernelSpec) -> Value {
    match kernel {
        KernelSpec::Psd {
            channels,
            samples,
            rate,
            taper,
        } => {
            json!({"kind":"psd","channels":channels,"samples":samples,"rate_hz":rate.to_string(),"taper":format!("{taper:?}"),"normalization":"OneSidedDensity","kernel":"direct-dft-table/0.1","detrend":"None","quality":"per_channel_require_all"})
        }
        KernelSpec::BandPower {
            channels,
            samples,
            rate,
            band,
        } => {
            json!({"kind":"bandpower","channels":channels,"samples":samples,"rate_hz":rate.to_string(),"low_hz":band.low.to_string(),"high_hz":band.high.to_string(),"include_high":band.include_high})
        }
        KernelSpec::LogPower {
            channels,
            floor,
            reference,
            unit,
        } => {
            json!({"kind":"log_power","channels":channels,"floor":floor.to_string(),"reference":reference.to_string(),"unit":unit.to_string(),"formula":"ln((E+floor)/reference)"})
        }
        KernelSpec::ChannelMap {
            input_channels,
            indices,
            width,
        } => {
            json!({"kind":"channel_map","input_channels":input_channels,"indices":indices,"width":width,"quality":"selected_channels","invalid_payload":"preserve"})
        }
        KernelSpec::Rereference { weights, width } => {
            json!({"kind":"rereference","weights":weights.iter().map(ToString::to_string).collect::<Vec<_>>(),"width":width,
                "formula":"A=I-1*w^T","coefficient_rounding":"exact_rational_to_f64","order":"input_channel_order","quality":"nonzero_matrix_coefficients","invalid_payload":0})
        }
        KernelSpec::Mix { weights, channels } => {
            json!({"kind":"mix","weights":weights,"channels":channels,"order":"input_order","quality":"require_all","invalid_payload":0})
        }
        KernelSpec::Extension { operator, channels } => json!({
            "kind":if operator.state.is_some(){"stateful"}else{"pointwise"},"channels":channels,"name":operator.name,"version":operator.version,
            "profile":operator.profile,"quantity":operator.quantity,"contract_digest":operator.contract_digest,
            "function_digest":operator.function_digest,"reference_digest":operator.reference_digest,
            "rust_symbol":operator.rust_symbol,"rust_source":operator.rust_source,
            "quality":"preserve_mask_invalid_payload","support":if operator.state.is_some(){"since_reset"}else{"current_sample"},"lookahead":0,
            "state":operator.state.as_ref().map(|s|json!({"codec":s.codec,"fixed_bytes":s.fixed_bytes,"channel_bytes":s.channel_bytes,"scratch_bytes":s.scratch_bytes,"checkpoint_bytes":s.checkpoint_bytes,"checkpoint_channel_bytes":s.checkpoint_channel_bytes}))
        }),
        KernelSpec::Predict { model } => {
            json!({"kind":"predict","activation":"Prepare","model":crate::learning::model_json(model)})
        }
        KernelSpec::Threshold { classes, minimum } => {
            json!({"kind":"threshold","classes":classes,"minimum":minimum,"select":"first_maximum","hold_index":-1,"invalid":"HoldInvalid"})
        }
        KernelSpec::Source(spec) => match &spec.kind {
            SourceKind::Synthetic => {
                json!({"kind":"synthetic","formula":"sin(2*pi*(10+3*c)*t)+0.25*cos(2*pi*3*t+c)"})
            }
            SourceKind::Csv { path, gap, invalid } => {
                json!({"kind":"csv","path":path.display().to_string(),"gap":format!("{gap:?}"),"invalid":format!("{invalid:?}"),"row_bytes":spec.row_bytes})
            }
            SourceKind::Replay { path, port, gap } => {
                json!({"kind":"replay","path":path.display().to_string(),"port":port,"gap":format!("{gap:?}"),"row_bytes":spec.row_bytes})
            }
        },
        KernelSpec::Scale { factors, width } => {
            json!({"kind":"scale","factors":factors,"width":width})
        }
        KernelSpec::Resample {
            channels,
            specification: s,
        } => {
            json!({"kind":"resample","kernel":"causal-polyphase/0.1","codec":"resample-state/0.1","channels":channels,"up":s.up,"down":s.down,"filter":s.coefficients,"input_rate_hz":s.rate.to_string(),"input_phase_seconds":s.input_phase.to_string(),"phase_seconds":s.phase.to_string(),"initial_state":"zero_invalid","gap":"ResetHistory","tail":"Discard","gain":"explicit_coefficients","filter_validation":"finite_causal_support"})
        }
        KernelSpec::Iir {
            channels,
            realization,
        } => {
            json!({"kind":"iir","channels":channels,"kernel":"sos-df2t/0.1","codec":"sos-state/0.1",
            "rate_hz":realization.rate.to_string(),"warmup_samples":realization.warmup,"sections":realization.sections.iter().map(|s|s.iter().map(ToString::to_string).collect::<Vec<_>>()).collect::<Vec<_>>(),
            "coefficient_order":["b0","b1","b2","a1","a2"],"a0":"1","realization_sha256":realization.digest(),
            "stability":"strict_schur","f64_schur_margin":bci_schema::iir::SCHUR_MARGIN,"initial_state":"zero","invalid":"ResetChannel","gap":"ResetAll","support":"since_global_reset"})
        }
        KernelSpec::Fir {
            channels,
            coefficients,
        } => json!({"kind":"fir","channels":channels,"coefficients":coefficients,"warmup":"Full"}),
        KernelSpec::Window {
            channels,
            size,
            hop,
            start,
        } => {
            json!({"kind":"window","channels":channels,"size":size,"hop":hop,"start":start,"tail":"Discard"})
        }
        KernelSpec::Covariance { channels, samples } => {
            json!({"kind":"covariance","channels":channels,"samples":samples,"normalization":"N-1"})
        }
        KernelSpec::Regularize {
            channels,
            shrinkage,
            floor,
        } => {
            json!({"kind":"regularize","channels":channels,"shrinkage":shrinkage,"floor":floor,"guard":"Cholesky"})
        }
        KernelSpec::Record { destination } => json!({"kind":"record","destination":destination}),
    }
}

fn report_json(report: &RunReport) -> Value {
    json!({"pending_records":report.pending_records,"input_offset":report.input_offset,"requested_samples":report.requested_samples,"completed_samples":report.completed_samples,"reserved_bytes":report.reserved_bytes,
        "end_of_input":report.end_of_input,"gaps":report.gaps,"missing_samples":report.missing_samples,"elapsed_seconds":report.elapsed_seconds,"source_samples_per_second":if report.elapsed_seconds>0.0 {report.completed_samples as f64/report.elapsed_seconds}else{0.0},
        "stopped":report.stopped,"states":report.states.iter().map(|s|format!("{s:?}")).collect::<Vec<_>>(),
        "nodes":report.nodes.iter().enumerate().map(|(id,n)|json!({"id":id,"generations":n.generations,"last_generation":n.last_generation,"generation_first_time_seconds":n.generation_first_time.map(|t|t.to_string()),"records":n.records,"valid_records":n.valid_records,"first_time_seconds":n.first_time.map(|t|t.to_string()),"last_time_seconds":n.last_time.map(|t|t.to_string()),"first_sequence":n.first_sequence,"last_sequence":n.last_sequence})).collect::<Vec<_>>(),
        "failure":report.failure.as_ref().map(|e|json!({"code":e.code,"node":e.node,"message":e.message,"source_span":[e.span.start,e.span.end]}))})
}
