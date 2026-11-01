# 종단 프로그램과 예상 판정

이 문서는 `bci/0.1`의 목표 사용 형태를 완전한 진입점 소스와 입력 artifact 계약으로 설명한다. 수치 기대값은 [원문 사례](../../bci_dsl_latex/sections/09_cases.tex)의 식과 아래 매개변수에서 계산한다. 예제의 파일 경로는 사용자 프로젝트 안에 배치할 fixture의 이름이다.

## 사례 A: EEG에서 실시간 선택 사건까지

8개 전압 채널·250 Hz 입력을 V로 변환하고 63-tap FIR, 250표본 창, SPD 공분산, 36차원 접공간 특징, 확률 추정과 선택 사건으로 연결한다.

```bci
language "bci/0.1";
module demo.motor;

use std.device as device;
use std.source as source;
use std.schema as schema;
use std.signal as signal;
use std.time as time;
use std.model as model;
use std.decision as decision;
use std.recording as recording;
use std.runtime as runtime;

schema EEG = schema.load("schemas/eeg8-uv.schema.json");
clock Session = time.session_clock();
device Amp = device.bind(
    profile: "profiles/a01-cyton.profile.json",
    config: #{ channels: 8, output_rate: 250[Hz] }
);
const ClockMap = time.mapping("evidence/amp-to-session.clock.json");
const Filter = signal.coefficients("filters/bandpass63.coeff");
model Reference = model.load("models/tangent-reference.model");
model Classifier = model.load("models/motor-classifier.model");
resource Budget = runtime.budget(
    memory: 256[MiB], queue_policy: Queue.Fault,
    profile: "profiles/workstation.resources.json"
);

run Main {
    runtime.configure(mode: Mode.Live, resources: Budget);
    let raw = source.acquire(Amp, expect: EEG, rate: 250[Hz]);
    let aligned = raw |> time.align(to: Session, mapping: ClockMap);
    let volts = aligned |> signal.convert_unit(to: "V");
    let filtered = volts |> signal.fir(coeffs: Filter, warmup: Warmup.Full);
    let epochs = filtered |> signal.window(
        size: 250, hop: 25, start: WindowStart.FirstValid,
        anchor: Anchor.End, quality: Quality.RequireAll
    );
    let covariance = epochs
        |> signal.covariance()
        |> signal.regularize(shrinkage: 0.1, floor: 1e-12[V^2]);
    let features = covariance |> signal.tangent(reference: Reference);
    let prediction = features |> model.predict(model: Classifier);
    let choice = prediction |> decision.threshold(minimum: 0.8);
    recording.write((raw, prediction, choice), to: "recordings/motor");
}
```

### 입력 artifact와 준비 조건

| artifact | 필수 내용 |
|---|---|
| eeg8-uv schema | 8개 순서 있는 전극 ID, 공통 기준, uV, 장치 channel map |
| A01 profile | 실제 모델·firmware·route·SDK 또는 decoder 버전, 8채널·250 Hz 설정 증거 |
| ClockMap | Amp clock epoch→Session origin, 유효 구간, Bound 또는 운영 모드가 허용한 Estimate, 가용 시각 |
| Filter | 유한 대칭 63계수, 선택한 통과·저지 대역, 무차원, digest |
| Reference | 변환 후 V 채널 schema의 SPD(8) R, V² 단위, Fit lineage |
| Classifier | 동일 reference digest를 사용하는 36개 feature, class 순서, Probability 출력, Fit lineage |
| Budget profile | queue·pool·actor·recorder 예산과 kernel evidence |

Classifier와 Reference는 prepare에서 검증한 immutable 모델을 active 초기 버전으로 등록한다. `source.acquire`는 native clock stream을 반환하고 `time.align`이 시간 뷰를 생성한다. ClockMap이 실제 epoch와 일치하는지 prepare가 검사한다.

### 타입과 시간

