# 채널 선택·재정렬·전압 재기준화

`signal.select`, `signal.reorder`, `signal.rereference`는 Vec·Epoch의 채널 축을 변환한다. 현재 실행 커널은 f64 값을 처리한다. 채널당 표본 수, Epoch 내부 표본률, 출력 발생 격자·시각·generation·Gap은 입력에서 이어받는다.

## 호출과 타입

```bci
let ordered = raw |> signal.reorder(channels: ["C4", "C3"]);
let referenced = ordered |> signal.rereference(weights: [0.5, 0.5]);
let selected = referenced |> signal.select(channels: ["C3"]);
```

| 호출 | 입력 조건 | 출력 |
|---|---|---|
| `select(channels: ids)` | 입력에 존재하는 고유 ID를 한 개 이상 지정 | ids 순서의 채널·값·mask |
| `reorder(channels: ids)` | 전체 입력 ID의 순열 | 같은 채널 수의 재배열 |
| `rereference(weights: w)` | 채널당 무차원 계수 하나, 정확한 합 1, 동일 전압 단위·기존 기준 | 새 기준 ID를 가진 같은 크기의 전압 값 |

선택·재정렬은 채널 ID·측정량·단위·기준을 함께 이동한다. 예를 들어 `Epoch<f64,2,4>`에서 C4만 선택하면 `Epoch<f64,1,4>`가 된다. 시간 격자와 채널당 4개 값의 순서는 그대로 유지한다. 공분산 포트의 채널 변환은 [전체 공간 연산 설계](design/05-operators.md)의 별도 행렬 계약으로 확장한다.

재기준화의 모든 입력 채널은 `electrical.voltage`, 같은 단위 차원·배율과 같은 기존 reference를 요구한다. 입력 단위 배율은 먼저 `signal.convert_unit`으로 맞춘다. 채널별로 서로 다른 기준을 가진 입력은 공통 측정 기준의 입력 스키마로 구성한다.

weights는 BCi 정적 유리수로 계산한다. `[1/3, 1/3, 1/3]`의 합은 정확히 1이다. `[0.333, 0.333, 0.333]`의 합은 999/1000이므로 컴파일러가 합계 오류를 반환한다. 음수 가중치도 합이 1인 조건에서 사용한다. `[1,0]`은 첫 채널 기준, `[0.5,0.5]`는 두 채널의 평균 기준이다.

## 수치와 품질

재기준화는 각 채널·표본마다 다음 식을 계산한다.

```text
A[c,j] = (c == j ? 1 : 0) - weights[j]
y[c,s] = Σ A[c,j] * x[j,s]
```

A의 대각·비대각 계수는 정확한 유리수로 계산한 뒤 f64로 변환한다. 입력 채널 순서로 곱과 합을 수행하고, 0 계수는 건너뛴다. 같은 수학적 기준의 입력 재정렬은 f64 덧셈 순서를 바꾸므로 마지막 비트에 반올림 차이가 생길 수 있다. 비유한 유효 출력은 런타임 수치 guard의 E0903으로 진단한다.

재기준화 출력 채널의 valid는 A의 0이 아닌 계수에 해당하는 입력 valid의 AND다. 무효 출력 채널의 값은 0으로 채운다. 모든 계수가 0인 행은 0·valid=1을 출력한다. 따라서 첫 채널 기준의 첫 출력은 원천 첫 채널이 무효인 경우에도 정확한 0이다. 다른 출력은 실제 기여 채널의 품질을 따른다.

선택·재정렬은 선택한 채널의 값과 mask를 복사하므로 무효 채널의 저장 payload도 보존한다. 세 연산자 모두 출력 mask로 InvalidInput flag를 다시 계산하고, Warmup·Gap 등 나머지 flags는 이어받는다. 제외한 채널만 무효였던 입력은 선택 후 valid 출력으로 진행할 수 있다.

sequence·정확한 시각·generation·Gap·source_range·origin은 입력 레코드의 값을 보존한다. source_range와 origin은 레코드 전체의 보수적 계보이며, 채널별 기여 관계는 계획의 index 매핑과 재기준화 계수로 표현한다.

## 기준 ID와 기록

재기준화 출력의 reference는 `rereference:sha256:<digest>`다. digest 입력은 다음 순서로 구성한다.

