# 입력 실행과 결과 기록

`bci run`은 `.bci`를 검사한 뒤 실행 계획을 만들고, 독립 검증·메모리 예약·수치 처리·기록 commit을 수행한다. 현재 실행 번들은 원천 1..64개, f64 포트, 순차 DAG와 동기 기록을 사용한다. 각 원천은 합성·측정 CSV·기록 재생 중 하나를 선택하며, 파일 계약은 [입력 안내](inputs.md)에 정의한다.

```sh
cargo run --offline --locked -- run examples/typed.bci --samples 1000 --output recordings/demo
cargo run --offline --locked -- run examples/file.bci --output recordings/file
```

## 명령과 완료 조건

| 옵션 | 동작 |
|---|---|
| `--samples N` | 각 원천에서 하나씩 읽는 동기 묶음의 상한. 범위 1..1000000000, EOF가 먼저 오면 종료 |
| `--entry 이름` | 여러 run 중 선언 이름 또는 모듈 전체 이름을 선택 |
| `--output 경로` | 모든 sink의 출력 기준 디렉터리. 상대 경로는 CLI 작업 디렉터리 기준 |

기본 출력 기준은 `recording.write(to: ...)`다. 이 상대 경로는 입력 `.bci`의 디렉터리에서 해석한다. 실행마다 새로운 `run-시각-pid-순번/` 디렉터리를 생성한다. 같은 기준 경로로 반복 실행하면 각 실행의 파일을 독립 보관한다.

상한을 생략하면 파일 원천들은 공통 EOF에서 종료하고, 합성 원천을 포함하면 기본 1000묶음에서 종료한다. `--samples`는 실제 읽은 동기 묶음을 센다. 시간 격자에서 누락된 표본 수는 Gap 관측값으로 별도 집계한다.

합성 신호는 논리 시간 `phase+n/rate`의 표본을 처리 가능한 속도로 생성한다. 각 채널 c의 값은 해당 채널에 선언한 단위로 다음 공식을 사용한다. t는 `n/rate`초이며 합성 원천의 origin은 Synthetic이다. 파생 결과는 기여한 입력들의 origin을 합치며 합성·측정 입력이 함께 기여하면 Mixed다.

```text
x_c[n] = sin(2π(10+3c)t) + 0.25 cos(2π·3t+c)
```

CLI는 완료 입력 묶음 수, 기록 수, 예약 bytes와 처리 루프에서 측정한 묶음/초를 출력한다. 시각은 signed 128-bit 유리수로 계산하고, 실제 경과 시간은 monotonic clock으로 측정한다. Live는 입력 가용성에 따른 인과 처리 효과를 나타낸다. 벽시계 주기·deadline·장치 서비스율은 [전체 실행 설계](design/09-runtime.md)의 환경 증거로 검증하는 항목이다.

## 예제의 실제 결과

[typed.bci](../examples/typed.bci)의 기본 1000표본 실행은 원시 표본 1000개와 SPD 공분산 30개를 기록한다. 실행 그래프는 7개 노드이며 현재 자료형 크기로 계산한 예약량을 CLI와 manifest에 기록한다.

길이 3의 FIR은 index 0·1을 warmup invalid로 출력한다. 첫 연속 250개 valid 표본은 index 2..251이다. 첫 창 anchor는 251/250초, 후속 창 간격은 25/250초다. 내부 표본률 250 Hz와 출력률 10 Hz를 구분한다. 마지막 완성 창의 anchor는 index 976이고 종료 시 남은 부분 창은 Tail.Discard로 처리한다.

## 지원 커널