| 출력 | 값·단위 | 명목 발생 규칙 |
|---|---|---|
| raw | Vec<f64,8>, uV | 250 Hz, native clock |
| volts/filtered | Vec<f64,8>, V | 250 Hz, Session view |
| epochs | Epoch<f64,8,250>, V | 10 Hz, 마지막 표본 anchor |
| covariance | SPD<f64,8>, V² | 10 Hz |
| features | Vec<f64,36>, 무차원 | 10 Hz |
| prediction | Probability<MotorClasses> | 10 Hz |
| choice | Event<SelectOrHold> | tick마다 한 결정, 입력 epoch ID 참조 |

입력 index 0의 시각을 0, gap이 없는 연속 입력으로 둔다. FIR n=0…61은 warmup, n=62부터 valid다. 첫 완전 유효 창은 filtered index 62…311이며 마지막 시각은 311/250=1.244 s다. 두 번째 창은 87…336이며 끝은 1.344 s다.

창 표본의 첫·끝 시각 차이는 249/250=0.996 s, 갱신 간격은 25/250=0.1 s, FIR 군지연은 31/250=0.124 s다. 첫 창의 **원천 의존 범위**는 필터 support를 포함한 raw index 0…311이다. 창의 표본 span과 전체 원천 support를 각각 기록한다.

끝점 이후 ingress 4 ms, queue 2 ms, kernel 3 ms, 전달 1 ms라는 Bound 조건을 적용하면 첫 출력 가용 상한은 1.254 s다. 10 ms는 이 네 조건의 합이며 profile의 실행 증거가 각 값을 뒷받침해야 한다. 실제 실행은 각 관측 경계의 측정값을 trace에 기록한다.

### 예상 오류

| 변경 | 예상 판정 |
|---|---|
| Classifier가 35차원 입력을 요구 | E0204, feature schema 길이·derivation 비교 |
| Reference가 다른 채널 순서를 사용 | E0703, reference input schema 불일치 |
| 125 Hz profile을 그대로 연결 | E0205 또는 binding Fail, expected/observed rate |
| valid_interval 밖에서 ClockMap 사용 | E0402, 새 mapping 요구 |
| queue budget 초과 | Overflow Fault, 영향 원천 범위와 종료 trace |

## 사례 B: EEG와 fNIRS의 공통 시간 창

각 모달리티를 Session clock으로 정렬한 뒤 `[start,start+1s)` 창을 100 ms 간격으로 결합한다. 한 출력은 EEG·광학 feature의 명목 곱 타입을 가진다.

```bci
language "bci/0.1";
module demo.multimodal;

use std.source as source;
use std.time as time;
use std.artifact as artifact;
use std.recording as recording;
use std.runtime as runtime;

clock Session = time.session_clock();
const EegMap = time.mapping("evidence/eeg.clock.json");
const OpticalMap = time.mapping("evidence/optical.clock.json");
const EegReducer = artifact.reducer("operators/eeg-window-features.json");
const OpticalReducer = artifact.reducer("operators/optical-window-features.json");
resource Budget = runtime.resources("profiles/multimodal.resources.json");

run Main {
    runtime.configure(mode: Mode.Live, resources: Budget);
    let eeg = source.replay("fixtures/eeg", mode: Replay.Availability)
        |> time.align(to: Session, mapping: EegMap);
    let optical = source.replay("fixtures/optical", mode: Replay.Availability)
        |> time.align(to: Session, mapping: OpticalMap);
    let joined = time.join(
        (eeg, optical),
        width: 1[s], hop: 100[ms], origin: 0[s],
        membership: Membership.Definite,
        reducers: (EegReducer, OpticalReducer),
        missing: Missing.RejectWindow
    );
    recording.write(joined, to: "recordings/multimodal");
}
```

