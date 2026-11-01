# 타입, 정제 조건과 효과

근거: [원문 데이터 모델](../../bci_dsl_latex/sections/02_data_model.tex), [코어 의미론](../../bci_dsl_latex/sections/03_semantics.tex).

## kind와 값 타입

컴파일러는 `Value`, `Nat`, `Unit`, `Schema`, `Clock`, `Rate`, `Quality`, `Role`, `Effect` kind를 구분한다. 생성자의 인수 위치마다 kind를 검사한다.

| 타입 | 의미와 조건 |
|---|---|
| `bool`, `i16`, `i32`, `i64`, `u16`, `u32`, `u64`, `f32`, `f64`, `string` | 저장 표현과 범위 |
| `Vec<D,C>` | 길이 C의 값, C > 0 |
| `Epoch<D,C,N>` | C×N 값과 내부 표본 격자, C,N > 0 |
| `Cov<D,C>`, `SPD<D,C>` | 대칭 공분산, 검증된 양정치 행렬 |
| `Score<Classes>`, `Probability<Classes>` | 명목 class 순서와 보정 의미 |
| `Event<Payload>`, `Product<A,B>` | 사건과 포트 곱 |
| `Stream<V,S,K,R,Q>` | 값·스키마·시계·발생 규칙·품질 |
| `Dataset<V,S,Role,Split>` | 데이터 역할과 분할 정체성 |
| `Model<Input,Output,FitLineage>` | 입출력 스키마와 학습 계보 |
| `Handle<Resource,Capabilities>` | actor가 소유하는 자원에 대한 선형 제어 권한 |

소스의 튜플 타입은 `Product`의 다항 확장이다. `Epoch` 내부 rate는 타입의 보조 refinement에 보관하고, stream의 `R`은 epoch 자체의 발생률을 뜻한다. 주파수 스펙트럼과 특징 벡터는 각각 주파수 bin schema와 feature schema를 가진다.

입력값에서 내부 rate를 추론할 수 있는 위치는 `Epoch<D,C,N>` 표기를 사용한다. graph 매개변수처럼 내부 격자를 지정하는 위치에서는 `Epoch<D,C,N,f_internal>`로 refinement를 명시한다. `WindowStart.FirstValid`의 런타임 위상은 `PendingUniform(f, phase_key)`로 나타내고 의무 ID를 통해 확정한 격자와 연결한다.

## 단위와 측정량

차원은 `(M,L,T,I,Theta,N,J)`의 정수 지수 벡터다. 단위는 `(dimension, scale, offset)`이며 `x_SI = scale*x + offset`으로 환산한다. scale과 offset은 정규화 유리수다. `uV`는 전압의 `10^-6 V`, `Hz`는 `s^-1`로 등록한다.

덧셈은 같은 명목 측정량·단위 표현을 요구한다. 다른 배율은 `signal.convert_unit`으로 변환한다. 곱셈·나눗셈은 차원을 합산·차감한다. offset을 갖는 affine 양의 곱셈은 선형 기준 단위로 변환한 값에 적용한다. `log`의 인수는 양의 무차원 값이다.

동일 차원을 가진 전압·광강도 장치 단위·농도 변화량은 각자의 명목 측정량 ID를 보유한다. 단위 일치와 측정량 일치를 별도로 검사한다. 단위 변환은 측정량 ID를 유지하고, 광학 변환은 출력 측정량을 새로 생성한다.

자원 매개변수는 별도의 `Storage`·`Count` quantity kind를 사용한다. `byte`, `KiB=1024 byte`, `MiB=1024² byte`, `MB=10^6 byte`를 등록하고 `byte/s`는 저장량 유량으로 해석한다. 물리량의 SI 차원 벡터와 자원량의 kind는 각자의 산술·인수 검사를 따른다.

## 채널 스키마

```text
Channel {
  id, quantity_id, dimension, unit,
  reference_id, role,
  geometry: { frame_id, coordinates, length_unit, placement_id } | None,
  device_channel, calibration_digest
}
Schema { id, ordered_channels[], derivation_digest }
```

스키마 정체성은 순서 있는 전체 기술자의 정규화 digest다. 별칭 이름은 표시 정보다. 채널 수가 같아도 순서·기준·파장·좌표계가 달라지면 schema ID가 바뀐다.

`select`는 입력 채널 ID의 순서 있는 부분집합을 받는다. `reorder`는 전단사 순열을 받는다. `rereference`는 가중치와 입력 채널 ID로 새 reference ID를 생성한다. feature schema는 원천 채널, 연산자, 대역, 정규화, 모델 basis digest를 포함한다.