| 연산자 | 실행 규칙 |
|---|---|
| `source.synthetic` | 호출당 합성 원천 1개, 선언한 채널·clock·rate·phase |
| `source.csv` | 헤더·시간 격자·Gap·무효 수치를 검사하고 Measurement origin으로 게시 |
| `source.replay` | 기록 포트의 Vec·Epoch·Spectrum·Cov·SPD 값·시각·품질·generation·origin 보존 |
| `source.assert_schema` | 정적 스키마 검사를 완료한 Vec·Epoch를 같은 타입으로 전달 |
| `signal.convert_unit`, `signal.scale` | Vec·Epoch의 채널별 배율을 값 배열에 적용 |
| `signal.select`, `signal.reorder` | Vec·Epoch의 채널 값·mask·스키마를 지정 ID 순서로 복사 |
| `signal.rereference` | 동일 전압·단위·기준의 Vec·Epoch에 `A=I−1wᵀ` 적용, 새 reference ID와 기여 채널의 valid 계산 |
| `signal.mix` | 같은 타입·tick의 Vec 포트 2..64개 가중 합, mask AND·flags OR·origin 병합 |
| `model.predict` | 고정 모델의 Fit 정규화·softmax, 무효 특징의 전체 mask 처리 |
| `decision.threshold` | 최대 확률·minimum에 따른 Select·Hold, 클래스 순서 동점 처리 |
| `signal.resample` | 인과 polyphase FIR, 정확한 p/q 격자·여러 출력·품질·Gap·checkpoint |
| `signal.iir` | 안정성을 검사한 SOS DF-II-T, 채널별 무효 입력 초기화·warmup·전체 Gap 초기화 |
| `signal.fir` | `Σh[l]x[n-l]`, 0 초기값, Full warmup, 품질·원천 범위 전파 |
| `signal.window` | 채널 우선 ring, 겹침·간격 창, FirstValid 또는 정수 start, RequireAll·End·Discard |
| `signal.psd` | f64 Epoch의 Rectangular·주기적 Hann 창, 단측 PSD·DC/Nyquist 정규화 |
| `signal.bandpower`, `signal.log_power` | 정확한 주파수 대역의 밀도 적분·같은 단위의 자연로그 특징 |
| `signal.covariance` | 평균 제거 후 `XH Xᵀ/(N-1)`, 대칭 성분을 동일 값으로 게시 |
| `signal.regularize` | `(1-lambda)S + lambda·max(trace(S)/C,floor)I`, Cholesky 검사 |
| `recording.write` | 튜플의 각 포트가 게시한 레코드를 port ID와 함께 기록 |
| 사용자 `stateful_f64/0.1` | 같은 격자의 누적 상태·Gap 초기화·제한된 상태 codec |
| 사용자 `pointwise_f64/0.1` | 등록한 Rust 함수를 valid 원소마다 호출, 무효 payload·시간·품질·Gap 보존 |

FIR·창은 원천 index와 generation을 기준으로 상태를 유지한다. FIR의 gap은 상태와 warmup을 초기화하고, FirstValid 창의 gap은 새 연속 구간의 위상을 찾는다. 무효 채널은 valid mask로 전달한다. 공분산은 입력 창의 전 채널 유효성을 요구하며, 무효 창의 결과는 0 값·무효 mask로 게시한다. SPD guard는 유효 공분산마다 실행한다. 비유한 결과와 실패한 Cholesky는 NumericFault를 발생시킨다.

실행 번들은 위 커널을 가진 계획을 수용한다. 사용자 함수의 등록·타입·수치 참조·panic 경계는 [Rust 확장 안내](extensions.md)를 따른다. 커널 연결이 필요한 정적 연산자에는 E0803과 해당 호출 위치를 표시한다. 하드웨어 획득·Record 전체 구간 필터·다중 포트 상태 확장 커널은 [연산자](design/05-operators.md)·[장치](design/06-hardware.md)·[확장](design/13-rust-extensions.md) 설계의 후속 구현 대상이다.

## 계획·소유권·메모리

컴파일러는 구체 커널, 입력 ID, 출력 타입, ID 의존 순서의 일정, 출력 슬롯·상태 예약량과 의무를 ExecutionPlan에 저장한다. verify는 원천 수, 포트 layout·단위·채널·발생 규칙, 선행 입력, 모든 출력의 소비, 필수 의무 연결, 예약량 합과 budget을 다시 계산한다.

