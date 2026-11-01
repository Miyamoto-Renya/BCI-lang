use crate::{input::Lines, stream_codec};
use bci_schema::{
    execution::{Frame, GAP, Gap, Origin},
    input::{
        GapPolicy, GridCursor, InputError, InputSource, SourceKind, SourceSpec, exact_time,
        row_bound,
    },
};
use serde_json::Value;
use std::{io::Read, path::Path};

/// commit된 기록의 선택 포트 타입과 reader 예약량을 읽는다.
pub fn inspect_replay(path: &Path, port: usize) -> Result<SourceSpec, InputError> {
    metadata(path, port).map(|m| m.spec)
}

struct Metadata {
    spec: SourceSpec,
    lines: Lines,
    ports: Vec<usize>,
    expected: u64,
    expected_valid: u64,
    legacy: bool,
    separate_indices: bool,
}
fn metadata(path: &Path, port: usize) -> Result<Metadata, InputError> {
    let manifest_path = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("manifest.json");
    let file = std::fs::File::open(&manifest_path)
        .map_err(|e| InputError::io(format!("{}: {e}", manifest_path.display())))?;
    let mut bytes = Vec::new();
    file.take(8_388_609)
        .read_to_end(&mut bytes)
        .map_err(|e| InputError::io(e.to_string()))?;
    if bytes.len() > 8_388_608 {
        return Err(InputError::contract("기록 manifest는 8 MiB 이하여야 한다."));
    }
    let manifest: Value = serde_json::from_slice(&bytes)
        .map_err(|e| InputError::contract(format!("{}: {e}", manifest_path.display())))?;
    let legacy = manifest["format"] == "bci-synthetic-run/0.1";
    if !legacy
        && manifest["format"] != "bci-run/0.2"
        && manifest["format"] != "bci-run/0.3"
        && manifest["format"] != "bci-run/0.4"
        && manifest["format"] != "bci-run/0.5"
        && manifest["format"] != "bci-run/0.6"
    {
        return Err(InputError::contract(
            "기록 manifest 판본은 bci-run/0.6·0.5·0.4·0.3·0.2 또는 bci-synthetic-run/0.1을 요구한다.",
        ));
    }
    if !matches!(manifest["status"].as_str(), Some("Closed" | "Stopped"))
        || !manifest["report"]["failure"].is_null()
    {
        return Err(InputError::contract(format!(
            "{}: 성공적으로 commit된 Closed·Stopped 기록을 선택한다.",
            manifest_path.display()
        )));
    }
    let mut lines = Lines::open(path)?;
    let header = lines
        .next(1_048_576, true)?
        .ok_or_else(|| InputError::contract("기록 header를 요구한다."))?;
    let header: Value = serde_json::from_str(header).map_err(|e| lines.error(e))?;
    let format = if legacy {
        "bci-synthetic-recording/0.1"
    } else if manifest["format"] == "bci-run/0.6" {
        "bci-recording/0.6"
    } else if manifest["format"] == "bci-run/0.5" {
        "bci-recording/0.5"
    } else if manifest["format"] == "bci-run/0.4" {
        "bci-recording/0.4"
    } else if manifest["format"] == "bci-run/0.3" {
        "bci-recording/0.3"
    } else {
        "bci-recording/0.2"
    };
    if header["kind"] != "header"
        || header["format"] != format
        || header["entry"] != manifest["entry"]
        || header["epoch"] != 0
    {
        return Err(lines.error("기록 header의 판본·entry·epoch를 manifest와 맞춘다."));
    }
    let ports = header["ports"]
        .as_array()
        .ok_or_else(|| lines.error("ports 배열을 요구한다."))?
        .iter()
        .map(|p| {
            p.as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| lines.error("정수 port ID를 요구한다."))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if ports.iter().filter(|&&p| p == port).count() != 1 {
        return Err(lines.error(format!("포트 {port}가 한 번 기록된 sink를 선택한다.")));
    }
    let nodes = manifest["nodes"]
        .as_array()
        .ok_or_else(|| lines.error("manifest nodes 배열을 요구한다."))?;
    let sink = header["sink"]
        .as_u64()
        .and_then(|id| nodes.iter().find(|n| n["id"].as_u64() == Some(id)))
        .ok_or_else(|| lines.error("manifest의 sink를 연결한다."))?;
    if sink["operator"] != "std.recording.write" || sink["inputs"] != header["ports"] {
        return Err(lines.error("sink 입력과 header ports를 맞춘다."));
    }
    let mut selected = None;
    let mut bound = 4096;
    for id in &ports {
        let node = nodes
            .iter()
            .find(|n| n["id"].as_u64() == Some(*id as u64))
            .ok_or_else(|| lines.error("기록 포트의 타입을 manifest에 지정한다."))?;
        let stream = stream_codec::decode(&node["output"]).map_err(|e| lines.error(e))?;
        let spec = SourceSpec {
            kind: SourceKind::Replay {
                path: path.into(),
                port: *id,
                gap: GapPolicy::Reset,
            },
            stream,
            row_bytes: 0,
        };
        let (values, channels) = spec.layout().map_err(|e| lines.error(e))?;
        bound = bound.max(row_bound(values, channels).map_err(|e| lines.error(e))?);
        if *id == port {
            selected = Some(spec);
        }
    }
    let mut spec = selected.ok_or_else(|| lines.error("선택한 포트 타입을 요구한다."))?;
    spec.row_bytes = bound;
    let stats = manifest["report"]["nodes"]
        .as_array()
        .and_then(|n| n.iter().find(|n| n["id"].as_u64() == Some(port as u64)))
        .ok_or_else(|| lines.error("선택 포트의 commit 관측값을 요구한다."))?;
    let expected = stats["records"]
        .as_u64()
        .ok_or_else(|| lines.error("commit 레코드 수를 요구한다."))?;
    let expected_valid = stats["valid_records"]
        .as_u64()
        .ok_or_else(|| lines.error("commit 유효 레코드 수를 요구한다."))?;
    lines.release_header();
    Ok(Metadata {
        spec,
        lines,
        ports,
        expected,
        expected_valid,
        legacy,
        separate_indices: manifest["format"] == "bci-run/0.5"
            || manifest["format"] == "bci-run/0.6",
    })
}

pub(crate) struct ReplaySource {
    meta: Metadata,
    cursor: GridCursor,
    read: u64,
    valid: u64,
    last_generation: Option<u64>,
    last_source: Option<u64>,
    last_sequence: Option<u64>,
}
impl ReplaySource {
    pub fn open(spec: &SourceSpec) -> Result<Self, InputError> {
        let SourceKind::Replay { path, port, gap } = &spec.kind else {
            return Err(InputError::contract("재생 원천 계약을 지정한다."));
        };
        let mut meta = metadata(path, *port)?;
        if meta.spec.stream.mismatch(&spec.stream).is_some()
            || meta.spec.row_bytes != spec.row_bytes
        {
            return Err(InputError::contract(
                "기록의 포트 타입·행 상한이 컴파일 입력과 달라졌다. 변경된 기록으로 다시 컴파일한다.",
            ));
        }
        if let SourceKind::Replay { gap: policy, .. } = &mut meta.spec.kind {
            *policy = *gap;
        }
        Ok(Self {
            meta,
            cursor: GridCursor::default(),
            read: 0,
            valid: 0,
            last_generation: None,
            last_source: None,
            last_sequence: None,
        })
    }
}
impl InputSource for ReplaySource {
    fn read(&mut self, output: &mut Frame) -> Result<bool, InputError> {
        let SourceKind::Replay { port, gap, .. } = self.meta.spec.kind else {
            unreachable!("replay")
        };
        loop {
            let Some(line) = self.meta.lines.next(self.meta.spec.row_bytes, true)? else {
                if self.read != self.meta.expected || self.valid != self.meta.expected_valid {
                    return Err(self.meta.lines.error(format!("commit 관측값은 {}개·유효 {}개, 파일은 {}개·유효 {}개다. 완결된 기록을 복원한다.",self.meta.expected,self.meta.expected_valid,self.read,self.valid)));
                }
                return Ok(false);
            };
            let row: Value = serde_json::from_str(line).map_err(|e| self.meta.lines.error(e))?;
            let selected = row["port"]
                .as_u64()
                .and_then(|p| usize::try_from(p).ok())
                .ok_or_else(|| self.meta.lines.error("data port를 요구한다."))?;
            if row["kind"] != "data" || !self.meta.ports.contains(&selected) {
                return Err(self
                    .meta
                    .lines
                    .error("header에 등록한 data 포트를 요구한다."));
            }
            if selected != port {
                continue;
            }
            let values = row["values"]
                .as_array()
                .ok_or_else(|| self.meta.lines.error("values 배열을 요구한다."))?;
            let valid = row["valid"]
                .as_array()
                .ok_or_else(|| self.meta.lines.error("valid 배열을 요구한다."))?;
            if values.len() != output.values.len() || valid.len() != output.valid.len() {
                return Err(self
                    .meta
                    .lines
                    .error("값·품질 배열 크기를 포트 shape와 맞춘다."));
            }
            for (to, from) in output.values.iter_mut().zip(values) {
                *to = from.as_f64().filter(|n| n.is_finite()).ok_or_else(|| {
                    self.meta
                        .lines
                        .error("유한 f64 값으로 기록한 파일을 요구한다.")
                })?;
            }
            for (to, from) in output.valid.iter_mut().zip(valid) {
                *to = from
                    .as_u64()
                    .filter(|&n| n <= 1)
                    .ok_or_else(|| self.meta.lines.error("valid는 0 또는 1을 요구한다."))?
                    as u8;
            }
            if matches!(
                self.meta.spec.stream.value,
                bci_schema::types::ValueType::Spectrum(..)
            ) && output.values.iter().any(|&v| v < 0.0)
            {
                return Err(self
                    .meta
                    .lines
                    .error("기록 PSD 밀도는 0 이상의 값을 요구한다."));
            }
            output.sequence = integer(&row, "sequence").map_err(|e| self.meta.lines.error(e))?;
            if output.sequence > i64::MAX as u64 {
                return Err(self
                    .meta
                    .lines
                    .error("sequence는 i64::MAX 이하를 요구한다."));
            }
            output.time = exact_time(
                stream_codec::text(&row, "time_seconds").map_err(|e| self.meta.lines.error(e))?,
            )
            .map_err(|e| self.meta.lines.error(e))?;
            let range = row["source_range"]
                .as_array()
                .filter(|v| v.len() == 2)
                .ok_or_else(|| {
                    self.meta
                        .lines
                        .error("source_range는 index 두 개를 요구한다.")
                })?;
            output.source_first = range[0]
                .as_u64()
                .ok_or_else(|| self.meta.lines.error("원천 index를 정수로 지정한다."))?;
            output.source_last = range[1]
                .as_u64()
                .ok_or_else(|| self.meta.lines.error("원천 index를 정수로 지정한다."))?;
            if output.source_first > output.source_last
                || (!self.meta.separate_indices && output.source_last > output.sequence)
            {
                return Err(self
                    .meta
                    .lines
                    .error("원천 범위를 anchor index 이내의 순서로 지정한다."));
            }
            output.flags = integer(&row, "flags")
                .ok()
                .filter(|&n| n <= 7)
                .ok_or_else(|| {
                    self.meta
                        .lines
                        .error("품질 flags는 등록한 0..7 비트 집합을 요구한다.")
                })? as u8;
            output.generation = if self.meta.legacy {
                0
            } else {
                integer(&row, "generation").map_err(|e| self.meta.lines.error(e))?
            };
            output.origin = if self.meta.legacy {
                Origin::Synthetic
            } else {
                match row["origin"].as_str() {
                    Some("Synthetic") => Origin::Synthetic,
                    Some("Measurement") => Origin::Measurement,
                    Some("Mixed") => Origin::Mixed,
                    _ => {
                        return Err(self
                            .meta
                            .lines
                            .error("origin을 Synthetic·Measurement·Mixed 중 하나로 지정한다."));
                    }
                }
            };
            output.gap = match row.get("gap") {
                None | Some(Value::Null) => None,
                Some(value) => {
                    let a = value.as_array().filter(|v| v.len() == 2).ok_or_else(|| {
                        self.meta.lines.error("gap은 누락 index 두 개를 요구한다.")
                    })?;
                    let first = a[0]
                        .as_u64()
                        .ok_or_else(|| self.meta.lines.error("gap 첫 index를 요구한다."))?;
                    let last = a[1]
                        .as_u64()
                        .filter(|&v| {
                            v >= first
                                && v < if self.meta.separate_indices {
                                    output.source_last
                                } else {
                                    output.sequence
                                }
                        })
                        .ok_or_else(|| {
                            self.meta
                                .lines
                                .error("gap 범위는 현재 anchor 이전의 순서 있는 index를 요구한다.")
                        })?;
                    if output.flags & GAP == 0 {
                        return Err(self
                            .meta
                            .lines
                            .error("명시적 gap에 GAP 품질 flag를 연결한다."));
                    }
                    Some(Gap { first, last })
                }
            };
            bci_schema::learning::typed_frame(&self.meta.spec.stream.value, output)
                .map_err(|e| self.meta.lines.error(e))?;
            let changed = self.last_generation.is_some_and(|g| g != output.generation);
            if self
                .last_sequence
                .is_some_and(|previous| output.sequence <= previous)
            {
                return Err(self
                    .meta
                    .lines
                    .error("sequence는 이전 원천 anchor보다 커야 한다."));
            }
            if let Some(range) = output.gap
                && ((if self.meta.separate_indices {
                    self.last_source
                } else {
                    self.last_sequence
                })
                .is_some_and(|previous| range.first <= previous)
                    || self.last_generation == Some(output.generation))
            {
                return Err(self
                    .meta
                    .lines
                    .error("gap은 이전 anchor 이후의 누락 범위와 새로운 generation을 요구한다."));
            }
            if gap == GapPolicy::Fault && (output.gap.is_some() || changed) {
                return Err(self.meta.lines.error(
                    "기록에서 Gap을 발견했다. Gap.Reset을 선택하거나 연속 기록을 사용한다.",
                ));
            }
            self.cursor
                .check(&self.meta.spec.stream.occurrence, output)
                .map_err(|e| self.meta.lines.error(e))?;
            self.last_generation = Some(output.generation);
            self.last_sequence = Some(output.sequence);
            self.last_source = Some(output.source_last);
            self.read += 1;
            self.valid += u64::from(output.valid.iter().all(|&v| v == 1));
            if self.read > self.meta.expected {
                return Err(self
                    .meta
                    .lines
                    .error("레코드 수가 commit 관측값을 초과했다."));
            }
            return Ok(true);
        }
    }
}
fn integer(value: &Value, key: &str) -> Result<u64, String> {
    value[key]
        .as_u64()
        .ok_or_else(|| format!("{key}는 u64 정수를 요구한다."))
}