reducer artifact는 각 입력 schema, 최소 유효 표본 수, 상태·scratch 상한, support·quality·numeric 계약을 가진 등록 연산자를 참조한다. EEG reducer는 전압 특징, optical reducer는 입력의 quantity에 맞는 광학 특징을 반환한다. reducer가 고정 N을 요구하면 artifact에 project_grid와 그 보간 정책을 포함한다.

예상 시간 fixture는 EEG L=10 ms, optical L=30 ms, 각 ingress→join D=2 ms, Timer G=1 ms다. pending이 계약대로 처리되면 오른쪽 경계 1.000 s인 창의 close 상한은 1.033 s다. reducer 계산·출력 지연은 이후에 더한다.

시각 하한 1.040 s에서 optical pending의 측정 하한이 0.990 s이면 frontier는 최대 0.990 s다. 이 상태에서는 `[0,1)` 창의 완결 조건이 거짓이며 pending 반영을 기다린다. pending이 D를 넘게 지연되면 resource/time monitor가 계약 위반을 기록한다.

## 사례 C: 그룹 분할과 전체 학습 파이프라인

```bci
language "bci/0.1";
module demo.evaluation;

use std.dataset as dataset;
use std.protocol as protocol;
use std.learn as learn;
use std.artifact as artifact;
use std.report as report;
use std.runtime as runtime;

dataset Data = dataset.open("fixtures/motor-catalog.json");
protocol Evaluation = protocol.grouped(
    group: Group.Session, outer_folds: 5, inner_folds: 4, seed: 42
);
const Pipeline = artifact.estimator("operators/motor-estimator.json");
resource Budget = runtime.resources("profiles/offline.resources.json");

run Main {
    runtime.configure(mode: Mode.Record, resources: Budget);
    let split = dataset.partition(Data, protocol: Evaluation);
    let result = learn.nested_evaluate(
        split, estimator: Pipeline,
        metric: Metric.BalancedAccuracy,
        aggregate: Group.Session
    );
    report.write(result, to: "results/session-generalization");
}
```

catalog는 각 outer Train에서 4개 inner fold를 만들 수 있는 세션 수, class coverage, 원천 range를 제공한다. estimator artifact는 전처리, SPD covariance, tangent reference fit, 분류기, 필요 시 확률 calibration을 포함한다. 하이퍼파라미터 후보와 선택 지표도 artifact에 고정한다.

각 outer fold는 inner Fit/Tune으로 후보를 선택하고 outer Train에 refit한 새 모델을 outer Eval에 적용한다. 결과는 fold별 모델 digest·예측·label·quality 선택·class별 분모를 포함한다. 이 예제의 기대 출력은 지표 구조와 분할 적합성 판정이며, 정확도 값은 입력 자료에서 계산한다.

전체 catalog의 평균으로 normalization을 먼저 fit하는 변형은 Eval 조상을 모델에 포함하므로 E0702를 생성한다. 같은 raw range를 복제한 두 파일은 catalog 원천 ID를 기준으로 overlap 검사를 받는다.

## 사례 D: 고밀도 입력의 자원 계획

```bci
language "bci/0.1";
module demo.highdensity;

use std.source as source;
use std.schema as schema;
use std.runtime as runtime;
use std.recording as recording;

schema AP = schema.load("schemas/ap384-i16.schema.json");
resource Budget = runtime.resources("profiles/ap-fifo.resources.json");

run Main {
    runtime.configure(mode: Mode.Live, resources: Budget);
    let ap = source.replay("fixtures/ap384", mode: Replay.Availability);
    let checked = source.assert_schema(ap, expect: AP, rate: 30000[Hz]);
    recording.write(checked, to: "recordings/ap384");
}
```

fixture는 채널당 30표본인 384×30 i16 block을 1 ms마다 제공한다. profile의 해당 FIFO 계약은 burst b=23,040 byte, r=23,040,000 byte/s, R=50,000,000 byte/s, T=0.002 s다.

