# 적합성, 참조 실행과 수용 조건

근거: [원문 적합성 관계](../../bci_dsl_latex/sections/10_conformance.tex). 이 문서는 향후 구현의 검사 입력과 기대 결과를 정의한다. 아래 수용 항목의 실행 결과는 구현별 보고서에 기록한다.

## 완료 판정 단위

언어의 완료 판정은 구문·타입·수치·시간·학습·장치·실행·재생 계약의 종단 연결을 대상으로 한다. 각 구현 작업은 소유 계약, 입력 fixture, 관측 oracle, 의존 계약, 통과 증거를 제출한다. 문서 번호와 작업 난이도는 완료 판정의 순서를 정하는 요소에서 분리한다.

원문의 관측 적합성은 `Obs(trace) ∈ allowed_traces(program,input,environment,numeric_tolerance)`다. 정수·사건 순서·모델 ID·세대는 exact, float는 선언한 오차 관계, 측정 시각은 구간 포함 관계로 검사한다. hard deadline은 Bound 가정과 실제 적용 환경을 함께 검사한다.

## 참조 실행기

reference executor는 단일 thread, 고정 사건 순서, 논리 시계, reference kernel을 사용한다. source는 fixture로 주고 Timer·mapping·activation도 입력 trace에 포함한다. 구현 runtime과 동일한 typed graph를 받아 값·품질·계보·상태 전이를 비교한다.

고정된 input trace와 environment에서 prefix를 잘라 반복 실행하여 `cut_u(F(S)) = cut_u(F(cut_u(S)))`를 검사한다. timer와 label을 포함한 의존 사건도 같은 cut에 적용한다. prefix 실행의 종료는 전체 입력 EndStream과 구분하는 `PrefixBoundary` 시험 제어로 표현한다. 이 제어는 창을 강제로 완결하는 source 종료 효과를 갖지 않는다.

## 검사 행렬

| ID | 대상 | 입력 fixture | 수용 조건 |
|---|---|---|---|
| C01 | parser/formatter | 모든 문법 생성자, Unicode 문자열, malformed token | AST 의미의 format round trip, 오류 span |
| C02 | module/kind | import cycle, 중복 이름, 타입 인수 kind | 지정 코드·관련 선언 위치 |
| C03 | 단위·schema | V/uV, 채널 permutation, reference 변경 | 명시 변환 후 일치, 불일치 포트 진단 |
| C04 | 효과·피드백 | Record 경로, zero-delay cycle, 초기값 | mode 규칙, G0 DAG, 초기값 타입 |
| C05 | 가용 인과성 | timer·label·activation을 포함한 prefix | 절단 법칙과 dependency availability |
| C06 | 수치 연산 | FIR impulse, PSD sinusoid, SPD 경계 | [05](05-operators.md) 수식과 허용오차 |
| C07 | clock mapping | drift, wrap, 유효 구간 경계, composition | 전체 구간 포함과 evidence kind 유지 |
| C08 | watermark | ingress·queue·join 재정렬, silence, late input | pending 반영 전 완결 차단, 단조 frontier |
| C09 | 품질·epoch | gap, saturation, recovery, schema 전이 | 마스크·reset·warmup·세대 순서 |
| C10 | 학습 계보 | overlap window, global stats, nested folds | role·group·ancestor 관계 |
| C11 | 모델 활성화 | 예측 중 모델 변경, delayed label | 시작 버전 유지와 시간 부등식 |
| C12 | decoder | profile별 byte/value, partial frame, counter wrap | exact 값·부호·순서·scale |
| C13 | binding | invalid config, access scope, Pending evidence | 3값 판정과 실패 근거 |
| C14 | SDF 계획 | rational rates, cycle tokens, 병렬 예약 | prefix token·capacity·주기 평형 |
| C15 | 자원 | burst, service pause, 느린 consumer | 선언 capacity·memory와 overflow policy |
| C16 | lifecycle/FFI | callback 중 stop, reconnect, SDK 오류 | 단일 소유, 완료 barrier, 새 epoch |
| C17 | 기록·재생 | truncated chunk, hash 변조, activation trace | commit prefix 복구와 관측 대응 |
| C18 | 외부 명령 | receipt 유실, duplicate request, crash | command ID, OutcomeUnknown, policy |
| C19 | 최적화 | fusion·공통화 전후 같은 graph | 타입·계보·품질·수치 관측 관계 |
| C20 | 종단 실행 | [12](12-examples.md)의 다섯 사용 사례 | 소스→plan→trace→replay의 전체 계약 |
| C21 | Rust 확장 등록 | [13](13-rust-extensions.md)의 함수·상태형 커널, 계약 profile | wrapper·registry·dtype·quantity·효과의 대응 |
| C22 | 구성·실행 분담 | 진입점별 구성, select 양쪽 상태, Hold | phase 진단, 상태 진행·계보·장치 호출 수 |
| C23 | 확장 배포·실패 | bundle 불일치, 출력 예산 초과, panic, 복원 | digest 검사·Fault 전이·checkpoint 계약 |