실행기는 같은 격자의 원천을 동기 묶음으로 검사하고, 다른 확정 Uniform 격자의 원천을 정확한 시각 순서로 병합한다. 각 입력 간선의 고정 큐에 레코드를 전달하고 입력이 준비된 노드를 의존 순서로 실행한다. 커널은 출력 슬롯을 재사용하며 drain 인터페이스로 추가 출력을 게시한다. 기록 sink는 포트별 큐를 소비한다. [재표본화·다중 표본률 계약](resampling.md)은 큐 예약·합류·EOF·복원을 정의한다.

`runtime.budget(memory: ..., queue_policy: Queue.Fault)`의 적용 대상은 출력 값·품질 슬롯, 입력 선행 슬롯·간선 큐, FIR·창 ring, 수치 scratch, 커널·격자 제어 구조, reader 버퍼·행 파싱 공간, sink별 64 KiB staging이다. compiler의 AST·상수·계획 기술자와 manifest JSON은 호스트 메모리로 관리한다. 노드별 예약량과 총합을 manifest에 기록하고, 예산 초과는 실행 준비 전에 E0801로 진단한다.

기록은 동기 backpressure로 원천 진행 속도를 제어한다. prepare는 새 파일 생성·header 쓰기·sync를 확인하고, 실행과 종료는 write·flush·sync 실패를 감시한다. 디스크 용량 고갈과 I/O 실패는 해당 세션의 저장 fault로 전달한다.

## 의무 판정과 종료

| 의무 | 실행 증거 |
|---|---|
| Input | reader 계약을 준비하고 각 행을 검사. EOF까지 수락하면 Pass, 명시적 상한·stop 종료는 Guarded |
| WindowPhase | 첫 완성 창의 정확한 anchor·원천 index를 Pass로 기록. 짧은 입력으로 완성 창 0개이면 Unobserved |
| Resampling | 정확한 격자·인과 support·품질·수치 guard와 실제 출력 수 |
| NumericGuard | 커널에 Cholesky guard를 연결하고 실제 통과한 유효 레코드 수를 기록 |
| Storage | 준비 성공과 동기 기록·flush·sync의 완료를 Committed로 기록. 저장 오류는 Fail |

정상 상태는 `Created → Preparing → Ready → Running → Draining → Closed`다. 실패 시 Faulted를 기록하고 Draining에서 보유 데이터를 flush·sync·close한다. 종료 manifest는 최초 실패의 코드·노드·원본 위치와 표본 경계까지 완료한 수를 보존한다. 개별 노드의 출력 수와 sink의 기록 수를 따로 제공한다.

Rust API `bci_runtime::run(plan, kernels, sources, recorder, stop)`는 AtomicBool stop 요청을 원천 레코드 경계에서 받아 동일 종료 경로를 수행한다. CLI의 정상 종료 경계는 입력 EOF와 `--samples`다. 기록 파일의 commit은 flush·sync 후 manifest 교체 순서로 수행한다. prepare manifest가 남아 있는 실행은 Preparing 상태로 식별한다.

`--checkpoint-out`과 `--resume`은 완료한 레코드 경계의 전체 커널·격자를 저장하고 복원한다. 표본 상한은 이번 호출의 추가 입력 수이며, 복원 기록은 새 디렉터리에 저장한다. `report.input_offset`은 복원 시작 위치다. 옵션·파일 형식·원천 prefix 검사·상태 SDK는 [checkpoint 안내](checkpoints.md)를 따른다.

종료 코드는 0=완료, 2=소스·계획 구성 오류, 4=수치·실행 fault, 5=소스·기록 I/O 오류다.

## 기록 형식과 확장 경계

각 실행 디렉터리는 `manifest.json`과 `sink-ID.jsonl`을 가진다. manifest의 판본은 `bci-run/0.6`, 데이터 판본은 `bci-recording/0.6`다. manifest는 원천 파일·정책, 선택한 커널·계수·배율, 각 포트의 값 kind·shape·채널 의미·단위 차원과 정확한 배율·clock, 일정·예약량·의무·실제 관측을 보관한다. 재생기는 `bci-run/0.5`·`bci-recording/0.5`, `bci-run/0.4`·`bci-recording/0.4`, `bci-run/0.3`·`bci-recording/0.3`, `bci-run/0.2`·`bci-recording/0.2`와 기존 `bci-synthetic-run/0.1`·`bci-synthetic-recording/0.1`도 읽는다.

