# 상태 연산자와 실행 checkpoint

`StatefulOperator`는 Rust 객체의 준비·계산·초기화·저장·복원 경계를 제공한다. 현재 `stateful_f64/0.1` profile은 단일 Vec<f64,C> 입력마다 같은 크기의 벡터를 한 개 게시하고, 채널·단위·clock·발생 격자·품질 mask를 보존한다. 사용자 누적 상태와 표준 재표본화·IIR·FIR·창·간선 큐 상태를 하나의 checkpoint로 저장한다.

## 실행과 이어 처리

[stateful.bci](../examples/stateful.bci)는 CSV → 사용자 누적 평균 → 단위 변환 → FIR → 창 → 공분산·SPD를 연결한다. 원시·평균·공분산 포트를 함께 기록한다.

```sh
cargo build -p bci-cli --example bci-lab --offline --locked
./target/debug/examples/bci-lab run examples/stateful.bci --samples 8 --output recordings/stateful-head --checkpoint-out recordings/stateful-at-8.json
./target/debug/examples/bci-lab run examples/stateful.bci --resume recordings/stateful-at-8.json --output recordings/stateful-tail
```

첫 실행은 입력 8개를 처리해 레코드 18개와 checkpoint를 저장한다. 복원 실행은 위치 8부터 EOF까지 입력 14개와 레코드 33개를 처리한다. 두 기록의 data 행을 순서대로 합치면 연속 실행의 51개 행과 일치한다. 입력에는 2표본 누락 구간과 무효 채널이 포함된다.

| 옵션 | 의미 |
|---|---|
| `--checkpoint-out 파일` | 완료한 입력 경계의 상태를 캡처하고 기록 commit 이후 새 파일에 저장 |
| `--resume 파일` | 계획·실행 파일·원천 prefix를 대조한 뒤 저장 위치부터 실행 |
| `--samples N` | 이번 호출에서 추가로 처리할 입력 묶음 상한 |
| `--output 디렉터리` | 이번 구간의 새 기록 디렉터리 |

복원과 저장 옵션을 함께 지정하면 새 경계로 이어 저장한다. checkpoint 경로는 CLI 작업 디렉터리 기준이다. 저장은 새 파일 경로를 요구하며, 같은 이름의 파일이 있으면 E0902를 반환한다. 정상 EOF·표본 상한·Rust API의 stop 경계에서 상태를 캡처한다. 입력·커널·기록 오류로 Fault가 발생한 실행의 복원 출발점은 이전에 commit된 checkpoint다.

## 상태 연산자 작성

[lab_state.rs](../crates/cli/lab_state.rs)의 `RunningMean`은 채널별 sum·count를 소유한다. valid=1인 입력을 누적하고 평균을 출력한다. valid=0인 채널은 누적 갱신을 건너뛰며, wrapper가 입력의 무효 payload·mask를 출력에 보존한다.

```rust
#[operator(
    name = "lab.signal.running_mean", version = "0.1.0",
    kind = "stateful", contract = "running_mean.operator.toml"
)]
impl StatefulOperator for RunningMean {
    // prepare, state_bytes, step, reset, checkpoint, restore를 구현한다.
}
```

매크로의 적용 대상은 구체 타입의 `impl StatefulOperator`다. 전체 impl 토큰을 함수 digest로 기록하고 `RunningMean::bci_registration()`을 생성한다. 번들은 `Extensions::with_stateful(vec![RunningMean::bci_registration()])`로 연결한다. pointwise와 stateful 선언은 schema의 `OperatorSpec`, 계획의 `KernelSpec::Extension`을 공유한다.

| SDK 메서드 | 계약 |
|---|---|
| `prepare(channels)` | 고정 채널 수로 객체·소유 버퍼 예약 |
| `state_bytes()` | 객체와 소유 버퍼 capacity를 합한 byte 수 반환 |
| `step(input, output, scratch)` | 읽기 전용 값·valid를 받아 예약한 출력 slice를 채움 |
| `reset(cause)` | Gap 또는 명시적 초기화 원인에 따라 누적 상태 초기화 |
| `checkpoint(writer)` | 고정 codec 판본의 상태를 제한된 writer에 저장 |
| `restore(reader)` | 새 객체에 상태를 복원하고 값·count·크기 불변식 검사 |

wrapper는 generation 변경·원천 index 불연속·명시적 Gap에서 `reset(Gap)`을 호출한다. 상태가 읽은 원천 범위는 초기화 이후 입력 구간을 보수적으로 포함한다. 시간·sequence·source_last·generation·flags는 현재 입력에서 전파한다. origin은 초기화 이후 입력들의 유래를 합치고, 서로 다른 유래가 기여하면 Mixed를 출력한다. scratch는 각 호출 전에 0으로 초기화하고, 지속 값은 소유 상태에 둔다.

상태와 출력은 prepare·step·restore 경계에서 자원·수치 조건을 확인한다. step 오류나 panic 뒤의 wrapper는 Fault 상태로 남고, 새 객체의 준비·복원으로 실행을 시작한다. 입력 metadata와 valid는 SDK가 소유하며 사용자 step은 값 계산과 자신의 누적 상태를 담당한다.

## 자원과 codec

[running_mean.operator.toml](../crates/cli/running_mean.operator.toml)은 다음 자원을 선언한다.

```toml
[resources]
fixed_bytes = 64
channel_bytes = 16
scratch_bytes = 0
checkpoint_bytes = 0
checkpoint_channel_bytes = 16

[state]
codec = "lab.running_mean.le/0.1"
```

