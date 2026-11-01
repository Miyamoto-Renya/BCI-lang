# 재표본화와 다중 표본률 실행

`signal.resample`은 확정 Uniform 격자의 `Vec<f64,C>`를 인과 polyphase FIR로 변환한다. 실행기는 포트별 고정 용량 큐로 여러 출력과 합류 대기를 처리한다. 같은 clock의 서로 다른 표본률·위상을 가진 원천은 정확한 시각 순서로 병합한다.

## 실행 예제

[resample.bci](../examples/resample.bci)는 250 Hz → 375 Hz → 250 Hz 경로와 원신호를 합성한다. [multirate.bci](../examples/multirate.bci)는 250 Hz와 375 Hz 원천을 읽고 첫 원천을 375 Hz로 변환해 합성한다.

```sh
cargo run --offline --locked -- run examples/resample.bci --samples 9 --output recordings/resample-full
cargo run --offline --locked -- run examples/resample.bci --samples 2 --output recordings/resample-head --checkpoint-out recordings/resample-at-2.json
cargo run --offline --locked -- run examples/resample.bci --samples 7 --resume recordings/resample-at-2.json --output recordings/resample-tail
cargo run --offline --locked -- run examples/multirate.bci --samples 17 --output recordings/multirate
```

첫 예제는 입력 9개에서 원시 9개·375 Hz 출력 13개·250 Hz 복귀 출력 9개·합성 출력 9개, 총 40개를 기록한다. 입력 2개 경계에는 합류 큐에 원시 레코드 1개가 남는다. 이 큐를 복원한 뒤 7개 입력을 처리하면 두 기록의 연결이 연속 실행과 일치한다.

두 원천 예제의 17개 입력 묶음은 시각 0..0.032초를 방문한다. 250 Hz 원천에서 9개, 375 Hz 원천에서 13개를 읽고 총 48개 레코드를 기록한다. 서로 같은 시각의 원천은 한 묶음에 포함한다.

## 수치·격자 계약

```bci
let fast = raw |> signal.resample(
    up: 3, down: 2,
    filter: [0.25, 0.5, 0.75, 0.75, 0.5, 0.25],
    phase: 0[s]
);
```

입력률을 `f`, 입력 위상을 `a`, 출력 위상을 `phi`, 배수를 `p/q`로 두면 다음을 계산한다.

```text
f_out = f*p/q
d = (phi-a)*f*p
t_out[m] = phi + m/f_out
y[m] = Σ_n x[n] * h[m*q+d-n*p]
```

- p와 q는 서로소인 1..1024 정수다. 필터 설계는 기약 비율의 고속 격자 `f*p`를 기준으로 계수를 제공한다.
- d는 0 이상 q 미만의 정수다. 출력 phase는 입력 위상부터 출력 한 주기 안의 고속 격자점이다. 단위 배율도 정확한 유리수로 계산한다.
- filter는 계수 artifact 또는 무차원 수치 배열이며, 유한 계수 1..65536개를 가진다. h[0]부터 인과 support를 정의한다.
- 계수를 그대로 곱한다. 보간 배율에 따른 이득은 필터 계수에 포함한다. 예제의 위상별 계수 합은 1이다.
- 필터 설계자는 목표 passband·stopband·anti-alias 감쇠를 선택한다. 현재 실행 profile의 검증 대상은 유한 계수·인과 support·정확한 격자·출력 수치다. 주파수 응답 증거를 포함하는 필터 artifact는 상세 설계의 확장 항목이다.

입력 시각 t를 수락하면 시각이 t 이하인 출력을 차례로 게시한다. 분수 시각의 출력은 그 시각 이상인 첫 입력에서 계산하며, 실제 기여 입력은 수식의 인과 support로 선택한다. 마지막 입력 이후의 출력과 FIR tail은 Tail.Discard 경계로 처리한다. 준비한 이력은 `ceil(len(h)/p)+1`개 Frame이며, 입력 한 개당 출력 상한은 `ceil(p/q)`다.

## 품질·Gap·계보

0 이외의 계수가 참조하는 입력의 valid를 채널별 AND로 합친다. 초기 이력은 0 값·무효로 취급하고 Warmup을 표시한다. 무효 실측 입력은 InvalidInput을 전파한다. 부분 support에서 계산한 값과 valid를 함께 기록한다. 모든 출력 값은 런타임의 유한 수치 검사를 통과해야 한다.