## 수치 oracle

- decoder는 byte열과 예상 integer·channel 배열을 짝으로 둔다. int24 부호 경계, endian, 길이·상태 필드를 포함한다.
- FIR은 impulse response, 상수 입력, block 분할 변화, gap reset을 비교한다. 같은 sample stream을 1/7/32개 block으로 공급해 값·상태·support의 대응을 확인한다.
- PSD는 DC, 짝수 N의 Nyquist, 홀수 N, 단일 bin 정현파를 사용한다. taper 에너지 정규화와 one-sided 배율을 검사한다.
- covariance는 상수 입력·rank 부족·재기준화 영공간을 포함한다. regularize 후 최소 고유값과 Cholesky 결과를 검증한다.
- tangent는 R=Sigma에서 영벡터를 기대하고, svec의 Frobenius norm 보존을 검사한다.
- optical은 양의 intensity·baseline, rank 부족 path model, 단위 스케일 변화를 사용한다.

각 tolerance는 dtype·입력 크기·입력 영역·norm에 연결한다. backend 간 차이는 해당 numeric contract로 판정한다. deterministic kernel 모드는 backend·thread count·RNG를 고정한 반복 실행의 payload digest를 비교한다.

## 시간과 학습의 경계 입력

watermark corpus는 `[u,v)`의 u,v와 정확히 같은 timestamp, 경계를 가로지르는 uncertainty interval, multi-source pending, mapping 만료, clock restart를 포함한다. 늦게 복구한 표본은 원래 measurement와 새 availability를 유지해야 한다.

학습 corpus는 같은 raw range에서 유래한 서로 다른 파일·창, 전처리 통계의 Eval 참조, Tune으로 선택한 threshold, 예측 후 도착한 label을 포함한다. 오류 보고서는 충돌 조상까지의 graph path를 제공해야 한다.

## 실행 자원과 실패 주입

resource 시험은 계량 boundary·payload dtype·block size를 고정한다. queue overflow, recorder stall, pool lease 장기 보유, actor timeout, connection loss를 주입한다. 기대 출력은 Fault/Gap/RejectedWindow와 상태 전이 trace다.

실행 계획의 증명서는 별도 verifier가 독립 재계산한다. scheduler가 생성한 반복벡터와 capacity 값을 그대로 신뢰하는 경로는 verifier의 입력 검증에서 재계산 대상으로 둔다. 병렬 실행은 허용 thread 수·의존 관계에서 가능한 start/complete 순서를 검사한다.

FFI 시험은 callback lifetime, repeated start/stop, 부분 초기화 후 close, error path의 buffer 반환을 포함한다. native integration fixture는 실제 profile instance·SDK·OS/arch를 보고서에 고정한다. protocol fixture 결과와 장치 integration 결과는 각 evidence kind로 기록한다.

## 보고서 형식

```text
ConformanceReport {
  suite_version, implementation_id, commit_or_source_digest,
  language_version, operator_registry_digest,
  profile_instances, environment, seed,
  cases: [{ id, fixture_digest, status, expected, observed,
            numeric_error, trace_refs, diagnostics, evidence }],
  counts: { passed, failed, blocked, total },
  required_contracts, unresolved_obligations
}
```

status는 Pass, Fail, Blocked다. Blocked는 필요한 장치·artifact·증거가 누락된 조건과 해결 입력을 기록한다. 전체 합계는 `passed+failed+blocked=total`을 만족한다. 구현의 적합 판정은 required_contracts에 속한 모든 case가 Pass인 경우에 부여한다.

## 구현 작업의 공동 계약

| 작업 경계 | 제공할 결과 | 함께 확인할 계약 |
|---|---|---|
| source/check | syntax tree, typed graph, 진단 | schema·operator registry |
| operator/kernel | 전이 구현과 reference corpus | dtype·quality·support·lineage |
| extension SDK/macros | Rust wrapper·registry·runtime bundle | signature·계약 병합·자원·오류·배포 digest |
| profile/adapter | descriptor와 ingress trace | binding·clock·epoch·소유권 |
| plan/runtime | verified plan과 실행 trace | SDF·event frontier·buffer·fault |
| learning/artifact | split·model·activation | fit lineage·availability·schema |
| recorder/tooling | manifest·replay·설명 도구 | content ID·관측 대응·진단 |

각 작업은 정상 입력, 경계 입력, 실패 입력, 종단 사례를 함께 제출한다. 구성 요소의 교체는 같은 계약과 corpus를 공유하는 방식으로 검토한다.
