# Rust 사용자 연산자와 정적 실행 번들

Rust의 조건문·반복문·함수를 BCi 그래프의 연산자로 연결한다. `pointwise_f64/0.1`은 유효 원소마다 Rust 함수를 호출하고, `stateful_f64/0.1`은 누적 상태와 레코드별 호출을 제공한다. 컴파일러가 Vec<f64,C>와 측정량을 검사한다. 표준 커널과 사용자 함수는 같은 실행 계획·기록·재생 경로를 사용한다. 상태 SDK와 저장·복원 절차는 [checkpoint 안내](checkpoints.md)에 정의한다.

## 실행 예제

[lab_signal.rs](../crates/cli/lab_signal.rs)의 rectify는 음수를 0으로 바꾸고 나머지 값을 보존한다. [lab.rs](../crates/cli/lab.rs)가 이 함수를 등록한 정적 번들을 조립한다. [extension.bci](../examples/extension.bci)는 합성 전압을 V로 변환한 뒤 사용자 함수를 호출하고 원시·변환 포트를 기록한다.

```sh
cargo run --example bci-lab --offline --locked -- extensions
cargo run --example bci-lab --offline --locked -- extensions check
cargo run --example bci-lab --offline --locked -- check examples/extension.bci
cargo run --example bci-lab --offline --locked -- run examples/extension.bci --samples 1000 --output recordings/extension
```

`extensions`는 등록 이름·버전·profile·digest·Rust 위치를 출력한다. `extensions check`는 실제 함수에 exact-bit 참조 입력을 적용한다. `check`·`explain`은 등록된 선언 데이터를 사용하고, `run`은 사용한 함수의 reference를 prepare에서 검사한 뒤 입력 처리를 시작한다. 기본 `bci` 번들은 표준 연산자 집합을, `bci-lab`은 표준 집합과 lab.signal.rectify를 연결한다.

## 함수와 계약 작성

```rust
use bci_extension::operator;

#[operator(
    name = "lab.signal.rectify",
    version = "0.1.0",
    kind = "pointwise",
    contract = "rectify.operator.toml"
)]
pub fn rectify(value: f64) -> f64 {
    if value < 0.0 { 0.0 } else { value }
}
```

매크로는 원래 함수와 `rectify_registration()`을 생성한다. 함수 서명은 일반 Rust의 `fn(f64) -> f64`이며 인수 이름은 자유롭게 정한다. 이름은 사용자 namespace의 점으로 구분한 식별자, 버전은 major.minor.patch를 사용한다. 매크로 인수는 name·version·kind·contract를 각각 한 번 지정한다.

contract 경로는 확장 crate의 `Cargo.toml` 디렉터리 기준이다. [rectify.operator.toml](../crates/cli/rectify.operator.toml)은 다음 계약을 선언한다.

```toml
contract_version = "bci-operator/0.1"
extends = "std.contracts.pointwise_f64/0.1"

[signature]
quantity = "electrical.voltage"
schema = "preserve"
unit = "preserve"

[numeric]
input_domain = "finite_f64"
reference = "rectify.reference.json"
comparison = "exact_bits"

[resources]
state_bytes = 0
scratch_bytes = 0
```

매크로는 TOML의 필드·profile·자원 조건을 검사한다. reference 경로는 계약 파일의 디렉터리 기준이다. [reference JSON](../crates/cli/rectify.reference.json)은 `[입력 bits, 출력 bits]` 쌍의 배열이며 각 bit 값은 유한 f64의 16자리 hex 문자열이다. 예제의 8쌍은 음수·양수·±0·최소 subnormal·최대 유한 값의 양쪽 부호를 포함한다. 계약과 reference 파일은 각각 최대 1 MiB, reference는 1..4096쌍을 사용한다. 두 파일의 변경은 Cargo의 매크로 재생성 입력으로 추적한다.

## 번들과 BCi 호출

```rust
fn main() -> std::process::ExitCode {
    let extensions = bci_extension::Extensions::new(vec![
        rectify_registration(),
    ]).expect("유일한 이름과 유효한 pointwise 계약");
    bci_cli::main(&extensions)
}
```

위 번들 코드의 Cargo 의존성은 `bci-extension`과 `bci-cli`다. 각 연산자 구현은 Rust 함수·모듈로 구성하고, 조립 지점에서 registration 목록을 한 번 만든다. `Extensions::new`는 이름 충돌·표준 namespace 사용·profile·digest 구조를 검사한다. 번들당 등록 상한은 4096개다.

