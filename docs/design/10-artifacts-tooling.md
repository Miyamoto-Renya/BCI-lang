# 패키지, artifact, 기록과 개발 도구

근거: [원문 manifest와 재생](../../bci_dsl_latex/sections/10_conformance.tex). 아래 디렉터리와 명령은 목표 구현의 파일·도구 계약이다.

현재 CLI는 `bci learn fit DATASET --output DIRECTORY`, `bci learn select DATASET --candidate MODEL --candidate MODEL --output DIRECTORY`, `bci learn evaluate MODEL DATASET --output FILE`로 [학습·평가 artifact](../learning.md)를 생성한다. 각 명령의 `--protocol FILE`은 최종 Eval에 [계층 집계·재표집](../evaluation.md)을 적용한다. 현재 CLI의 직접 파일 명령은 `bci check 파일.bci`, `bci explain 파일.bci --view types`, `bci run 파일.bci`다. [소스 검사](../compiler.md)와 [실행](../runtime.md)은 입력 소스의 디렉터리에서 artifact·기록 상대 경로를 해석한다. [CSV·포트 재생](../inputs.md)도 같은 경로 규칙을 사용한다. `run --output`은 현재 작업 디렉터리 기준 출력 경로를 지정한다. 아래 package 명령의 기준 경로는 package root다.

## 사용자 프로젝트

```text
experiment/
  bci.toml
  bci.lock
  src/main.bci
  schemas/
  profiles/
  filters/
  models/
  protocols/
  evidence/
  extensions/
  registry/
  target/bci/
  recordings/
```

```toml
[package]
name = "motor-feedback"
version = "0.1.0"
language = "bci/0.1"
entry = "demo.motor.Main"

[dependencies]
std = "0.1.0"

[registry]
operators = "registry/operators.json"
profiles = "profiles/catalog.json"

[execution]
resource_profile = "profiles/workstation.resources.json"
record_directory = "recordings"
```

경로는 package root 상대 경로다. artifact resolver는 정규화 경로와 digest를 잠금 파일에 기록한다. `bci.lock`은 package·표준 라이브러리·operator·kernel·profile·model·coefficient·schema의 버전과 SHA-256을 가진다. locked build는 파일 내용의 digest를 확인한다.

## 명령 인터페이스

| 명령 | 입력과 출력 |
|---|---|
| `bci fmt [path] --check` | 구문 tree 기반 서식 검사 또는 정규화 |
| `bci check [entry] --format json` | 타입·효과·정적 계약과 Pending 의무 |
| `bci lock` | 현재 의존성과 artifact의 고정 digest 생성 |
| `bci extension build NAME --locked` | [13](13-rust-extensions.md)의 Rust 확장·registry exporter·정적 runtime bundle 빌드 |
| `bci build [entry] --locked` | typed graph와 plan template, manifest 생성 |
| `bci probe --profile PATH --output PATH` | 장치 descriptor와 조회 evidence 저장 |
| `bci bind PLAN --descriptor PATH --evidence DIR` | template을 구체화하여 bound graph·실행 계획·의무 판정 생성 |
| `bci verify PLAN` | 증명서·참조 digest·타입·자원 관계 독립 검사 |
| `bci run PLAN --record DIR` | prepare·실행·종료와 trace 기록 |
| `bci replay DIR --mode availability` | 기록된 사건 순서와 논리 availability 재생 |
| `bci replay DIR --mode retrospective --entry ENTRY` | 새 Record 그래프·time view 생성 |
| `bci explain PLAN --view types` | types/binding/schedule/memory/time/lineage 뷰 |
| `bci conformance SUITE --report PATH` | [11](11-conformance.md)의 판정 보고서 |

종료 코드는 0=명령 성공, 2=구문·정적 계약 오류, 3=필수 binding 증거 대기, 4=실행 fault, 5=artifact I/O·무결성 실패다. `check`는 정적 성공과 함께 Pending 목록을 출력할 수 있다. `verify`의 실행 가능 판정과 `run`은 필수 의무의 Pass를 요구한다. `build`는 Pending을 담은 template 생성에 성공할 수 있으며 산출물 kind를 `PlanTemplate`로 표시한다.

CLI JSON은 `command, status, diagnostics, obligations, artifacts, measurements`를 가진다. stderr는 진행·진단, stdout은 요청한 구조화 산출물로 사용한다. 실행 중 stop은 종료 요청 사건으로 전달한다.

확장은 `[extensions.NAME]`의 Cargo manifest·registry 경로·namespace로 연결한다. extension build는 고정 Cargo.lock·toolchain 설정으로 Rust 빌드를 수행하고, `bci.lock`은 extension API·계약·reference·kernel·runtime bundle digest를 고정한다. `bci check`는 생성된 registry descriptor를 사용한다. runtime은 선택한 bundle의 kernel 집합과 plan의 참조를 대조한다.

## manifest 스키마

```text
Manifest {
  schema_version, language_version, compiler_id, manifest_id,
  source: { files, normalized_ast_digest, source_map_digest },
  graph: { typed_graph_digest, bound_graph_digest, node_map },
  registry: { operators, kernels, numeric_contracts, extensions, runtime_bundle },
  hardware: { profile_instances, configurations, access_evidence },
  types: { schemas, units, clock_domains, occurrence_rules },
  timing: { mappings, evidence, lateness, deadlines, anchors },
  learning: { splits, models, fit_lineage_roots },
  execution: { actors, schedules, buffers, pools, limits, policies },
  verification: { obligations, certificates, verifier_id },
  provenance: { roots, artifact_dependencies },
  environment: { os, arch, cpu_allocation, sdk_versions },
  recording: { run_id, trace_schema, chunk_policy }
}
```

