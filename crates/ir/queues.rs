//! 인과 DAG의 출력 burst와 합류 대기 큐 예약 계약.
use crate::plan::PlannedNode;
use bci_schema::{kernel::KernelSpec, number::Rational, types::Occurrence};

/// 노드·입력 인수 순서의 고정 ring 용량.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Queues {
    /// 각 입력 간선에 예약하는 레코드 수.
    pub capacities: Vec<Vec<usize>>,
    /// 큐 슬롯·codec·제어 상태의 전체 예약량.
    pub bytes: usize,
}
impl Queues {
    /// 고속 출력 burst와 경로별 시간 선행의 보수적 합으로 용량을 계산한다.
    pub fn for_nodes(nodes: &[PlannedNode]) -> Result<Self, String> {
        let mut horizon = Rational::ZERO;
        for node in nodes {
            if let KernelSpec::Resample { specification, .. } = &node.kernel {
                specification.offset()?;
                horizon = horizon.checked_add(Rational::ONE.divide(specification.rate)?)?;
            }
        }
        let mut bursts: Vec<usize> = Vec::with_capacity(nodes.len());
        let mut result = Self {
            capacities: Vec::with_capacity(nodes.len()),
            bytes: nodes
                .len()
                .checked_mul(128)
                .and_then(|n| n.checked_add(1024))
                .ok_or("큐 제어 예약량 범위를 초과했다.")?,
        };
        for (id, node) in nodes.iter().enumerate() {
            if node.inputs.iter().any(|&i| i >= id) {
                return Err("큐 입력은 선행 노드를 참조한다.".into());
            }
            let input_burst = node.inputs.iter().map(|&i| bursts[i]).max().unwrap_or(1);
            let burst = if let KernelSpec::Resample { specification, .. } = &node.kernel {
                input_burst
                    .checked_mul(specification.burst())
                    .ok_or("재표본화 burst 범위를 초과했다.")?
            } else {
                input_burst
            };
            bursts.push(burst);
            let mut capacities = Vec::new();
            for &source in &node.inputs {
                let output = nodes[source]
                    .output
                    .as_ref()
                    .ok_or("큐 입력 타입을 지정한다.")?;
                let rate = match output.occurrence {
                    Occurrence::Uniform { rate, .. } | Occurrence::PendingUniform { rate, .. } => {
                        rate
                    }
                    _ => return Err("고정 큐 일정은 Uniform 발생률을 요구한다.".into()),
                };
                let capacity = if horizon.is_zero() {
                    1
                } else {
                    horizon
                        .multiply(rate)?
                        .ceil_natural()?
                        .checked_add(bursts[source])
                        .and_then(|v| v.checked_add(1))
                        .ok_or("큐 용량 범위를 초과했다.")?
                };
                if capacity > 65536 {
                    return Err(
                        "간선 큐 상한은 65536 레코드다. 재표본화 비율과 경로를 조정한다.".into(),
                    );
                }
                let bytes = nodes[source]
                    .kernel
                    .buffer_bytes()?
                    .checked_add(128)
                    .and_then(|v| v.checked_mul(capacity))
                    .and_then(|v| v.checked_add(128))
                    .ok_or("큐 예약량 범위를 초과했다.")?;
                result.bytes = result
                    .bytes
                    .checked_add(bytes)
                    .ok_or("큐 예약량 합의 범위를 초과했다.")?;
                capacities.push(capacity);
            }
            result.capacities.push(capacities);
        }
        Ok(result)
    }
}
