# Workspace 구조와 유지보수 규칙

현재 workspace는 11개 Rust 패키지, 패키지 의존 정책, 구조 검사 도구와 CLI 진입점으로 구성한다. [현재 컴파일러](compiler.md)는 소스를 검사하고 TypedGraph·순차 실행 계획을 생성한다. [런타임](runtime.md)은 합성·CSV·재생 입력의 수치 처리·기록 경로를 실행한다. 설계 문서의 논리적 구성 요소를 이 문서의 물리적 패키지에 배치한다. 전체 동작 계약은 [상세 설계](design/README.md)를 따른다.

## 얕은 디렉터리 구조

```text
BCI-lang/
  Cargo.toml                 workspace, 공통 의존성·판본·lint
  Cargo.lock                 workspace 의존성 잠금
  architecture.toml          패키지별 책임과 허용 의존 방향
  crates/
    schema/                  Cargo.toml, lib.rs, 도메인별 .rs
    ir/                      Cargo.toml, lib.rs, 그래프·계획 .rs
    compiler/                Cargo.toml, lib.rs, 컴파일 단계별 .rs
    verify/                  Cargo.toml, lib.rs, 증명서 검사 .rs
    runtime/                 Cargo.toml, lib.rs, 실행 구성 요소별 .rs
    extension/               Cargo.toml, lib.rs, SDK .rs
    extension-macros/        Cargo.toml, lib.rs, 등록 코드 생성 .rs
    operators/               Cargo.toml, lib.rs, 연산자별 .rs
    devices/                 Cargo.toml, lib.rs, 장치 공통 계약 .rs
    artifact/                Cargo.toml, lib.rs, 형식별 codec .rs
    cli/                     Cargo.toml, main.rs, lib.rs, 명령·번들별 .rs
  tools/                     구조 검사·전체 검사·회귀 검사
  examples/                  CLI·통합 검사 공통 .bci 예제와 입력 기술자
  docs/
    architecture.md          물리 구조와 변경 규칙
    compiler.md              소스 검사 사용법과 구현 범위
    runtime.md               실행·자원·수치·기록의 구현 범위
    inputs.md                CSV·포트 재생·Gap·품질 정책
    extensions.md            Rust 등록 매크로·정적 번들
    checkpoints.md           상태 SDK·실행 저장·복원
    design/                  언어·실행 상세 설계
  .github/workflows/          CI 검사 진입점
  bci_dsl_latex/             원문 형식 설계
  target/                    Cargo 생성 산출물
```

Rust 소스 경로는 `crates/compiler/check.rs`처럼 세 경로 성분으로 유지한다. Cargo의 `[lib] path = "lib.rs"`와 `[[bin]] path = "main.rs"`로 진입점을 지정한다. crate 내부 파일은 같은 깊이에 두고 `mod check;`로 연결한다. 위 트리의 도메인별 `.rs`는 해당 기능을 작성할 때 생성할 배치 규칙이다.

통합 테스트와 benchmark도 패키지 루트의 `integration.rs`, `bench_window.rs`에 두고 Cargo target으로 명시한다. 패키지 전용 fixture는 소유 패키지의 `fixtures/`에 배치하고, CLI·문서·통합 검사에서 공유하는 예제는 루트 `examples/`에 둔다. 파일을 책임별로 나누고 `pub(crate)`로 내부 경계를 유지한다.

## 패키지 책임과 의존 방향

| 패키지 | 책임 | 허용하는 workspace 의존성 |
|---|---|---|
| bci-schema | 측정·시간·품질·효과·연산자 기술자 | 공통 값 타입의 기초 경계 |
| bci-ir | TypedGraph, BoundGraph, plan, certificate | schema |
| bci-artifact | codec, manifest, 기록·복구 | schema, ir |
| bci-devices | profile, binding predicate, DeviceAdapter | schema |
| bci-compiler | source, syntax, resolve, check, bind, plan | schema, ir, artifact, devices |
| bci-verify | 계획·증명서 독립 검사 | schema, ir |
| bci-extension-macros | Rust 등록 속성과 wrapper 생성 | proc-macro 컴파일 경계 |
| bci-extension | 연산자 SDK, context, codec 인터페이스 | schema, extension-macros |
| bci-operators | 표준 수치 커널과 참조 구현 | schema, extension |
| bci-runtime | actor, 일정, pool, supervisor | schema, ir, verify, extension, devices |
| bci-cli | 명령 처리와 실제 구현체 조립 | schema, compiler, runtime, artifact, devices, operators, extension |

기계가 검사하는 정책의 원본은 [architecture.toml](../architecture.toml)이다. Cargo manifest는 실제 의존 간선, 정책의 `allows`는 허용 간선을 나타낸다. 일반·개발·빌드·target별 의존성에 같은 방향 규칙을 적용한다. 별칭 dependency도 실제 package와 path로 정규화해 검사한다.

의존성은 소비자가 더 낮은 계약 계층을 참조하는 방향으로 흐른다. verify는 schema·ir를 이용해 compiler의 계획을 독립 검사한다. compiler는 장치 공통 descriptor를 받아 bind하고, runtime은 장치·연산자 인터페이스를 통해 실행한다. 실제 제조사 SDK와 표준 커널의 선택은 CLI 또는 전용 bundle의 조립 코드가 담당한다.

