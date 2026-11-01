# 소스 검사와 TypedGraph

현재 compiler는 한 `.bci` 모듈을 파싱하고 이름·구성값·연결 타입·Live/Record 효과를 검사해 `TypedGraph`를 생성한다. CLI와 Rust API가 같은 컴파일 경로를 사용한다. 현재 구현은 아래 명령과 지원 범위로 확인할 수 있다.

```sh
cargo run --offline --locked -- check examples/typed.bci
cargo run --offline --locked -- explain examples/typed.bci --view types
```

## 예제의 결과

[typed.bci](../examples/typed.bci)는 두 전압 채널을 250 Hz로 선언하고, 단위 변환·사용자 graph의 FIR·창·공분산·SPD 정규화·기록 sink를 연결한다. 스키마와 계수는 같은 디렉터리의 JSON 기술자에서 읽는다. 상수 `Hop`은 뒤에 선언한 `BaseHop`을 참조한다.

정상 입력의 정적 검사 결과는 graph 정의 1개, run 1개, 실행 그래프 노드 7개다. 표본수 250·hop 25의 창은 10 Hz로 발생한다. 창 내부 표본률 250 Hz와 창의 발생률 10 Hz를 별도 필드에 보관한다. 공분산 출력 단위는 V²다.

예제의 Pending 의무는 다음 세 항목이다.

| 종류 | 확정할 조건 | 소유 경계 |
|---|---|---|
| WindowPhase | 첫 연속 250개 valid 표본의 시작 위상 | 실행 창 상태 |
| NumericGuard | 출력 Cholesky 검사 후 SPD refinement 게시 | 실행 수치 커널 |
| Storage | 기록 경로의 용량·처리율·열기 조건 | prepare·기록 sink |

`check`는 정적 검사 성공 시 종료 코드 0과 Pending 목록을 출력한다. 소스 오류는 코드 2, 파일·기술자 읽기 오류는 코드 5다. 오류에는 파일·줄·문자 열, 원본 소스 줄과 수정 조건을 표시한다.

## 지원하는 소스 구성

| 구성 | 현재 처리 |
|---|---|
| 언어·모듈 선언 | `language "bci/0.1"`, 단일 모듈의 이름 공간 |
| import | 등록한 표준·사용자 모듈과 별칭 |
| 최상위 구성 | const, schema, clock, device, resource |
| 값 | 정확한 십진·지수 상수, 단위, bool, JSON 문자열 escape, 배열·튜플·레코드 |
| 정적 연산 | 산술·비교·논리식, 레코드 필드 참조, 전방 선언 참조 |
| graph | Stream 매개변수, 반환 타입, Live/Record 효과, 호출 시 독립 노드 구체화 |
| run | 지역 let, 호출·pipe·sink, 실행 모드와 자원 구성 |
| 주석 | 줄 주석과 중첩 블록 주석 |

파서는 [표면 문법](design/01-language.md)의 선언·표현식 구조를 읽는다. 이름 해석과 의미 검사는 등록한 symbol·생성자·타입 규칙을 기준으로 수행한다. 등록 범위 밖의 호출은 E0103, 선언 kind와 기술자 불일치는 E0201을 반환한다. 일반 프로그래밍의 제어 흐름은 Rust 연산자에 작성한다.

CLI의 단일 파일 모드는 입력 파일의 디렉터리를 artifact 상대 경로의 기준으로 사용한다. Rust API의 `compile(source, base)`는 호출자가 기준 디렉터리를 지정한다. `compile_with_extensions(source, base, specs)`는 정적 번들의 사용자 pointwise·stateful 선언을 추가한다. [Rust 확장 안내](extensions.md)는 등록·타입 검사·번들 조립을 설명한다. 사용자 package manifest·외부 모듈 파일 연결·독립 registry 파일 로딩은 [패키지·확장 설계](design/13-rust-extensions.md)의 후속 구현 대상이다.

## 표준 호출의 의미 검사

