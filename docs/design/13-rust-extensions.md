# 일반 프로그래밍과 Rust 연산자 확장

상태: `bci/0.1` 설계 초안. 이 문서는 BCi 표면 언어와 Rust 구현의 책임, 사용자 연산자 API, 등록 매크로와 배포 계약을 정의한다. 코드와 매크로는 목표 SDK 인터페이스를 나타낸다. 원시 연산자의 의미 계약은 [03](03-semantics.md), registry 필드는 [05](05-operators.md)를 따른다.

현재 구현은 pointwise 함수와 단일 입력·동일 격자 stateful impl의 등록 매크로·TOML 계약·exact-bit reference·compiler 연결·정적 번들 실행을 제공한다. 명령과 API는 [Rust 확장 사용법](../extensions.md), 상태·전체 순차 계획의 저장과 복원은 [checkpoint 안내](../checkpoints.md)에 기록한다. 아래 일반화 API의 다중 포트·매개변수 codec·RejectRecord·epoch migration과 자동 bundle build·registry exporter·bci.lock은 후속 구현 계약이다.

## 언어의 책임

BCi는 장치·측정량·시간·학습 계보가 붙은 처리 그래프를 선언한다. 일반 알고리즘은 Rust 함수·구조체·trait로 작성하고, 등록한 연산자 경계에서 BCi 그래프에 연결한다. 이 분담으로 범용 제어 흐름, 메모리 관리와 라이브러리 활용을 Rust 도구 체계에 맡긴다.

| 기능 | BCi의 표현 | Rust의 책임 |
|---|---|---|
| 이름과 상수 | `let`, `const`, 정적 산술·비교 | 커널 내부 지역 변수와 계산 |
| 배열·설정 객체 | 고정 길이 배열, 레코드, artifact 참조 | 알고리즘 자료구조와 직렬화 구현 |
| 반복 사용 | 매개변수가 있는 `graph` 호출 | 함수·모듈·trait·generic 구현 |
| 구성별 그래프 선택 | 모듈과 `run` 진입점 선택 | 복잡한 설정 계산과 구성 artifact 생성 |
| 데이터에 따른 조건 | 등록한 결정·선택 연산자 호출 | `if`, `match`를 이용한 값 계산 |
| 반복과 재귀 | 연산자에 입력 크기·매개변수 전달 | `for`, `while`, 재귀 알고리즘 |
| 누적 상태 | 상태 연산자의 인스턴스와 epoch 연결 | 구조체 상태와 전이 구현 |
| 오류 | 품질·Reject·Fault 정책 연결 | `Result`와 도메인 오류 생성 |
| 동시 실행 | actor·일정·자원 계약 | 런타임이 배정한 실행 문맥에서 커널 수행 |
| 장치·파일·시간 관측 | capability와 source/sink 선언 | 해당 adapter와 런타임 인터페이스 |

`bci/0.1`의 일반 실행문 집합은 [01](01-language.md)의 `let`, 호출, graph 반환으로 고정한다. 사용자 정의 범용 함수·반복문·재귀·closure의 작성 위치는 Rust 확장 crate다. BCi 파일의 파서는 그 선언형 문법을 담당하고, Rust 컴파일러는 확장 crate의 본문을 처리한다.

## 구성 시점과 실행 시점

### 구성 시점

BCi 상수 평가는 고정 입력의 산술·배열·레코드와 등록한 유한 기술자 생성자를 평가한다. `.bci`에 나타난 `name(...)` 형태의 호출은 symbol의 종류에 따라 다음처럼 처리한다.

| symbol 종류 | 처리 시점 | 산출물 |
|---|---|---|
| 상수·기술자 생성자 | check/build 또는 명시한 bind 구체화 | 값과 타입·구성 의무 |
| 사용자 `graph` | 그래프 구체화 | 연산자 인스턴스와 연결 |
| Rust 원시 연산자 | 구체화 시 노드 생성, 실행 시 커널 호출 | TypedNode와 실행 출력 |

상수 문맥에서 stream 연산자를 호출하면 `E0211 PhaseMismatch`를 생성한다. 원시 연산자의 Rust 본문 실행은 실행 계획의 actor 호출에 속한다. 컴파일 시 기술자 생성자는 compiler registry가 제공하는 제한된 생성자 집합에서 선택한다.

예를 들어 8채널과 16채널의 구성이 다르면 `run EightChannel`과 `run SixteenChannel`을 선언하고 `bci build demo.motor.EightChannel --locked`처럼 진입점을 선택한다. 공통 부분은 `graph`로 공유한다. 해당 진입점의 장치 설정은 bind에서 실제 descriptor와 대조한다.