schema는 공유 도메인 계약, ir는 그래프·계획 데이터 구조를 소유한다. 둘의 구분을 유지해 schema에 컴파일 단계별 상태가 누적되는 것을 관리한다. 공통 utility가 필요하면 사용 도메인의 파일에 먼저 배치하고, 여러 패키지가 공유하는 의미 계약이 생겼을 때 소유 위치를 결정한다.

## 논리적 설계와 물리적 패키지의 대응

| 설계 08의 논리적 이름 | 물리적 위치 | 배치 이유 |
|---|---|---|
| bci-source | compiler의 source·syntax 파일 | 소스 처리와 타입 검사가 함께 변경되는 경계 |
| bci-check | compiler의 check·effect 파일 | 하나의 HIR·진단 API 공유 |
| bci-plan | compiler의 plan·schedule 파일 | 구체화한 그래프에서 계획 생성 |
| bci-profile | devices의 profile·binding 파일 | compiler와 runtime이 공유하는 장치 계약 |
| bci-schema, bci-ir | 각 독립 패키지 | compiler·runtime·검증기가 공유하는 데이터 |
| bci-verify | verify 독립 패키지 | 계획 생성 구현과 분리한 검증 책임 |
| bci-extension, bci-extension-macros | 각각 독립 패키지 | 공개 SDK와 proc-macro의 컴파일 경계 |
| bci-operators, bci-runtime, bci-artifact | 각 독립 패키지 | 수치·실행·저장 책임과 변경 범위 |
| bci-cli | cli 독립 패키지 | 사용자 진입점과 실제 구현체 조립 |
| bci-adapter-이름 | 추가 시 crates/adapter-이름 | 제조사 SDK·OS 조건의 독립 관리 |
| bci-lsp | 추가 시 crates/lsp | compiler 공개 API를 사용하는 별도 진입점 |

compiler 내부 단계는 파일 단위로 분리하고, 외부 API는 lib.rs에서 의도한 항목만 노출한다. 패키지 추출의 기준은 독립 배포·외부 의존성·플랫폼 조건·다른 소비자와의 계약 공유다. 파일 개수와 설계 문서 번호는 패키지 수를 결정하는 기준에서 분리한다.

## 확장 추가 위치

### 장치

새 장치 adapter는 `crates/adapter-cyton/` 같은 형제 패키지에 둔다. 공통 API는 bci-devices를 참조하고, 제조사 SDK·FFI·codec·플랫폼 설정은 adapter 패키지가 소유한다. 기본 runtime은 DeviceAdapter와 검증한 profile을 받고, 조립 계층이 선택한 구현체를 전달한다.

새 adapter를 연결할 때 workspace member·workspace dependency·architecture policy·조립 패키지의 dependency를 함께 변경한다. OS별 의존성도 루트 workspace.dependencies에서 판본을 고정하고 leaf 패키지에서 target 조건을 선언한다. native library가 필요한 검증은 해당 플랫폼 CI job에 추가한다.

### 알고리즘과 Rust SDK

기본 제공 연산자는 operators의 `linear.rs`, `window.rs`, `matrix.rs`에 둔다. 독립 배포·추가 native 의존성이 필요한 연산자는 `crates/operator-이름/`으로 분리한다. 현재 [Rust 확장 SDK](extensions.md)는 pointwise·stateful 등록 매크로와 정적 registry를 제공한다. schema/extension.rs가 선언을 소유하고, compiler·verify는 이 값 타입을 검사한다. extension의 pointwise.rs·stateful.rs·codec.rs는 registry·함수·상태 wrapper·codec을 소유한다. CLI의 lab.rs·lab_signal.rs·lab_state.rs는 Cargo example 번들의 조립과 사용자 함수 예제다.

사용자가 만드는 외부 실험 프로젝트의 `extensions/` 구조는 [사용자 프로젝트 규격](design/10-artifacts-tooling.md)을 따른다. BCi 자체 저장소에서 관리하는 구현 패키지는 `crates/`의 같은 깊이에 모은다.

### 저장 형식과 도구

기록 codec은 artifact에서 관리하고, runtime의 기록 sink·재생 source에는 조립 코드가 전달한다. 언어 서버는 compiler의 동일 API를 사용한다. 도구별 출력 형식과 UI 상태는 해당 도구 패키지가 소유한다.

## 현재 생성한 실행 범위