| 묶음 | 등록한 호출 | 확인하는 계약 |
|---|---|---|
| schema | `schema.electrical`, `schema.load` | 고유 채널 ID, 순서, 측정량·단위·기준 |
| time | `session_clock`, `device_clock`, `uniform`, `mapping`, `align` | clock 정체성, 양의 rate, 매핑 양 끝과 명목 격자 변환 |
| source | `synthetic`, `acquire`, `assert_schema` | 입력 기술자와 기대 schema·rate |
| signal | `mix` | 같은 Vec 포트 2..64개·유한 무차원 weights·동일 타입 검사 |
| source | `csv`, `replay` | CSV 헤더의 채널·단위, 기록 포트 타입·clock·발생률, 입력 정책 |
| signal | `coefficients`, `convert_unit`, `scale`, `fir`, `filtfilt` | 선형 단위 차원, 무차원 계수, 수치 표현, 모드 |
| signal | `window`, `covariance`, `regularize` | 양의 창 길이·hop, N≥2, 공분산 단위, shrinkage·floor |
| signal | `sos`, `iir` | 정규 SOS·Schur 안정성·설계 표본률·warmup 계약 |
| signal | `psd`, `bandpower`, `log_power` | Spectrum 주파수 축·단측 밀도·대역 경계·특징 단위 |
| model | `load`, `predict` | 모델 digest·입력 특징·클래스·역할 계보 검사와 Probability 출력 |
| decision | `threshold` | 확률 타입·0..1 minimum 검사와 Select·Hold 출력 |
| signal | `resample` | 서로소 비율·정확한 위상·인과 FIR·다중 출력 실행 계약 |
| signal | `select`, `reorder`, `rereference` | Vec·Epoch 채널 축, 고유 ID·순열, 전압 기준·유리수 계수 합 |
| runtime | `budget`, `configure` | 자원 기술자, 모드, 진입점 구성 위치 |
| recording | `write` | Stream 또는 Stream 튜플과 출력 자원 |
| 사용자 pointwise·stateful | 등록한 사용자 namespace의 함수 | Vec<f64,C>, 모든 채널의 quantity, 동일 schema·단위·시간·품질 출력 |

`source.synthetic`는 입력 계약을 명시하는 source 노드를 생성한다. `source.acquire`의 현재 계약은 electrical.voltage 채널과 장치 구성의 channels·output_rate를 검사하고, 실제 프로필·기능·접근 증거는 Binding 의무로 전달한다.

`source.csv`는 선언한 스키마와 CSV 헤더를 비교한다. `source.replay`는 commit manifest와 JSON Lines header에서 선택 포트의 전체 타입을 복원한다. 각 데이터 행의 시간·품질·형식 검사는 [파일 입력](inputs.md)의 Input 의무로 연결한다.

`signal.filtfilt`는 Record 효과와 입력 종료·경계 초기값 의무를 만든다. Live 경로에 연결하면 E0301을 반환한다. 모든 연산자의 입력 효과를 출력으로 전파하고, 기록 자원은 그래프의 writes 집합에 포함한다.

graph의 `! live` 선언은 입력부터 반환까지 Live 효과를 요구한다. Record 효과를 가진 입력을 처리하는 사용자 graph는 `! record`로 선언하며, 호출하는 run도 Mode.Record를 지정한다.

`check`와 `explain --view types`는 타입·연결·구성값·후속 의무를 가진 정적 그래프를 처리한다. `Compilation::execution_plan(entry, samples)`는 선택한 run의 순차 실행 계획을 만든다. `samples: Some(N)`은 동기 입력 묶음 상한, None은 모든 파일의 공통 EOF 또는 합성 입력을 포함할 때 기본 1000묶음을 선택한다. 다중 원천은 동일 clock·확정 Uniform rate·phase를 요구한다. 구체 계약과 예제는 [다중 입력 안내](multi-input.md)를 따른다. 계획은 [실행 안내](runtime.md)의 reader·수치 커널·독립 검증·기록 경로로 전달한다. 장치 획득·일반 SDF 일정·모델 학습은 해당 [상세 설계](design/README.md)의 구현 대상이다.

## 타입과 상수

유리수는 signed 128-bit 분자·양의 분모로 정규화하고 중간 산술을 검사한다. 예를 들어 `0.1`은 1/10, `250[Hz] / 25`는 10 Hz다. 지수 범위·0으로 나누기·표현 범위 초과는 E0501을 반환한다.