채널 수 C의 소유 상태 상한은 `fixed_bytes + C*channel_bytes`, scratch는 `C*scratch_bytes`, 사용자 checkpoint payload는 `checkpoint_bytes + C*checkpoint_channel_bytes`다. 상태·scratch 합의 노드별 상한은 64 MiB다. wrapper 제어 구조와 출력 슬롯은 기존 실행 메모리 예산에 포함한다.

StateWriter는 예약 상한 안에 little-endian u64·f64 bit·선택 정수·byte열을 쓴다. StateReader는 잘린 입력, 잘못된 tag, 비유한 f64, 남은 byte를 진단한다. 타입별 restore는 여기에 count·cursor·phase 등 자신의 불변식을 더한다. 사용자 codec 판본·계약·impl 변경은 계획 digest를 바꾸며 새 checkpoint 계보를 시작한다.

checkpoint 전체 커널 예약 상한은 16 MiB, JSON 파일 상한은 64 MiB다. 직렬화·역직렬화 자료는 준비·종료 단계의 호스트 메모리로 관리한다. save 옵션의 전체 상한을 입력 열기 전에 확인한다.

## 저장 상태와 복원 절차

checkpoint는 `bci-checkpoint/0.2` 형식으로 다음을 보관한다.

- 표본 상한을 제외한 계획 digest와 동기 또는 시각 병합 정책으로 처리한 입력 묶음 수.
- 실행 파일 전체 SHA-256, checkpoint payload SHA-256.
- 묶음 순서·원천 ID 순서로 처리한 모든 원천 값·품질·정확한 시각·Gap·generation·origin의 prefix hash.
- 사용자 상태와 wrapper의 이전 generation·sequence·원천 범위·누적 origin.
- 재표본화 입력 이력·다음 출력 index·보류 Gap과 합류 대기 큐.
- IIR의 채널별 SOS 지연·warmup 카운터·누적 origin과 원천 범위.
- FIR ring·valid·원천 범위·origin·cursor·filled, 창 ring·origin·phase·연속 valid 수.
- 각 출력 포트의 시간 격자·generation·마지막 시각·index.

PSD·대역 전력·로그 특징·채널 선택·재정렬·재기준화·가중 합성·선형 배율·pointwise·공분산·SPD 정규화는 호출 간 계산 상태가 비어 있어 빈 payload를 사용한다. 공분산·Cholesky scratch는 복원 시 초기화한다.

복원은 다음 순서로 수행한다.

1. 파일 판본·실행 파일·payload digest와 계획의 노드·타입·계수·codec·자원·연결을 대조한다.
2. 모든 원천을 처음부터 다시 열고 저장 위치까지 원래 입력 병합 정책으로 읽어 prefix hash와 각 원천 격자를 확인한다. 이 탐색에서 수치 커널과 기록 sink의 호출 수는 0이다.
3. 새 커널 객체에 payload를 복원하고 각 포트의 격자를 연결한다.
4. 새 기록 디렉터리를 준비하고 저장한 간선 큐와 다음 입력 묶음을 이어 처리한다.

현재 원천 위치 복구의 읽기 비용은 처리한 원천 레코드 수에 비례한다. [다중 입력 계약](multi-input.md)은 공통 위치와 prefix 대조 규칙을 정의한다. 파일의 처리 완료 prefix는 동일한 의미 값·시간·품질을 요구하고, 후속 입력은 기존 입력 계약으로 검사한다. 누적 입력 위치 상한은 10억 묶음이다. 변경을 허용하는 실행 인수는 추가 표본 상한과 CLI 출력 디렉터리다. 소스 span을 포함한 계획 변경과 다른 실행 바이너리는 E0909로 진단한다.

저장 파일은 임시 파일 쓰기·sync 후 새 경로로 원자적으로 게시한다. 기록 commit이 성공한 상태를 대상으로 저장한다. 복원 구간의 manifest는 `report.input_offset`에 시작 위치를, `completed_samples`와 노드 카운터에 이번 구간 관측값을 저장한다. 레코드의 sequence·time·generation은 원래 값을 유지한다.

## 검사 근거와 확장 위치

기존 CLI 통합 검사 파일에 상태·복원 검사 함수 하나를 추가했다. 별도 평균 계산과 결과를 비교하고, 입력 2·12·17개 경계에서 저장·복원한 전체 파이프라인의 data 51개를 연속 실행과 비교한다. 이 경계는 FIR warmup, Gap 직전, Gap 이후 창 진행 상태를 포함한다. 계획·입력 prefix·격자·잘린 payload·실행 파일·digest 불일치와 기록 실패의 checkpoint 폐기도 검사한다.

`extensions check`는 누적 평균의 참조 4쌍을 순서대로 적용하며 각 입력 뒤 객체를 저장·복원하고, generation 전환 뒤 첫 결과도 확인한다. 기존 pointwise 참조 8쌍과 합쳐 bci-lab의 참조 입력은 12개다.

| 파일 | 책임 |
|---|---|
| schema/extension.rs | 공통 OperatorSpec·StateContract·채널별 상한 |
| extension/stateful.rs·codec.rs | 상태 trait·wrapper·제한된 codec |
| extension-macros/stateful.rs | impl 등록·TOML·참조 연결 |
| operators/state.rs·linear.rs·window.rs | 표준 상태 codec·불변식 |
| ir/checkpoint.rs | 실행 checkpoint 값 타입 |
| runtime/checkpoint.rs·sequential.rs | 계획·원천 대조·상태 복원·경계 캡처 |
| artifact/checkpoint.rs | 파일 digest·형식·원자적 게시 |

다중 포트·발생률 변경·RejectRecord·epoch migration·매개변수 codec은 [전체 상태 확장 설계](design/13-rust-extensions.md)의 후속 계약이다.

SOS IIR의 상태 codec과 채널별 초기화는 [IIR 안내](iir.md)에 정의한다.