명시적 Gap 또는 generation 전환은 필터 이력을 초기화하고 새 입력 시각 이상의 첫 출력 격자점부터 처리한다. Gap은 그 경계의 첫 출력까지 보관한다. source_range와 origin은 실제 기여한 이력을 합쳐 계산한다. 기여 입력이 없는 0 출력의 메타데이터는 현재 입력을 기준으로 한다.

sequence는 재표본화 출력의 격자 index다. source_range와 gap은 원천 index 범위를 유지한다. 기록 0.5는 포트 sequence와 원천 범위를 각각 검사한다. 합류한 여러 원천의 source_range는 manifest의 source_ports에 속한 index들을 포함하는 보수적 구간이다.

## 입력 병합과 큐 일정

같은 clock·같은 격자의 여러 원천은 기존 SynchronousExact 계약으로 시각·sequence·generation·Gap·EOF를 대조한다. 서로 다른 확정 Uniform 격자의 원천은 UniformTimeMerge 계약을 사용한다. 각 reader에 다음 레코드 한 개를 보관하고, 가장 이른 시각의 원천들만 수락한다. 각 원천의 연속 격자는 개별 검사한다. 모든 reader가 EOF에 도달하면 입력을 마친다.

`--samples`와 completed_samples는 수락한 입력 묶음 수다. UniformTimeMerge에서 각 원천의 실제 읽기 수는 노드별 records에 기록한다. 동기 입력의 Gap은 공통 누락을 한 번 집계하고, 시간 병합의 Gap은 각 원천의 누락을 집계한다. 서로 다른 clock의 변환은 time.align 실행 계약의 후속 구현 대상이다.

각 입력 간선은 고정 ring을 소유한다. 노드는 모든 입력 큐가 준비되면 발화하고, `Kernel::drain`으로 추가 출력을 게시한다. 단일 입력·기존 Rust 확장은 기본 drain 구현을 사용한다. `signal.mix`는 소비할 레코드의 시각·sequence·generation·Gap 일치를 검사한다. 기록 sink는 각 포트의 큐를 독립적으로 소비한다.

큐 용량은 모든 재표본화 입력 주기의 합 H와 경로의 출력 burst B로 계산한다. 재표본화가 있는 그래프의 입력 간선 용량은 `ceil(H*port_rate)+B+1`이다. B는 선행 burst와 커널의 출력 상한을 곱해 보수적으로 전파한다. 간선당 상한은 65536 레코드다. compiler와 verify가 같은 계약에서 용량·전체 예약량을 다시 계산하며, 큐 슬롯·reader 선행 슬롯도 memory 예산에 포함한다. 용량 초과는 Queue.Fault로 E0901을 발생시킨다.

Gap은 해당 원천에서 도달하는 경로의 대기 큐를 비운다. 합류 입력들의 generation·시각 경계는 해당 노드의 계약을 따라야 한다. EOF에 합류 대기 레코드가 남으면 E0901로 입력 구간·출력 범위 불일치를 보고한다. 표본 상한·stop 종료에서는 pending_records로 대기 수를 보고하고 checkpoint에 저장한다.

## 저장과 검증

checkpoint 0.2는 필터 이력·다음 출력 index·보류 Gap·간선 큐·격자를 저장한다. 복원은 저장된 입력 묶음 수만큼 같은 병합 순서를 재실행해 prefix와 원천별 격자를 확인한다. reader의 선행 슬롯도 이 과정에서 다시 구성한다. 필터·큐 payload는 판본·길이·시각·수치·격자 범위를 검사한다.

manifest는 커널 계수·비율·phase, input_policy, queue_capacities, queue_reserved_bytes와 pending_records를 기록한다. 파일 판본은 bci-run/0.6·bci-recording/0.6이고, 기존 0.5·0.4·0.3·0.2·합성 0.1 재생을 지원한다.

[CLI 통합 검사](../crates/cli/pipeline.rs)의 한 시나리오에서 독립 zero-insertion 합성곱 참조, 정확한 시각, 비영 phase, 품질·Gap, 합류 대기, 파일 EOF, 기록 재생, checkpoint 연결과 잘못된 비율·계획을 확인한다.

schema/resample.rs가 수치 계약, ir/queues.rs가 큐 예약, operators/resample.rs가 필터, runtime/ingress.rs와 scheduler.rs가 입력 병합·큐 실행을 소유한다. Rust 파일은 기존 11개 패키지의 같은 깊이에 배치한다.