manifest는 content-addressed immutable artifact다. 실행 중 새 mapping·모델·설정은 append-only event와 새 artifact를 생성한다. session catalog가 최초 manifest와 이후 변경 artifact를 연결한다.

## 정규화와 hash

content ID는 `sha256(domain_tag || 0x00 || canonical_bytes)`다. domain tag에는 artifact kind와 schema version을 넣는다. JSON metadata의 key는 UTF-8 byte 순으로 정렬하고 string은 Unicode NFC로 정규화한다. array 순서는 의미 순서를 유지한다.

정수는 canonical decimal, rational은 기약분수 `[numerator,denominator]`이며 denominator>0이다. float payload는 dtype와 little-endian IEEE bit pattern을 보존한다. NaN payload와 signed zero도 payload digest에 포함한다. JSON의 float 매개변수는 `{dtype,bits_hex}`로 인코딩하여 문자열 표현 차이를 제거한다.

origin metadata와 artifact의 display label은 별도 영역이다. 의미에 영향을 주는 필드와 표시 필드의 포함 범위는 schema마다 정의한다. manifest_id 자체는 hash 입력에서 제외하고 나머지 immutable semantic 필드를 해시한다.

원천 RecordID는 `(source_id, acquisition_epoch, sequence, within_block_index)`다. 같은 값의 두 물리 측정은 서로 다른 RecordID를 갖는다. provenance node는 입력 ID·원천 범위·operator·parameter·선택 사건·output selector를 정규화해 hash한다.

## 기록 컨테이너

BCi 기록 디렉터리는 manifest, artifact store, trace chunk, commit log, index를 포함한다. metadata는 canonical JSON, 대형 수치 배열은 dtype·shape·stride를 가진 binary blob으로 저장한다.

```text
recording/
  session.json
  artifacts/sha256/<digest>
  chunks/00000000.bcit
  commits.jsonl
  index.json
```

`.bcit` chunk header는 magic `BCIT`, version u16, flags u16, header_len u32, payload_len u64를 little-endian으로 인코딩한다. 뒤에는 header_len byte canonical JSON과 payload_len byte의 framed records가 따른다. 각 frame은 kind u16, metadata_len u32, body_len u64, metadata JSON, body bytes 순서다. parser는 길이를 파일 크기·최대 record size와 비교한 뒤 allocation한다.

chunk JSON에는 run ID, 최초·최종 global sequence, 원천 sequence 범위, record count, codec ID가 들어간다. 기본 codec은 raw다. chunk hash는 완성 파일 전체에서 계산해 commit entry에 저장한다. commit entry는 이전 commit hash와 현재 chunk hash를 연결한다.

기록기는 임시 chunk 작성, flush·fsync, atomic rename, 상위 directory fsync, commit append·fsync, index 갱신 순서를 따른다. 새 commit 파일과 index 파일도 상위 directory의 지속성 조건을 적용한다. 복구는 유효한 commit chain의 마지막 chunk까지 읽고 hash·length·record count를 검증한다. 완성된 orphan chunk는 recovery catalog로 수집하고 명시적 복구 transaction으로 연결한다.

## 사건과 재생 의미

trace에는 Data, Gap, Watermark, Timer, ClockMapping, ActivateModel, Configure, EndEpoch, EndStream, CommandDispatch, CommandReceipt, Fault, Checkpoint가 들어간다. 각 record는 원래 measurement·availability·clock epoch와 source sequence를 유지한다. 수락 global sequence는 실제 제어 사건 선형화를 복원한다.

availability replay는 원래 availability와 global sequence를 논리 시계로 사용하고 재생 호스트 실행 시각을 `replay_wall_time`에 따로 저장한다. 빠른 재생은 논리 시간을 건너뛰고, 실시간 재생은 playback speed에 맞춰 대기한다. 두 실행의 값·모델 선택·논리 관측 비교 기준은 동일하다.

retrospective replay는 새 mapping·필터·모델을 사용한 Record 그래프를 실행하고 새 run ID, time view와 lineage를 생성한다. XDF/SNIRF/Open Ephys/SpikeGLX import는 해당 판본의 timestamp·scale·원천 범위를 공통 schema로 변환한다.

재생의 외부 출력은 기본 `Sink.Simulated`로 dispatch intent와 예상 receipt를 기록한다. 실제 장치 출력 연결은 별도의 live binding과 output capability·명령 정책을 요구한다.

## LSP와 설명 도구

LSP는 문법 강조, go-to-definition, 타입 hover, operator 서명, 단위 오류 수정 제안, obligation 탐색을 제공한다. hover는 inferred stream type과 schema·clock·effect를 보여준다. 진단 JSON을 CLI와 공유한다.

explain은 node→source, output→lineage, obligation→evidence 경로를 반환한다. memory 뷰는 queue·pool·actor state·metadata 별 예약량과 측정 peak를 구분한다. time 뷰는 신호 지연·창 span·availability 지연·command edge를 각 필드로 표시한다.
