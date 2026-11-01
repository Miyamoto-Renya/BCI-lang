# BCi 언어

BCi는 측정량, 채널, 시간, 품질과 학습 계보를 타입과 실행 계약으로 연결하는 BCI 전용 언어다. Rust 컴파일러와 런타임을 목표 구현 구조로 선택한다.

현재 저장소는 형식 설계 원문, 상세 설계 문서, 11개 패키지의 Rust workspace와 구조 검사 도구를 제공한다. CLI는 `.bci`의 구문·이름·타입·효과를 검사하고 처리 그래프를 설명한다. `bci run`은 합성 신호·측정 CSV·기록 포트를 같은 수치 처리 경로에 연결하고 JSON Lines와 실행 manifest를 기록한다. 언어·실행 기능의 전체 목표 계약은 `bci/0.1` 설계 문서에 정의한다.

- [상세 설계 문서와 결정 목록](docs/design/README.md)
- [프로젝트 구조와 유지보수 규칙](docs/architecture.md)
- [소스 검사·TypedGraph의 사용법과 구현 범위](docs/compiler.md)
- [실행 명령·수치 커널·기록 형식](docs/runtime.md)
- [측정 CSV·포트 재생·Gap·품질 정책](docs/inputs.md)
- [Rust 사용자 연산자·등록 매크로·정적 번들](docs/extensions.md)
- [상태 연산자·checkpoint 저장과 복원](docs/checkpoints.md)
- [동기 다중 입력·가중 합성·입력 묶음 복원](docs/multi-input.md)
- [채널 선택·재정렬·EEG 재기준화](docs/channels.md)
- [PSD·대역 전력·로그 특징](docs/spectral.md)
- [모델 학습·후보 선택·분할·평가·온라인 예측과 판정](docs/learning.md)
- [시행·세션·피험자별 평가와 재현 가능한 bootstrap 구간](docs/evaluation.md)
- [재표본화·다중 표본률 원천·큐 실행](docs/resampling.md)
- [SOS IIR 필터·안정성 검사·상태 복원](docs/iir.md)
- [표면 문법](docs/design/01-language.md)
- [일반 프로그래밍과 Rust 연산자 확장](docs/design/13-rust-extensions.md)
- [전체 사용 사례와 예상 판정](docs/design/12-examples.md)
- [원문: 설계 명세 1.0](bci_dsl_latex/main.tex)

설계 문서는 전체 언어의 계약을 기준으로 구성한다. 문서 번호는 주제 식별자이며, 각 구성 요소는 공동 적합성 조건을 공유한다.

BCi는 장치·시간·학습 계약과 그래프 구성을 표현한다. 일반 알고리즘의 조건문·반복문·재귀·자료구조는 Rust 확장으로 작성하고, 등록한 연산자를 BCi에서 호출한다.

Rust 소스는 `crates/<책임>/<파일>.rs` 깊이에 배치한다. 패키지의 의존 방향과 파일 깊이는 [architecture.toml](architecture.toml)과 자동 검사로 유지한다.

```sh
cargo run --offline --locked -- --help
cargo run --offline --locked -- check examples/typed.bci
cargo run --offline --locked -- explain examples/typed.bci --view types
cargo run --offline --locked -- run examples/typed.bci --samples 1000 --output recordings/demo
cargo run --offline --locked -- run examples/file.bci --output recordings/file
cargo run --offline --locked -- run examples/multi.bci --output recordings/multi
cargo run --offline --locked -- run examples/channels.bci --output recordings/channels
cargo run --offline --locked -- run examples/spectral.bci --output recordings/spectral
cargo run --offline --locked -- run examples/iir.bci --output recordings/iir
cargo run --offline --locked -- run examples/multirate.bci --samples 17 --output recordings/multirate
cargo run --offline --locked -- learn fit examples/learning.dataset.json --output recordings/learning-fit
cargo run --offline --locked -- run examples/learning.bci --output recordings/learning-run
cargo run --offline --locked -- run examples/selection.bci --output recordings/selection-run
cargo run --example bci-lab --offline --locked -- run examples/extension.bci --samples 1000 --output recordings/extension
python tools/check.py
```

검사에는 Rust 1.97 이상, rustfmt·clippy, Python 3.11 이상을 사용한다. 전체 검사 결과와 구현 범위는 [아키텍처 문서](docs/architecture.md)에 정의한 기준으로 보고한다.

## 라이선스

프로젝트 소스와 문서는 [Apache License 2.0](LICENSE)을 적용한다. 포함된 Noto Serif CJK KR 글꼴은 [SIL Open Font License 1.1](bci_dsl_latex/assets/OFL.txt)을 적용하며, 저작권·출처는 [FONT-NOTICE.txt](bci_dsl_latex/assets/FONT-NOTICE.txt)에 보존한다.