| 구성 요소 | 현재 코드·도구 | 구현을 연결할 설계 |
|---|---|---|
| Cargo workspace | 11개 패키지, 로컬 dependency, 공통 lint·판본 | 각 책임의 구현 파일을 동일 패키지에 추가 |
| schema·ir | 정확 산술·스트림·효과·커널·Frame 계약, Probability·Decision·전체 후보와 Tune 선택 계보·평가 protocol, TypedGraph·순차 ExecutionPlan | 전체 도메인 계약·일반 실행 계획 |
| compiler·artifact | 소스 검사·계획 생성·CSV·기록 reader, JSON Lines·모델·평가·실행 manifest | 전체 operator·package·추가 파일 codec |
| verify·runtime | 순차 계획 검사·동기·시각 병합 원천·고정 큐·다중 출력 실행·Gap·품질·EOF·checkpoint·종료·오류 | 이벤트·순환 그래프 일정·장치·병렬 일정 |
| extension·extension-macros·operators | pointwise·stateful registry·매크로·reference·codec, Fit 정규화·softmax·Tune 선택·독립 Eval·계층 집계·cluster bootstrap·예측·판정·재표본화·IIR·채널 변환·재기준화·주파수 특징·표준 커널 주입 | 다중 포트 확장·자동 bundle 생성·추가 수치 커널 |
| CLI | 공통 CLI library, check·explain types·run·learn fit/select/evaluate·extensions, bci-lab 번들 | 상세 설계 10의 전체 명령 |
| devices | 공통 InputSource 경계와 Rustdoc 책임 | 장치 adapter·binding |
| 구조 검사 | 의존 방향·순환·상속·명시적 target·Rust 파일 깊이 검사 | 새 패키지·의존성 변경에 반복 적용 |
| 검사 진입점 | 구조 회귀, fmt, clippy, cargo test/doc, CLI 확인 | 구현별 수치·장치·종단 시험 추가 |

이 표의 실행 범위와 언어 적합성 판정은 각자의 결과를 보고한다. [적합성 행렬](design/11-conformance.md)은 향후 구현 기능의 입력·oracle·수용 조건을 정의한다.

## 개발과 검사

개발 환경은 Rust 1.97 이상, rustfmt·clippy, Python 3.11 이상을 사용한다. CI는 Rust 1.97.1을 선택한다. Python 도구는 표준 라이브러리를 사용한다. 의존성 잠금은 루트 Cargo.lock을 함께 관리한다.

```sh
cargo run --offline --locked -- --help
cargo run --offline --locked -- --version
cargo run --offline --locked -- check examples/typed.bci
python tools/architecture.py
python tools/check.py
```

전체 검사는 현재 캐시에 있는 의존성으로 `--offline --locked`를 적용한다. 외부 Rust 의존성을 추가하는 변경은 `cargo fetch --locked`로 준비한 뒤 동일 검사를 수행한다. Cargo.lock 갱신은 의존성 변경과 같은 변경 묶음에 포함한다. CI와 로컬은 tools/check.py를 공통 진입점으로 사용한다.

공개 API는 rustdoc을 작성하고, lib.rs의 export가 패키지 경계를 드러내도록 유지한다. 내부 파일은 `pub(crate)`를 기본으로 사용한다. 전체 검사에서 Clippy와 rustdoc 경고를 오류로 처리한다. 수치 코드의 release build도 정수 overflow 검사를 유지한다.

workspace의 unsafe_code lint는 deny다. native FFI를 구현하는 leaf adapter는 필요한 boundary 모듈에 범위를 한정한 허용 속성과 포인터·수명·aliasing 불변식을 함께 작성한다. 해당 boundary의 정상·오류·종료 경로를 검증한다.

## 구조 변경 규칙

1. 패키지의 공개 계약과 직접 소비자를 먼저 기록한다.
2. 같은 책임의 기능은 기존 패키지의 형제 파일로 추가한다.
3. 독립 계약이 필요한 기능은 crates의 새 형제 패키지로 배치한다.
4. Cargo와 architecture.toml에 실제 의존성·허용 방향을 함께 기록한다.
5. 새로운 구조 예외와 기능 변경은 대응 문서와 검사 근거를 함께 갱신한다.

원문 LaTeX는 설계 근거, docs/design은 의미 계약, 이 문서는 물리 구조, architecture.toml은 검사 정책을 소유한다. 각 정보의 원본을 이 네 위치로 구분해 중복 수정을 줄인다.

동기 다중 입력은 runtime/ingress.rs의 묶음 준비와 operators/mix.rs의 가중 합성으로 연결한다. 포트 접근은 extension의 Inputs view를 공유한다. 원천별 시각·Gap·EOF, 계보와 checkpoint 계약은 [다중 입력 안내](multi-input.md)에 정의한다.

채널 공간 변환은 schema/spatial.rs의 공통 계약과 operators/spatial.rs의 수치 커널로 구성한다. 기준 ID 계산에는 기존 workspace의 sha2를 사용한다. compiler는 채널 이름을 index로 해석하고 verify는 계획의 index·계수와 출력 스키마를 대조한다. 사용법과 자원 계약은 [채널 변환 안내](channels.md)에 정의한다.

주파수 특징은 schema/spectral.rs의 타입·대역 계약과 operators/spectral.rs의 수치 커널로 구성한다. Spectrum의 주파수 축은 artifact codec에서 기록·복원한다. 실행·자원·검사 근거는 [PSD 안내](spectral.md)에 정의한다.

고정 SOS IIR는 schema/iir.rs의 안정성·자원 계약과 operators/iir.rs의 채널 상태·codec으로 구성한다. artifact/descriptor.rs가 파일 정규화를 담당한다. 사용법은 [IIR 안내](iir.md)를 따른다.
