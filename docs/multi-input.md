# 동기 다중 입력과 가중 합성

현재 실행기는 같은 clock의 원천 1..64개를 읽는다. 같은 확정 Uniform rate·phase는 이 문서의 동기 묶음 계약을 사용한다. 서로 다른 격자는 [다중 표본률 시간 병합](resampling.md)의 계약을 사용한다. 합성·CSV·재생 reader를 조합하고, 각 원천에서 읽은 레코드 하나씩을 동기 입력 묶음으로 처리한다. `signal.mix`는 같은 타입의 벡터들을 가중 합성한다.

## 언어 계약

```bci
let mixed = signal.mix(input: (left, right), weights: [0.25, 0.75]);
```

입력 튜플은 2..64개 `Vec<f64,C>` 포트를 가진다. 채널 순서·측정량·단위 배율·clock·발생 규칙·품질 타입이 모두 일치해야 한다. 필요한 단위 변환은 합성 전에 `signal.convert_unit`으로 표현한다. 같은 포트를 여러 인수로 참조하거나 한 원천의 여러 처리 경로를 합성할 수 있다.

weights는 입력마다 하나씩 지정하는 유한 무차원 수치다. 음수와 0도 허용하며 출력 채널 c는 인수 순서로 `Σ weights[i] * input[i][c]`를 계산한다. 지정한 계수를 그대로 적용하므로 평균은 합이 1인 계수로 표현한다. 계산 결과가 비유한 값이면 런타임 수치 guard가 E0903을 반환한다.

품질과 계보는 모든 입력 인수를 기준으로 계산한다. 0 계수도 해당 입력의 품질·계보 의존성을 유지한다.

| 필드 | 합성 결과 |
|---|---|
| valid[c] | 모든 입력의 해당 채널 mask를 AND |
| values[c] | valid=0이면 0, valid=1이면 가중 합 |
| flags | 모든 입력 flags의 OR, 무효 채널이 있으면 InvalidInput 추가 |
| time·sequence·generation·gap | 입력들이 공유하는 값 |
| source_range | 입력 원천 범위 전체를 포함하는 최소 닫힌 구간 |
| origin | 모두 같으면 해당 origin, 서로 다르면 Mixed |

FIR·창은 실제 기여한 ring 구간의 origin을 합친다. 상태 확장 wrapper는 초기화 이후 입력들의 origin을 보수적으로 합친다. Gap 초기화와 checkpoint 복원에서도 이 범위를 유지한다.

## 묶음 수락과 종료

`runtime/ingress.rs`가 원천 ID 순서로 reader를 호출하고 모든 입력의 정확한 유리수 시각·sequence·generation·Gap을 비교한다. 묶음 검사가 성공하면 일정에 따라 커널과 sink를 호출한다. 시각이나 종료 위치가 다르면 E0904로 해당 원천을 표시하고 실행을 종료한다. 실패한 입력 묶음에서 게시한 출력 수는 0이다.

각 원천의 clock과 확정 격자는 계획 생성과 독립 검증에서 검사한다. 실행 중에는 각 reader의 데이터 계약과 각 포트의 시간 격자를 검사한다. 부분 누락을 가진 입력 파일은 공통 시각·Gap 경계로 준비한다. 서로 다른 rate는 재표본화로 연결한다. 서로 다른 clock의 정렬과 비동기 event join은 [시간 설계](design/04-time.md)와 [전체 실행 설계](design/09-runtime.md)의 후속 구현 대상이다.

모든 원천이 함께 EOF에 도달하면 종료한다. `--samples N`은 각 원천에서 최대 N개씩 읽는 공통 상한이다. 파일 원천으로만 구성하면 기본값은 EOF이며, 합성 원천을 포함하면 기본 상한은 1000묶음이다. 서로 길이가 다른 입력의 공통 prefix는 명시적 상한으로 실행한다.

`completed_samples`와 처리율의 단위는 입력 묶음이다. 노드별 records는 해당 노드가 게시한 레코드 수다. Gap·missing_samples는 공통 격자의 누락을 한 번 집계한다. reader마다 출력·선행 슬롯을 재사용하고, reader·sink의 동기 호출이 다음 묶음의 진행 속도를 제어한다. 예약량은 원천별 reader 버퍼·슬롯, 간선 큐, mix 계수 배열과 기존 커널 상태를 합해 검사한다.

## 저장·재생·재개

manifest는 `bci-run/0.6`, JSON Lines는 `bci-recording/0.6`을 사용한다. manifest의 `sources`는 원천 ID 목록, `input_policy`는 `SynchronousExact`, 각 노드의 `source_ports`는 그래프를 거슬러 올라가 도달하는 원천 ID 목록이다. 레코드의 source_range는 이 원천들의 공통 index 범위에 적용하는 보수적 계보다. 재생기는 0.6·0.5·0.4·0.3·0.2와 기존 합성 기록 0.1을 읽는다.

checkpoint의 position은 모든 원천에서 처리한 묶음 수다. prefix hash는 묶음 순서와 각 묶음 안의 원천 ID 순서로 모든 원천을 포함한다. 복원은 모든 입력을 저장 위치까지 읽고 prefix·각 원천 격자를 대조한 다음 커널 상태를 복원한다. 두 번째 이후 원천의 처리 완료 데이터가 바뀐 경우에도 E0909로 진단한다. FIR·창의 origin ring과 상태 확장의 누적 origin도 저장한다.

## 실행 예제와 검사

[multi.bci](../examples/multi.bci)는 [samples.csv](../examples/samples.csv)와 [paired.csv](../examples/paired.csv)를 읽어 합성 → 단위 변환 → FIR → 창 → 공분산·SPD로 처리한다. 두 파일의 값은 서로 다르며 시간·Gap·무효 채널 위치는 같다.

```sh
cargo run --offline --locked -- run examples/multi.bci --output recordings/multi-full
cargo run --offline --locked -- run examples/multi.bci --samples 12 --output recordings/multi-head --checkpoint-out recordings/multi-at-12.json
cargo run --offline --locked -- run examples/multi.bci --resume recordings/multi-at-12.json --output recordings/multi-tail
```

전체 실행은 22묶음을 처리하고 원시 44개·합성 22개·공분산 7개, 총 73개 레코드를 기록한다. 공통 Gap은 1개, 누락 index는 2개다. 앞 12묶음과 복원 구간의 data 행을 합치면 전체 실행과 일치한다.

기존 [CLI 통합 검사](../crates/cli/pipeline.rs)의 함수 하나에 별도 가중 합 계산, 두 포트의 재생, 원천 prefix 변경, 동기 시각·Gap·EOF 오류, 계획 변조와 계수 타입 검사를 모았다. Mixed origin이 FIR·창을 거쳐 전파되는 구간과 중간 복원 결과도 비교한다.

## 유지보수 경계

- compiler는 타입·계수·원천 격자를 검사하고 IR에 모든 입력 연결을 저장한다.
- verify는 원천 목록·일정·mix 계약·자원 합을 독립적으로 검사한다.
- runtime ingress는 묶음을 준비하고, scheduler는 입력 큐가 준비된 노드를 실행한다.
- extension의 `Inputs`는 선언된 입력 포트를 읽는 view다. 기존 단일 입력 커널은 기본 `process_inputs` 구현을 통해 `process`로 연결한다.
- operators/mix.rs는 합성과 품질·계보 병합을 담당한다. artifact는 여러 원천의 연결과 Mixed origin을 기록한다.

11개 패키지와 `crates/<책임>/<파일>.rs` 깊이를 유지한다. 다중 입력 확장은 공통 실행 인터페이스를 사용한다.
