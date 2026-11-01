# SOS IIR 필터와 상태 복원

`signal.sos`는 파일에 정의한 고정 필터를 읽고, `signal.iir`는 각 채널의 이전 출력을 피드백 상태로 사용하는 인과 필터를 실행한다. 현재 profile은 f64 Vec 입력, 순서 있는 SOS 절 1..64개, zero 초기 상태와 transposed direct form II 실현이다.

```bci
const Filter = signal.sos("smooth.sos.json");
let filtered = raw |> signal.iir(realization: Filter);
```

입력 채널·측정량·단위·clock·발생 규칙은 출력에서 보존한다. 필터의 설계 표본률은 입력 발생률과 정확히 같아야 한다. Uniform과 PendingUniform 포트 모두 연결하며, 레코드 간 시간 연속성은 실행기의 GridCursor가 검사한다. 입력 sequence가 원천 창의 anchor index인 특징 벡터도 해당 발생률의 필터로 연결할 수 있다.

## 파일 계약과 안정성

[smooth.sos.json](../examples/smooth.sos.json)은 두 절을 직렬 연결한 수치 예제다.

```json
{
  "format": "bci-sos/0.1",
  "sample_rate_hz": "250",
  "warmup_samples": 2,
  "sections": [
    ["0.375", "0", "0", "1", "-0.75", "0.125"],
    ["0.75", "-0.75", "0", "1", "-0.5", "0"]
  ]
}
```

각 행은 `b0,b1,b2,a0,a1,a2` 순서의 정확한 십진 문자열이다. a0는 0 이외의 값을 요구하며, loader는 나머지 다섯 계수를 a0로 나눠 정규화한다. 표본률도 정확한 십진 문자열을 사용한다. 파일 상한은 1 MiB, warmup_samples 범위는 0..1000000이다. 필드·배열 길이·계수 표현·자원 상한 위반은 구성 진단으로 반환한다.

정규 분모 `z²+a1*z+a2`의 모든 극점이 단위원 안에 있도록 다음 값을 검사한다.

```text
1+a1+a2 > 0
1-a1+a2 > 0
1-a2    > 0
1+a2    > 0
```

컴파일러는 정규화한 유리수로 네 조건을 확인하고, 실제 f64 계수로 계산한 네 값에도 각각 `> 1e-12`의 여유를 요구한다. 이 수치는 Schur 부등식 값의 여유다. 독립 verifier는 실행 계획의 같은 계약을 다시 검사한다. warmup 길이는 파일에 선언한 유효성 정책이며 필터 작성자가 선택한다.

실행 계약 digest는 `bci-iir-sos-df2t/0.1`, 설계 표본률, warmup과 정규 계수들을 순서대로 해시한다. 각 필드는 little-endian u64 byte 길이와 UTF-8 기약 유리수 문자열로 직렬화한다. 같은 절의 여섯 계수를 공통 배수로 바꿔도 정규화 결과가 같으면 digest가 같다. digest는 정규 실현의 의미 필드를 식별하며, 실행 manifest에 실제 정규 계수와 digest를 함께 저장한다.

## 계산과 품질

한 절은 현재 입력 x와 두 지연 상태로 다음 순서의 f64 연산을 수행한다. y를 다음 절의 입력으로 전달한다.

```text
y       = b0*x + z1
next_z1 = b1*x - a1*y + z2
next_z2 = b2*x - a2*y
```

모든 채널·절의 상태는 준비 시 0이다. 출력 또는 중간 지연 값이 비유한 수치이면 E0903으로 종료한다. warmup으로 무효 표시한 구간에서도 지연 상태의 유한성을 검사한다.

- valid=1인 입력은 채널의 모든 절을 갱신한다. 초기화 이후 처음 W개 유효 입력의 출력은 계산값·valid=0·Warmup이며, W+1번째부터 valid=1이다. W=0이면 첫 유효 입력부터 valid=1이다.
- valid=0인 입력은 해당 채널의 지연 상태·warmup 카운터를 초기화하고 0·invalid·InvalidInput을 출력한다. 다른 채널은 자신의 상태와 카운터로 계산한다.
- 명시적 Gap 또는 generation 변경은 모든 채널의 지연 상태·카운터를 초기화한다. 경계 레코드부터 새 warmup을 계산한다.