| 항목 | 계산 | 기대값 |
|---|---|---|
| i16 payload | 384×30,000×2 | 23.04 MB/s |
| f64 payload | 384×30,000×8 | 92.16 MB/s |
| block bytes | 384×30×2 | 23,040 byte |
| backlog 상한 | b+rT | 69,120 byte = 67.5 KiB |
| FIFO block 수 | ceil(69,120/23,040) | 3 |
| FIFO delay | T+b/R | 2.4608 ms |
| block 첫 표본→완성 | 29/30,000 | 약 0.966667 ms |
| 첫 표본→출력 경계 | 위 두 지연의 합 | 약 3.427467 ms |

MB는 10^6 byte, KiB는 1024 byte다. 이 FIFO의 3개 block capacity에 실행 중 lease·actor work·recorder metadata를 추가하여 전체 메모리 예산을 계산한다. profile의 R,T는 해당 흐름과 완료 경계를 명시한다. f64 변환을 추가하면 새로운 payload rate와 service 조건으로 계획을 재검사한다.

## 사례 E: 모델 교체와 지연 피드백의 사건 trace

모델 버전 M1은 b1=0에 활성화되어 있다. 아래 시각의 단위는 ms이며 같은 actor의 수락 순서를 사용한다.

| 시각 | 사건 | 기대 상태·출력 |
|---|---|---|
| 10 | P1 시작 | M1 reference 획득 |
| 11 | M2 Activate | active=M2 |
| 12 | P1 완료 | prediction.model=M1 |
| 13 | P2 시작 | M2 reference 획득 |
| 20 | P1 label 가용 | P1 평가 후 적응 자료에 반영 |
| 22 | M3 학습 완료 | a3=22, fit lineage에 label 포함 |
| 23 | M3 Activate | b3=23 |

피드백의 최소 fixture는 body `(input,previous) → (input+previous,input+previous)`, 초기 지연 1 tick·초기값 0·입력 `[1,2,3]`다. tick 출력은 `[1,3,6]`이고 state lineage는 앞선 tick을 참조한다. 이 body는 등록한 scalar 합산 원시 연산자를 사용한다. d=0 설정은 E0304를 기대한다.

## 예제에서 사용하는 구성 API

| API | 구체화·실행 계약 |
|---|---|
| `schema.load`, `model.load`, `signal.coefficients`, `artifact.reducer/estimator` | 잠금 파일의 immutable artifact를 조회하고 schema·digest를 검사 |
| `time.session_clock` | 세션 origin의 명목 clock descriptor 생성 |
| `time.mapping` | mapping artifact 로드, prepare 시 epoch·가용성·유효 구간 검사 |
| `device.bind` | profile와 config에서 binding descriptor 생성 |
| `runtime.budget/resources` | 메모리·큐·CPU·증거 profile 생성/조회 |
| `runtime.configure` | run마다 1회, 첫 source 연결 전에 mode와 budget 고정 |
| `source.acquire` | prepare descriptor 검증 후 native clock stream 생성 |
| `source.replay` | 원래 source schema와 논리 availability를 복원 |
| `source.assert_schema` | 데이터·시간 유지, descriptor 비교 obligation 생성 |
| `model.predict` | 모델 schema 검사, 작업 시작 버전 snapshot, 결과 lineage 연결 |
| `recording.write` | stream 또는 stream tuple을 sink에 연결, 관련 제어·계보 자동 기록 |
| `dataset.partition`, `learn.nested_evaluate` | protocol에 따라 fold와 역할 구체화, 학습·평가 Record graph 생성 |
| `report.write` | evaluation artifact와 지표 metadata를 결과 경로에 commit |

`runtime.configure`의 Mode.Live는 진입점 전체에 Live effect를 요구하고, Mode.Record는 전체 기록 의존성을 허용한다. resource 값 `MiB`는 저장량 kind의 단위다. enum과 quality 이름은 표준 prelude에서 제공한다.