반복적인 구성 생성이 큰 작업은 Rust 도구가 schema·계수·구성 artifact 또는 `.bci` 소스를 생성한다. 생성 소스에도 동일한 이름·타입·효과·자원 검사를 적용하고, 생성기 버전·입력·출력 digest를 provenance에 기록한다. 구성 조건이 실행 도중 바뀌면 [09](09-runtime.md)의 새 계획·epoch barrier 절차를 수행한다.

### 실행 시점

신호 임계값 비교, 특징 추출의 반복 계산, 모델 내부 조건 분기는 Rust 커널이 수행한다. 한 번의 호출에서 읽을 입력·상태·매개변수와 생성할 출력은 OperatorSpec에 고정한다. `while`·재귀를 사용하는 커널은 입력 영역에 따른 종료 조건, 최대 출력 수·작업 공간을 제공한다. hard deadline 경로는 여기에 적용 환경의 호출시간 Bound 증거를 요구한다.

`control.select(condition, when_true: a, when_false: b)`는 같은 tick의 조건과 두 입력을 수락한 뒤 한 값을 선택한다. a와 b는 같은 포트 타입·tick 대응을 요구하며, 두 생산자의 상태 전이는 계획에 따라 실행된다. 선택 결과의 lineage에는 조건과 수락한 입력들을 보수적으로 포함한다. 일반적인 Rust의 단축 조건 실행은 한 커널 본문 안에서 구현한다.

조건부 외부 동작은 결정 연산자가 `Select`/`Hold` 또는 명령 사건을 생성하고 하나의 명시적 sink가 해석한다. Hold는 장치 호출 수 0, 명령 사건은 dispatch·receipt 계약을 따른다. 이 구조로 조건 판단과 장치 효과를 각각 기록한다.

## 사용자 연산자의 두 작성 형태

| 형태 | 적합한 계산 | 작성자가 제공하는 내용 |
|---|---|---|
| `pointwise` 함수 | 한 유효 값에서 한 값 생성 | Rust 함수, 수치 영역·오차, 계약 profile |
| `stateful` 연산자 | 필터·창·누적 통계·다중 입출력 | 상태 구조, prepare/step/reset/checkpoint, 전체 포트·자원 계약 |

pointwise wrapper는 block의 유효 원소를 순회하며 함수를 호출한다. invalid 위치는 원래 마스크·원인과 함께 전달하고, payload 취급은 해당 quality profile을 따른다. 표본당 호출과 block 단위 커널 최적화는 같은 관측 관계를 만족해야 한다.

### pointwise 예제

```rust
use bci_extension::operator;

#[operator(
    name = "lab.signal.rectify",
    version = "0.1.0",
    kind = "pointwise",
    contract = "contracts/rectify.operator.toml"
)]
pub fn rectify(value: f64) -> f64 {
    if value < 0.0 { 0.0 } else { value }
}
```

대응하는 계약 파일은 다음과 같다.

```toml
contract_version = "bci-operator/0.1"
extends = "std.contracts.pointwise_f64/0.1"

[signature]
quantity = "electrical.voltage"
schema = "preserve"
unit = "preserve"

[numeric]
input_domain = "finite_f64"
reference = "fixtures/rectify.reference.json"
comparison = "exact_bits"

[resources]
state_bytes = 0
scratch_bytes = 0
```

`pointwise_f64` profile은 f64 벡터의 동일 shape 출력, 1입력/1출력, Live mode, 요구 capability 공집합, 현재 표본 support, lookahead=0, 마스크 전파 품질 정책, 기본 Gap/Epoch/Tail 정책을 제공한다. 예제의 signature가 입력 측정량을 전압으로 제한한다. 마스크 전파는 유효 원소에 수치 함수를 적용하고 invalid 원소의 마스크·원인을 유지하는 규칙이다. 무상태 codec은 빈 byte열이고 fit role·외부 write 집합은 공집합이다. block 출력 buffer와 wrapper metadata는 런타임이 따로 계량한다. reference artifact에는 음수·0·양수·signed zero·범위 경계의 입력/출력 bit 쌍을 넣는다.

profile과 계약 파일을 병합한 완전한 OperatorSpec을 registry에 저장한다. 각 필드에 profile/사용자 선언/검증기의 출처를 남긴다. 같은 채널 schema를 유지하면서 연산자 ID·계수·입출력 범위를 lineage에 추가한다. 호출시간 evidence는 별도 resource profile에서 제공한다.

BCi의 호출은 기존 함수 호출·pipe 문법을 사용한다.

```bci
use lab.signal as lab_signal;

let rectified = volts |> lab_signal.rectify();
```