입력 flags는 전파하고 IIR가 발생시킨 Warmup·InvalidInput을 추가한다. 시간·sequence·generation·Gap·source_last는 현재 입력에서 유지한다. source_first는 전체 상태 초기화 이후 입력 원천 범위의 최솟값이다. origin은 같은 구간에 기여한 입력 유래를 합치며 서로 다른 유래가 있으면 Mixed다. 이 범위와 origin은 채널별 초기화를 포함하는 보수적 레코드 계보다.

## 기록과 checkpoint

manifest는 커널 `sos-df2t/0.1`, codec `sos-state/0.1`, 표본률, 정규 계수 순서, warmup, 안정성 조건, 초기값·무효 채널·Gap 정책과 digest를 기록한다. 기록 포트의 값·품질·시간·계보는 기존 0.4 파일 형식을 사용한다.

checkpoint에는 각 채널·절의 z1·z2, 채널별 warmup 카운터, 이전 generation·sequence, 누적 source_first·origin을 저장한다. 복원은 codec 판본·payload 길이·유한 상태·카운터 상한·metadata 필드 관계를 검사한다. 처리 전 상태의 payload는 0 상태를 요구한다. 정상 준비·저장·복원 경계는 기존 [checkpoint 계약](checkpoints.md)을 따른다.

필터 표본률·계수·warmup 변경은 계획 digest를 바꾼다. 같은 실행 계획으로 재개하면 계산 상태가 이어지고, 원천 prefix·계획·상태 payload 불일치는 기록 준비 전에 진단한다. 원시 포트를 재생하여 같은 필터를 실행하면 동일한 입력 순서·Gap·채널 초기화 규칙을 적용한다.

## 실행 예제와 검사

[iir.bci](../examples/iir.bci)는 CSV → 전압 단위 변환 → SOS IIR → 창 → 공분산·SPD를 실행한다. 원시 22개·IIR 22개·공분산 7개, 총 51개 레코드를 기록한다.

```sh
cargo run --offline --locked -- run examples/iir.bci --output recordings/iir-full
cargo run --offline --locked -- run examples/iir.bci --samples 12 --output recordings/iir-head --checkpoint-out recordings/iir-at-12.json
cargo run --offline --locked -- run examples/iir.bci --resume recordings/iir-at-12.json --output recordings/iir-tail
```

기존 [통합 검사 파일](../crates/cli/pipeline.rs)에 검사 함수 하나를 추가했다. 별도의 직접 차분방정식 계산과 출력 값을 절대 오차 1e-18로 비교하고 채널별 mask·flags·계보를 대조한다. 입력 1·7·12·17개 뒤 저장·복원한 전체 기록 51개를 연속 실행과 비교한다. 경계는 초기 warmup, 무효 채널 직후, Gap 직전·이후를 포함한다. 원시 재생, 정규화 digest, 불안정 계수·잘못된 표본률·잘린 payload·비유한 상태도 검사한다.

## 자원과 유지보수

채널 수 C, SOS 절 수 S에 대해 f64 계수는 40S byte, 지연 상태는 16CS byte, warmup 카운터는 8C byte다. 기존 제어 예약 1024 byte와 출력 Frame·값·mask를 더해 예산을 검사한다. prepare에서 배열을 예약하고 처리 루프에서 재사용한다. 레코드당 연산량은 O(CS)다.

| 파일 | 책임 |
|---|---|
| schema/iir.rs | 정규 실현·안정성·표본률·digest·자원 계약 |
| artifact/descriptor.rs | 파일 형식·유리수 a0 정규화 |
| compiler/operators.rs·plan.rs | 구성 함수·타입 검사·구체 커널 연결 |
| verify/sequential.rs | 계획의 실현·입출력·자원 재검증 |
| operators/iir.rs | 채널별 DF-II-T 계산·초기화·상태 codec |
| artifact/recording.rs | 실현과 실행 정책 기록 |

11개 패키지와 기존 Rust 파일 깊이를 유지한다. 일반 상태공간 실현과 자동 필터 설계는 [전체 연산자 설계](design/05-operators.md)의 확장 대상이다.