정적 번들 표기는 `bci-static-f64/0.10`, 표준 커널 표기는 `bci-standard-f64/0.8`다. `executable_sha256`는 실행 파일 전체를 식별한다. 사용자 노드는 함수·계약·reference digest와 Rust 위치를 추가로 기록한다.

JSON Lines의 첫 줄은 header다. 각 data 줄은 `port, sequence, time_seconds, source_range, generation, origin, gap, flags, valid, values`를 가진다. sequence는 포트가 사용하는 anchor index이며 재표본화 포트는 출력 격자 index를 사용한다. time_seconds는 정확한 유리수 문자열이다. source_range는 기여한 첫·마지막 원천 index의 닫힌 범위다. gap은 직전 누락 index의 닫힌 범위 또는 null이다. values는 채널 우선 평탄 배열이며 공분산은 행 우선 C×C다. flags의 1은 Warmup, 2는 InvalidInput, 4는 Gap이다. epoch는 실행 header의 0으로 고정하고, Gap 상태 초기화는 generation으로 구분한다.

| 파일 | 책임 |
|---|---|
| schema/execution.rs·kernel.rs | Frame·품질·관측 보고서, 구체 커널 구성·예약 계약 |
| schema/input.rs | InputSource·SourceFactory, 원천 기술자·정확한 격자 검사 |
| ir/plan.rs·compiler/plan.rs | 계획 자료와 TypedGraph에서 계획 생성 |
| verify/sequential.rs | 순차 일정·포트·자원·의무 검사 |
| extension/lib.rs | Kernel·KernelFactory 실행 경계 |
| schema/extension.rs·extension/pointwise.rs | 사용자 계약·registry·유효 원소 wrapper·reference 검사 |
| extension-macros/lib.rs | Rust 서명·TOML 계약 검사와 registration 생성 |
| operators/iir.rs·spatial.rs·spectral.rs·mix.rs·linear.rs·window.rs·matrix.rs | 표준 수치 커널과 상태 |
| operators/source.rs | 합성 InputSource |
| runtime/ingress.rs·sequential.rs | 동기 묶음·슬롯·진행·guard·종료·fault |
| artifact/recording.rs | JSON Lines·파일 commit·manifest |
| artifact/input.rs·csv.rs·replay.rs·stream_codec.rs | 유한 행 reader·CSV·선택 포트 재생·타입 codec |
| cli/lib.rs·execute.rs·lab.rs | 공통 CLI·실제 커널·기록기·사용자 번들 조립 |

파일은 각 패키지의 루트에 배치한다. runtime은 SourceFactory·KernelFactory·Recorder 인터페이스를 호출하고, CLI가 합성·파일 reader, 표준 커널과 디스크 기록기를 주입한다. 장치 adapter도 같은 InputSource의 read·EOF·오류 경계에 연결한다.

검증은 합성·CSV·확장 예제를 재사용하는 [통합 테스트 함수 9개](../crates/cli/pipeline.rs)에 모았다. 표준 수치 참조, 채널 변환·재기준화의 스키마·품질, 입력·기록·재생의 값·시각·품질 일치, Gap·시간·잘린 기록, 사용자 계약·reference·panic·비유한 출력과 전체 커널의 checkpoint·복원을 검사한다. 전체 검사는 `python tools/check.py`로 실행한다.

채널 선택·재정렬·재기준화의 Vec·Epoch 규칙, 기준 digest와 실행 예제는 [채널 변환 안내](channels.md)를 따른다.

주파수 특징의 Spectrum 타입, bin 경계·단위·정규화와 예제는 [PSD 안내](spectral.md)를 따른다.

고정 SOS 파일·안정성·초기화와 지연 상태 복원은 [IIR 안내](iir.md)를 따른다.

모델·Probability·Decision의 기록·품질·실행 고정 계약은 [학습·예측 안내](learning.md)에 정의한다.