위 BCi 코드는 `volts`가 선언된 run 본문의 호출 부분이다. import는 해당 모듈의 최상위에 둔다. dtype·quantity·schema와 계약 profile이 호출 포트에 적용된다.

### 상태 연산자 API

다음 Rust 서명은 SDK의 상태 연산자 경계를 정의한다. 등장하는 문맥·buffer·codec 타입은 SDK가 제공하는 타입이다.

```rust
pub trait StatefulOperator: Sized {
    type Params: ParameterCodec;

    fn prepare(
        params: &Self::Params,
        ctx: &mut PrepareContext,
    ) -> Result<Self, OperatorFault>;

    fn step(
        &mut self,
        input: InputEventView<'_>,
        ctx: &mut StepContext<'_>,
        output: &mut OutputBatch<'_>,
    ) -> Result<StepOutcome, OperatorFault>;

    fn reset(&mut self, cause: ResetCause) -> Result<(), OperatorFault>;
    fn checkpoint(&self, dst: &mut StateWriter<'_>) -> Result<(), OperatorFault>;
    fn restore(&mut self, src: &mut StateReader<'_>) -> Result<(), OperatorFault>;
}
```

상태 타입에는 `#[operator(name = "lab.signal.running_mean", version = "0.1.0", kind = "stateful", contract = "contracts/running_mean.operator.toml")]`를 붙인다. 매크로는 해당 타입의 StatefulOperator 구현을 요구하는 factory와 wrapper를 생성한다.

PrepareContext는 확정한 dtype·shape·schema, 검증한 매개변수, 예약 allocator와 불변 artifact를 제공한다. StepContext는 현재 사건·논리 tick, 허용한 timer/model 관측, 의존성 수집기와 scratch lease를 제공한다. InputEventView의 borrow 수명은 step 호출 범위다. 입력을 다음 호출까지 보관하는 연산자는 계획에 포함된 소유 lease를 요청한다.

OutputBatch는 계획에서 예약한 수·byte 범위의 출력만 수락한다. 일반 수치 커널은 값·품질·의존성 참조를 반환하고, wrapper가 record ID·epoch·availability·lineage를 생성한다. 원천 timestamp의 생성은 source adapter 계약에 속한다. StepOutcome은 Emit 또는 RejectRecord이며, 오류 Result는 Fault 경로로 전달한다.

wrapper는 step 동안 출력을 staging하고 성공 시 게시한다. Fault 뒤의 actor 상태는 `Faulted`로 이동하며, 복구는 checkpoint 또는 초기 상태에서 시작한다. RejectRecord가 상태에 미치는 효과는 연산자 계약에 명시한다. reset 원인은 Gap, EpochChange, ExplicitReset이고 migration을 제공하는 연산자는 별도 검증 계약을 등록한다.

## 매크로가 생성하는 코드와 검사 책임

| 대상 | 등록 매크로·SDK | 추가 증거·실행 검사 |
|---|---|---|
| Rust 함수·trait 서명 | 허용 함수형·associated type·adapter 생성 | Rust compiler가 본문과 타입 검사 |
| 매개변수 | 이름·기본값·codec 연결 | BCi kind·unit·정제 조건 검사 |
| registry | 이름·버전·profile 병합·계약 digest | 누락 필드·ID 충돌·kernel 연결 검사 |
| block과 수명 | view·output wrapper 생성 | lifetime·예약량·출력 수 검사 |
| 시간·품질·계보 | 공통 profile 전파와 의존성 수집 | 인과성·상태·수치 corpus 검증 |
| 상태 | factory·codec 호출 연결 | reset·restore·migration 불변식 검사 |
| 호출시간·종료 | 취소 확인 지점과 계량 연결 | 입력 영역·Bound/Estimate 증거 검사 |

매크로의 자동 생성 범위는 서명과 선언 계약에서 결정되는 연결 코드다. Rust 함수의 실제 수치 동작·숨은 의존성·호출시간은 원시 연산자 적합성 검사의 대상이다. `reference_kernel`, numeric tolerance와 state invariant의 증거는 [11](11-conformance.md)의 corpus와 보고서로 제출한다.

일반 계산 커널의 관측 의존성은 입력·상태·명시적 매개변수·허용한 StepContext로 구성한다. 파일 I/O·장치 I/O·wall clock·난수·별도 작업 실행을 사용하는 구현은 해당 효과를 등록하고, source/sink·Timer·seed/state·executor 자원으로 그래프에 노출해야 한다. 이 조건은 인과성·재생·자원 계약을 유지하는 확장 작성 규칙이다.

in-process 확장은 같은 process 권한으로 실행하는 Rust 코드다. 정적 registry와 wrapper는 계약 및 경계 검사를 제공한다. 개발 조직은 확장 소스·의존성·빌드 산출물을 검토해 실행 kernel을 등록한다. 외부 process를 사용하는 계산은 IPC source/sink 경계의 시간·메모리·실패 계약을 추가한다.