## 발생 규칙과 시계

`Uniform(f, phase)`에서 f는 양의 유리수 Hz, phase는 해당 시계 origin 상대 유리수 시간이다. `Events`는 레코드별 시간 구간을 사용한다. 균일 stream의 gap은 원래 격자 index를 보존한다. 유효 값의 관측 빈도는 quality와 gap 기록에서 계산한다.

시계 타입은 domain ID와 origin ID를 가진다. 레코드의 clock epoch와 time-view ID는 런타임 refinement다. 같은 `Clock` 타입에서도 서로 다른 epoch의 레코드를 합칠 때 epoch barrier를 요구한다. [시간 규격](04-time.md)이 매핑 유효 구간을 검사한다.

## 효과

연산 효과는 원문 `(mode, capabilities, lookahead)`에 학습·상태·출력 효과를 추가한다.

```text
Effect {
  mode: Live | Record,
  capabilities: Set<Capability>,
  support: SymbolicSupport,
  lookahead: Duration | Unbounded,
  fit_roles: Set<Role>,
  state_owner: ActorClass | Pure,
  writes: Set<OutputResource>
}
```

직렬 합성은 연결 타입의 정확한 일치, 기능 합집합, 학습 의존성 합집합을 요구한다. mode join은 `Live ⊔ Live = Live`, `Record ⊔ m = Record`다. support는 시간 좌표를 일치시킨 뒤 실제 인덱스/구간 사상을 합성한다. 순수 고정 offset 구간은 Minkowski 합을 사용한다. `lookahead`는 합성 support에서 출력 anchor 이후의 최대 길이로 계산한다.

`Live`에는 유한 전방 창이 포함된다. 연산자는 필요한 자료와 완결 사건의 가용 시각을 기다린다. `Record`의 전체 구간 필터는 입력 범위와 종료 사건을 요구한다. live run의 출력 경로에서 Record 효과를 발견하면 `E0301`을 생성한다. 미리 생성한 고정 모델 artifact를 읽는 예측 효과는 Live다.

## 제약과 의무

| 범주 | 예 | 판정 위치 |
|---|---|---|
| 유한 구조 | C,N>0, 채널 순열, 클래스 순서 | 컴파일 |
| 정확 산술 | rational rate, 단위 차원, hop | 컴파일 |
| 그래프 | 기능 집합, fit 흐름, 순환 지연 | 컴파일 |
| 구성 | SDK 채널 목록, 실제 출력률, 접근 scope | bind/prepare |
| 수치 | Cholesky 성공, 확률 정규화, 행렬 rank | artifact 검증/호출 |
| 시간·자원 | mapping 유효성, 서비스 상한 | prepare 및 실행 중 monitor |

각 obligation은 `id, predicate, source_span, phase, evidence_kind, dependencies, failure_policy`를 가진다. 판정은 `Pass(evidence)`, `Fail(counterexample)`, `Pending(required_inputs)`다. 실행 진입점의 경로 의무는 prepare 종료 시 Pass를 요구한다. 실행 중 변경되는 조건은 guard와 fault 경로를 계획에 배치한다.

동적 descriptor는 `Exists<S,R>. Stream<V,S,K,R,Q>`로 내부 표현한다. `source.acquire`의 `expect`와 `rate` 인수가 원하는 구체 타입을 제시하고, prepare가 descriptor 일치 증거를 생성한다. 타입을 특정할 수 없는 포트의 소비자는 descriptor 기반 계약을 등록해야 한다.

## 진단 예

| 입력 조건 | 진단 | 수정 행동 |
|---|---|---|
| EEG V stream을 uV 포트에 직접 연결 | `E0203 UnitMismatch` | `convert_unit` 삽입 |
| 같은 8채널의 순서가 서로 다름 | `E0204 SchemaMismatch` | `reorder`와 모델 schema 재검사 |
| 250 Hz 신호를 200 Hz 포트에 연결 | `E0205 RateMismatch` | 재표본화 필터와 위상 선언 |
| clock A와 clock B의 창 결합 | `E0401 ClockMismatch` | 유효한 mapping으로 공통 time view 생성 |
| N=1 공분산 | `E0501 DomainViolation` | N≥2 창 선택 |
| Hold 사건을 확률 벡터로 취급 | `E0201 KindMismatch` | decision event 포트에 연결 |

추론 결과는 `explain types` 산출물에 포트별 정규화 타입과 변환 경로로 표시한다.