1. 계약 판본 `bci-rereference/0.1`.
2. 공통 기존 reference의 길이와 UTF-8 값.
3. 0 이외의 가중치를 가진 채널을 ID 순으로 정렬한 뒤 각 ID와 기약 유리수 가중치의 길이·값.

길이는 little-endian u64 byte 수다. 동치 유리수 표기와 채널 재정렬에 맞춘 계수 재배치는 같은 기준 ID를 만든다. 전압 단위 변환도 이 물리적 기준 ID를 보존한다. 변경한 가중치나 기존 기준은 새 ID를 만든다. 기준 ID는 동일한 변환 구성을 식별하며, 입력 의미의 정체성은 전체 채널 스키마·원천 연결과 함께 검사한다.

manifest의 channel_map 커널에는 입력 채널 수·index 배열·채널당 값 수를 기록한다. rereference 커널에는 정확한 유리수 가중치 문자열·계산 순서·계수 반올림·품질 규칙을 기록한다. 각 출력 스키마는 변환한 reference·순서·단위를 보관한다. 재생 reader는 이 스키마와 기록 값을 함께 복원한다. 파일 형식은 `bci-run/0.6`·`bci-recording/0.6`이다.

## 실행과 복원

[channels.bci](../examples/channels.bci)는 CSV → 재정렬 → 평균 재기준화 → C3 선택 → 단위 변환 → FIR → 창 → 공분산·SPD를 실행한다.

```sh
cargo run --offline --locked -- run examples/channels.bci --output recordings/channels-full
cargo run --offline --locked -- run examples/channels.bci --samples 12 --output recordings/channels-head --checkpoint-out recordings/channels-at-12.json
cargo run --offline --locked -- run examples/channels.bci --resume recordings/channels-at-12.json --output recordings/channels-tail
```

전체 실행은 입력 22개를 처리하고 원시·재정렬·재기준화·선택 포트 각 22개와 공분산 7개, 총 95개를 기록한다. 이 예제는 무효 채널과 2개 index의 Gap을 포함한다. 입력 12개 뒤 저장한 checkpoint로 재개한 결과를 앞 기록에 이어 붙이면 전체 실행과 일치한다.

채널 변환 커널의 지속 상태는 고정 index·계수 배열이다. checkpoint의 호출 간 상태 payload는 0 byte이고, 복원은 계획에 있는 구성으로 커널을 다시 만든다. 가중치·채널 순서·reference 변경은 계획 digest를 바꾸므로 E0909로 다른 실행 계보를 식별한다. downstream FIR·창의 상태는 기존 codec으로 저장한다.

## 자원과 구현 위치

출력 값·mask는 prepare 단계에서 예약한다. 선택 커널은 출력 채널 수만큼 usize index를 소유한다. 재기준화 커널은 채널 수 C에 대해 f64 대각·비대각 배열 2개, 총 16C byte를 소유한다. 커널별 고정 제어 예약량은 기존 1024 byte를 적용한다. 재기준화의 레코드당 연산량은 O(C²N), 선택은 O(KN)이며, N은 채널당 표본 수이고 K는 출력 채널 수다. 수치 처리 루프는 예약한 배열을 재사용한다.

| 위치 | 책임 |
|---|---|
| schema/spatial.rs | 채널 shape·순열·정확한 합계·기준 ID 계약 |
| compiler/operators.rs·plan.rs | 호출 인수 검사·채널 이름 해석·구체 index와 계수 계획 |
| verify/sequential.rs | 입력·출력 shape·기준·매개변수·예약량 재검증 |
| operators/spatial.rs | 채널 복사·재기준화 수치·출력 mask |
| schema/execution.rs | 채널 수가 변하는 Frame의 공통 metadata 전달 |
| artifact/recording.rs | 커널 구성과 출력 스키마 기록 |

11개 패키지와 기존 파일 깊이를 유지한다. schema의 기준 ID 계산은 workspace에 잠긴 sha2를 공유한다. [CLI 통합 검사](../crates/cli/pipeline.rs)의 함수 하나에서 Vec·Epoch 수치 참조, 품질 의존성, 기준 ID, 재생·복원과 잘못된 채널·가중치·계획을 함께 확인한다.
