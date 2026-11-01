//! 실행 파일·내용 digest를 대조하는 checkpoint 파일 codec.
use bci_ir::checkpoint::{Checkpoint, MAX_STATE_BYTES};
use bci_schema::input::{GridCursor, exact_time};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};
static SERIAL: AtomicU64 = AtomicU64::new(0);

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn unhex(value: &Value, limit: usize) -> Result<Vec<u8>, String> {
    let value = value.as_str().ok_or("hex 문자열을 요구한다.")?;
    if value.len() > limit.saturating_mul(2)
        || !value.len().is_multiple_of(2)
        || !value.bytes().all(|c| c.is_ascii_hexdigit())
    {
        return Err("checkpoint hex 길이·문자를 확인한다.".into());
    }
    (0..value.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&value[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}
fn hash(value: &Value) -> Result<String, String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).map_err(|e| e.to_string())?)
    ))
}

/// 기록 commit 이후 checkpoint를 새 경로에 원자적으로 게시한다.
pub fn save(path: &Path, state: &Checkpoint) -> Result<(), String> {
    if state
        .kernels
        .iter()
        .map(Vec::len)
        .sum::<usize>()
        .saturating_add(state.scheduler.len())
        > MAX_STATE_BYTES
        || state.kernels.len() > 65536
        || state.grids.len() != state.kernels.len()
    {
        return Err("checkpoint payload·노드 수 상한을 맞춘다.".into());
    }
    let payload = json!({"plan":hex(&state.plan_digest),"prefix":hex(&state.prefix_digest),"position":state.position,
        "scheduler":hex(&state.scheduler),
        "kernels":state.kernels.iter().map(|v|hex(v)).collect::<Vec<_>>(),
        "grids":state.grids.iter().map(|g|json!({"generation":g.generation,"phase":g.phase.map(|v|v.to_string()),"time":g.last_time.map(|v|v.to_string()),"index":g.last_index})).collect::<Vec<_>>()});
    let value = json!({"format":"bci-checkpoint/0.2","executable_sha256":crate::recording::executable_digest()?,"payload_sha256":hash(&payload)?,"payload":payload});
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let pending = parent.join(format!(
        ".bci-checkpoint-{}-{}.tmp",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&pending)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        serde_json::to_writer(&mut file, &value).map_err(|e| e.to_string())?;
        file.write_all(b"\n")
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        fs::hard_link(&pending, path).map_err(|e| {
            format!(
                "{}: checkpoint를 새 파일 경로에 저장한다: {e}",
                path.display()
            )
        })?;
        #[cfg(unix)]
        File::open(parent)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        Ok(())
    })();
    drop(file);
    let _ = fs::remove_file(pending);
    result
}

/// 같은 실행 파일의 완전한 checkpoint를 읽는다.
pub fn load(path: &Path) -> Result<Checkpoint, bci_schema::input::InputError> {
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|f| f.take(64 * 1024 * 1024 + 1).read_to_end(&mut bytes))
        .map_err(|e| {
            bci_schema::input::InputError::io(format!(
                "{}: {e}. checkpoint 경로와 읽기 권한을 확인한다.",
                path.display()
            ))
        })?;
    decode(&bytes).map_err(|message| bci_schema::input::InputError {
        code: "E0909",
        message: format!("{}: {message}", path.display()),
    })
}

fn decode(bytes: &[u8]) -> Result<Checkpoint, String> {
    if bytes.len() > 64 * 1024 * 1024 {
        return Err("checkpoint 파일 상한은 64 MiB다.".into());
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if value["format"] != "bci-checkpoint/0.2"
        || value["executable_sha256"] != crate::recording::executable_digest()?
        || value["payload_sha256"] != hash(&value["payload"])?
    {
        return Err("checkpoint 판본·실행 파일·내용 digest를 맞춘다.".into());
    }
    let payload = &value["payload"];
    let digest = |key: &str| -> Result<[u8; 32], String> {
        unhex(&payload[key], 32)?
            .try_into()
            .map_err(|_| "SHA-256은 32 bytes를 요구한다.".into())
    };
    let kernels = payload["kernels"]
        .as_array()
        .filter(|a| a.len() <= 65536)
        .ok_or("커널 payload 배열을 요구한다.")?;
    let grids = payload["grids"]
        .as_array()
        .filter(|a| a.len() == kernels.len())
        .ok_or("커널마다 격자를 지정한다.")?;
    let scheduler = unhex(&payload["scheduler"], MAX_STATE_BYTES)?;
    let mut total = scheduler.len();
    let kernels = kernels
        .iter()
        .map(|v| {
            let bytes = unhex(v, MAX_STATE_BYTES - total)?;
            total += bytes.len();
            Ok(bytes)
        })
        .collect::<Result<_, String>>()?;
    let grids = grids
        .iter()
        .map(|g| {
            let number = |key: &str| -> Result<Option<u64>, String> {
                if g[key].is_null() {
                    Ok(None)
                } else {
                    g[key]
                        .as_u64()
                        .map(Some)
                        .ok_or_else(|| format!("{key} 정수를 요구한다."))
                }
            };
            let time = |key: &str| {
                if g[key].is_null() {
                    Ok(None)
                } else {
                    exact_time(g[key].as_str().ok_or("시각 문자열을 요구한다.")?).map(Some)
                }
            };
            Ok(GridCursor {
                generation: number("generation")?,
                last_index: number("index")?,
                phase: time("phase")?,
                last_time: time("time")?,
            })
        })
        .collect::<Result<_, String>>()?;
    Ok(Checkpoint {
        plan_digest: digest("plan")?,
        prefix_digest: digest("prefix")?,
        position: payload["position"]
            .as_u64()
            .filter(|&v| v <= 1_000_000_000)
            .ok_or("원천 위치 범위를 확인한다.")?,
        kernels,
        grids,
        scheduler,
    })
}