## 빌드, 등록과 배포

사용자 프로젝트는 `extensions/lab-signal/`에 Rust crate, 계약 파일, reference fixture를 둔다. Cargo는 Rust 의존성·본문·매크로를 빌드하고, BCi는 완성된 registry descriptor와 연산자 의미를 검사한다. 패키지 선언은 다음 경계를 사용한다.

```toml
[extensions.lab_signal]
manifest = "extensions/lab-signal/Cargo.toml"
registry = "registry/lab-signal.json"
namespace = "lab.signal"
```

`bci extension build lab_signal --locked`는 extension의 Cargo.lock과 고정 toolchain 설정을 사용해 registry exporter 및 runtime bundle을 빌드한다. exporter는 계약을 완전한 OperatorSpec으로 내보내고, bundle은 표준·사용자 kernel을 함께 정적 등록한다. 빌드 작업은 선택한 프로젝트 코드와 Cargo build script를 실행하는 개발 도구 작업이다. `bci check`는 생성된 descriptor를 읽어 소스 진단을 수행한다.

등록 artifact는 `extension_api_version, registry_schema_version, operator_specs, kernel_ids, contract_digests, reference_digests, source_digest, cargo_lock_digest, toolchain_id, target, feature_set, runtime_bundle_digest`를 가진다. `bci.lock`은 이 레코드를 고정한다. build/bind가 고른 kernel ID와 실제 bundle의 registry를 run 전에 대조한다. 이 방식의 배포 단위는 정적으로 연결한 runtime bundle이다.

Rust generic은 extension에서 선택한 f32/f64·상수 크기 specialization으로 구체화한다. runtime descriptor의 동적 채널 수는 검사된 slice/view로 전달한다. BCi 타입 checker는 등록한 signature와 refinement를 기준으로 호환성을 판단한다. 사용자 generic·trait 선언은 확장 Rust 코드에 작성한다.

계약·kernel·reference의 변경은 관련 digest와 bundle을 갱신한다. checkpoint의 state codec version이 바뀌면 명시한 migration 또는 상태 초기화 정책을 선택한다. `.bci` 호출 span과 Rust 함수 위치는 registry source map으로 연결하여 오류의 양쪽 위치를 표시한다.

## 오류와 자원 경계

| 조건 | 진단·동작 | 해결 입력·행동 |
|---|---|---|
| 상수 평가에 stream kernel 사용 | E0211 PhaseMismatch | 기술자 생성자 또는 run 내부 노드 호출 선택 |
| registry의 계약·함수 wrapper 불일치 | E0510 ExtensionContractMismatch | signature·codec·contract 재생성 |
| 요구하는 SDK/registry 판본 불일치 | E0901 ArtifactVersionMismatch | 호환 bundle과 lock 선택 |
| 출력 수·byte 상한 초과 | E0907 OperatorBudgetExceeded, Fault | 계약과 kernel 수정 후 계획 재검증 |
| kernel 호출의 panic | E0908 OperatorPanic, actor Fault | source 위치·입력·계약 digest로 원인 추적 |
| 종료·시간 상한 증거 대기 | Pending obligation | 입력 영역과 호출시간 evidence 제공 |

panic unwind를 지원하는 bundle은 wrapper의 unwind 경계에서 상태를 Faulted로 처리한다. abort 설정·native process 종료는 supervisor가 process fault로 기록한다. 실행 deadline monitor는 위반을 감지하고 계획의 fault 정책을 수행한다. 실행 중인 native 함수의 강제 중단이 필요한 배포는 process 격리 경계에서 종료 정책을 제공한다.

## 수용 조건

1. pointwise 함수에서 registry·wrapper·BCi import를 생성하고 exact-bit reference를 통과한다.
2. signature·profile·dtype·quantity 불일치는 Rust 진단 또는 BCi의 관련 포트 진단으로 반환한다.
3. stateful 연산자는 block 분할 변화, gap reset, checkpoint/restore에 대해 상태·출력·계보 관계를 만족한다.
4. `control.select`는 두 분기의 상태 진행과 조건의 lineage를 보존한다. Hold는 장치 호출 수 0을 기록한다.
5. 구성 진입점 선택은 선택한 graph·config·kernel digest를 manifest에 고정한다.
6. 출력 예산 위반·panic·bundle 불일치에는 지정한 Fault/진단을 반환한다.
7. 확장 연산자를 포함한 기록 재생은 [11](11-conformance.md)의 값·시간·모델·품질 관측 관계를 만족한다.