단위는 SI 7차원과 저장량 차원을 가진 선형 단위다. V/mV/uV, s/ms/us/ns, Hz/kHz, m/cm/mm/nm, kg/A/K/mol/cd, byte/KiB/MiB/MB 및 곱·나눗셈·정수 거듭제곱을 해석한다. 타입 연결은 단위의 정확한 배율도 비교하고, 배율 변화는 `convert_unit`으로 표현한다.

Stream 타입은 값 구조·전체 채널 의미·clock ID·발생 규칙·품질 어휘를 비교한다. 값 구조는 Vec, Epoch, Cov, SPD를 표현한다. graph의 Stream 매개변수와 반환 타입에서 채널 수는 숫자 또는 정적 정수 선언 이름을 사용할 수 있다.

Epoch 매개변수의 내부 격자는 `Epoch<f64,2,250,250[Hz]>`처럼 네 번째 인수로 명시한다. 값이 주어진 반환·let 타입의 `Epoch<f64,2,250>`는 추론한 내부 격자를 유지한다. `WindowStart.FirstValid`는 `PendingUniform`의 명목 위상 ID를 생성하고, `start: 0` 같은 정수 시작은 입력 위상과 마지막 표본 index로 출력 위상을 계산한다.

## JSON 입력 기술자

스키마 형식은 [eeg.schema.json](../examples/eeg.schema.json)을 따른다. 채널 필드는 id, quantity, unit, reference이며 의미 비교는 이 전체 필드와 순서를 사용한다. 계수 형식은 [smooth.coefficients.json](../examples/smooth.coefficients.json)을 따르고, values를 정확한 십진 문자열 배열로 작성한다. 각 판본은 허용 필드와 자료형을 검사한다.

명목 매핑 기술자는 다음 필드를 사용한다.

```json
{
  "format": "bci-clock-mapping/0.1",
  "from": "demo.typed.Amp.clock",
  "to": "demo.typed.Session",
  "slope": "1",
  "offset_seconds": "0"
}
```

align은 `t_out=slope*t_in+offset`의 명목 rate·phase를 계산한다. epoch·가용 시각·유효 구간·오차의 실제 증거는 ClockMapping 의무에 기록한다. 임의 하드웨어와 시간 정확도의 실행 판정은 이 의무의 해소를 요구한다.

## 소유 파일과 산출물

| 위치 | 책임 |
|---|---|
| schema의 number·unit·types·registry·extension | 정확 산술, 도메인 타입, 공통 호출 서명·사용자 계약 |
| compiler의 lexer·parser·syntax | 원본 byte 범위를 가진 AST |
| compiler의 check | 이름·선언·graph 구체화·미연결 출력 검사 |
| compiler의 evaluate·typing | 구성값 계산과 타입 표기 해석 |
| compiler의 operators | 표준 호출의 타입 변환과 의무 생성 |
| compiler의 plan | 구체 커널·순차 일정·메모리 예산·실행 가능 의무 연결 |
| artifact의 descriptor | 스키마·계수·명목 mapping 로딩 |
| ir의 graph·value | 노드·입력 ID·정확한 인수·효과·위치·의무 |
| ir의 plan | 실행 일정·출력 슬롯·커널 상태 예약량 |

TypedGraph는 설명 문자열과 별도로 타입 있는 인수를 저장한다. 필터 계수 전체, 단위, 자원 구성과 장치 설정이 다음 컴파일 단계로 전달된다. 설명 출력은 큰 구성의 요약을 사용한다.

검증은 [공통 예제](../examples/typed.bci)와 [두 개의 통합 검사](../crates/compiler/scenarios.rs)를 사용한다. 예제 변형으로 핵심 계약 오류를 만들고 같은 컴파일 경로에서 판정한다. `tools/check.py`는 이 검사를 Cargo test로 실행하며 같은 예제를 CLI 검사·타입 설명에도 사용한다.

[채널 변환](channels.md)은 실행 계획에 index 매핑과 정확한 재기준화 계수를 저장한다. 기준 ID 계산과 출력 타입 규칙은 schema/spatial.rs가 소유하며 compiler와 독립 verifier가 각각 적용한다.

[주파수 특징](spectral.md)은 Epoch에서 Spectrum·대역 전력·로그 특징으로 연결한다. Spectrum 타입 인수는 dtype·채널 수·원래 DFT 표본 수·내부 표본률이다. 측정량 스키마에 taper·대역·로그 구성의 의미를 보존한다.