```bci
use lab.signal as lab_signal;
// run 본문에서 선언한 volts 포트에 연결한다.
let rectified = volts |> lab_signal.rectify();
```

컴파일러는 별칭·pipe·위치 인수와 `input:` 이름 인수를 해석한다. 모든 채널은 계약의 quantity를 만족해야 한다. 출력은 입력의 채널 순서·단위·reference·clock·발생률·위상·품질 타입을 그대로 유지한다. 같은 규칙을 사용자 graph 정의와 구체 run에 적용한다.

독립 호스트는 `bci_compiler::compile_with_extensions(source, base, extensions.specifications())`로 컴파일하고 `extensions.kernels(&standard_factory)`를 `bci_runtime::run`에 전달한다. compiler는 schema의 선언 데이터에 의존하고, 실제 함수 포인터는 extension SDK가 소유한다.

## 실행·품질·실패 경계

| 조건 | 실행 계약 |
|---|---|
| valid=1 | Rust 함수를 한 번 호출하고 유한 f64 출력을 요구 |
| valid=0 | 입력 payload와 mask를 그대로 복사, 함수 호출 수 0 |
| 시각·Gap·generation | sequence·time·source_range·origin·gap·flags·generation 보존 |
| 상태·메모리 | 사용자 state·scratch 0, 출력 슬롯과 고정 제어 구조를 기존 예산에 포함 |
| 계획과 번들의 계약·함수·reference 차이 | E0510, 원천 열기와 기록 준비 전에 종료 |
| reference 결과 차이 | E0510, 실제 입력 처리 전에 종료 |
| 비유한 함수 출력 | E0903, 해당 출력 게시 전 Fault |
| unwind panic | E0908과 BCi 호출 위치·Rust 위치, Fault → Draining → Closed |

pointwise 작성자는 입력 값만으로 출력이 결정되는 함수를 제공하며 state·scratch=0과 유한 입력에서의 종료·유한 출력을 계약한다. 일반 조건문·반복문과 고정 지역 값은 함수 본문에 작성한다. 장치·파일·clock·난수 관측은 효과와 source/sink 계약으로 연결하는 확장 설계를 따른다. `panic=unwind` 빌드는 호출 경계에서 Fault로 회수하고 `panic=abort` 빌드는 프로세스 종료 정책을 적용한다.

wrapper는 예약된 출력 슬롯에 값을 계산한 뒤 성공한 레코드를 게시한다. 호출 실패 시 앞서 완료한 레코드와 최초 실패를 종료 manifest에 보존한다. 입력 파일 EOF·표본 상한·stop 경계는 기존 런타임과 같다.

## 기록과 재생 정체성

각 pointwise 노드의 manifest에는 이름·버전·profile·측정량·계약 원문 SHA-256·Rust 함수 토큰 SHA-256·reference 원문 SHA-256·Rust symbol·소스 위치를 기록한다. 실행 파일 전체 SHA-256은 `executable_sha256` 필드에 저장한다. 함수 토큰 digest는 해당 함수 선언의 변경을, 실행 파일 digest는 링크된 커널·도움 함수·의존성을 포함한 바이너리를 식별한다. digest 계산은 입력 루프 전 prepare에서 수행한다.

원시 포트를 `source.replay`로 읽고 같은 확장 그래프를 실행하면 값·품질·시간·Gap 관계를 비교할 수 있다. 계산 결과 포트를 직접 재생하면 기록한 값과 타입을 복원한다. 데이터 형식은 `bci-recording/0.6`이며 실행 manifest는 정적 번들 정보와 커널 계약을 함께 보관한다.

기존 CLI 통합 검사 파일의 함수 하나가 reference 8쌍, CSV 22표본의 원시·확장 출력 44개와 재생 일치, signed zero·무효 payload·Gap 보존, 타입·효과·계약·reference·panic·비유한 출력 경계를 확인한다.

단일 입력·동일 격자의 상태 연산자 prepare/step/reset/checkpoint/restore API와 전체 순차 계획의 저장·복원을 구현했다. 패키지 manifest 기반 자동 bundle 생성, 독립 registry exporter와 bci.lock 연결은 [확장 설계](design/13-rust-extensions.md)의 후속 구현 범위다.
