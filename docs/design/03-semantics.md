# 코어 의미론과 상태 계약

근거: [원문 코어 의미론](../../bci_dsl_latex/sections/03_semantics.tex). 이 문서는 원문의 전이·합성 규칙에 실행 사건 순서와 실패 동작을 연결한다.

## 코어와 관측

```text
Core = Primitive(id, config)
     | Sequence(Core, Core)
     | Parallel(Core, Core)
     | Delay(ticks, initial_values, tick_domain)
     | Feedback(Core, delayed_ports)
```

stream 이름 재사용은 불변 블록의 fan-out으로 구체화한다. fan-out의 각 소비 경로는 독립 queue와 품질 정책을 가진다. `Parallel`은 포트 곱을 생성한다. 데이터의 시간 대응은 별도의 join 원시 연산자가 결정한다.

원시 연산자 `o=(input, output, Z, z0, delta, J, E)`에서 `delta`는 상태·입력 사건을 받아 새 상태와 유한 출력 사건열을 반환한다. `z0∈J`, `z∈J ∧ valid(x) ⇒ z'∈J ∧ valid(outputs)`를 registry의 검증 계약으로 둔다. 수치 정의역 오류는 구조화한 fault 출력으로 표현하여 전이를 전역적으로 정의한다.

관측 대상은 값, 가용 시각, 대표 측정 구간, quality, 원천 범위, 모델 버전, 세대, 외부 명령과 응답이다. 상태 저장 최적화는 이 관측 관계와 수치 허용오차를 보존해야 한다.

## 레코드와 사건

```text
Envelope {
  record_id, source_id, sequence,
  acquisition_epoch, schema_epoch, clock_epoch,
  value_or_payload,
  sample_index: Observed | Reconstructed | Unresolved,
  measurement_interval, available_at, availability_boundary,
  time_view_id, quality, lineage_id
}
InputEvent = Data | Gap | Watermark | Timer | EndEpoch | EndStream
           | Configure | ActivateModel | ClockMapping | Fault
```

`available_at`은 실행 세션 기준 시계의 ns 값이다. 다른 호스트의 관측은 원래 구간과 변환 근거를 보존하고, 실행 ingress가 자기 경계의 availability를 부여한다. 각 내부 actor의 출력도 완료 경계에서 새로운 availability를 얻는다.

같은 가용 시각의 독립 입력은 `(available_at, source_id, sequence)` 순서로 선형화한다. 동일 원천의 제어와 데이터는 하나의 증가 sequence를 공유한다. 의존 사건은 원인 사건 뒤에서 처리한다. reference executor는 이 순서를 사용하고, 병렬 실행기는 actor별 입력 수락 순서와 실제 모델 선택을 trace에 남긴다.

## 가용 시각 인과성

`cut_u(S)`는 시각 u까지 가용한 사건열이다. Live 연산자는 다음을 만족한다.

```text
cut_u(F(S)) = cut_u(F(cut_u(S)))
available(y) >= max(available(r) for r in dependencies(y))
```

의존성에는 이전 상태를 만든 입력, 분기 조건, 모델·계수, 창 완결 사건, Timer가 포함된다. 처리 시간이 달라지는 실행끼리의 비교는 동일 환경 사건열을 고정한 뒤 수행한다. 실시간 trace의 실제 완료 시각은 runtime clock에서 취득한다. 참조 재생은 기록된 논리 availability와 관측 순서를 사용한다.

상태 누적 연산자의 provenance는 상태가 참조한 조상들의 closure를 포함한다. 값에 대한 직접 기여와 분기·매개변수 선택 의존성을 모두 추적한다. 구현은 입력 범위 노드와 이전 상태 lineage 노드를 연결하여 조상 집합을 압축한다.

## 논리 지연과 피드백

`delay(d,z)`는 d≥1과 d개 초기값을 요구한다. tick k<d에서 z[k], 이후에는 입력 x[k-d]를 출력한다. 지연 출력의 대표시각은 현재 tick이고, 원래 입력의 측정 구간은 support와 lineage에 남는다. 초기값은 `Origin.Synthetic`과 초기화 artifact를 가진다.

피드백 구성 함수는 다음 서명을 가진다.

```text
flow.feedback(input, body: Graph<(A,S),(B,S)>,
              ticks: Nat, initial: Array<S>, tick: TickDomain) -> B
```

body graph의 첫 입력은 외부 입력, 둘째 입력은 지연 상태다. 반환 튜플의 둘째 출력이 지연 입력에 연결된다. tick domain은 Uniform stream의 격자 또는 명시적 Timer stream이다. event stream의 tick 대응은 `control.sample_on` 계약으로 명시한다.

검사기는 모든 지연 간선을 제거한 `G0`에 위상 정렬을 적용한다. 순환이 남으면 `E0304 InstantCycle`과 순환 포트를 보고한다. 초기값 타입·길이·품질이 일치하고 전이가 결정적이면 각 tick의 실행은 유일하다. 원문의 유일성 조건은 [런타임](09-runtime.md)의 준비·실행 가능성 조건과 함께 적용한다.

## 품질 전파

quality는 채널/표본별 유효성, origin, flags를 가진다. origin은 `Measured`, `Interpolated`, `Recovered`, `Synthetic`을 포함한다. 여러 입력을 결합하면 lineage에 모든 origin을 유지하고 출력에는 `Mixed`와 구성 origin 집합을 기록한다.

| 상황 | 기본 동작 | 후속 영향 |
|---|---|---|
| 유효 입력의 수치 변환 | 마스크 유지, 변환 lineage 추가 | schema 규칙에 따라 채널 대응 |
| FIR 초기 L−1개 | `Warmup`과 invalid | `Warmup.Full` 창이 첫 완전 유효 구간을 선택 |
| gap | 원래 누락 범위의 Gap 사건 | 상태 reset, warmup 재시작 |
| saturation | 채널 마스크 invalid, `Saturation` | 창 품질 정책으로 전달 |
| 명시적 보간 | 보간값·원천 범위·`Interpolated` | 해당 정책을 허용한 소비자가 처리 |
| clock boundary 모호성 | `BoundaryAmbiguous` | definite/possible 소속 정책 적용 |
| schema 전이 | EndEpoch 후 상태 초기화 | 새 epoch와 새 타입 증거 |

창의 기본 `Quality.RequireAll`은 전체 채널·표본의 valid를 요구한다. 조건 실패 시 `RejectedWindow` 사건에 창 ID, 원인, 원천 범위를 기록한다. invalid 값을 NaN으로 대체하는 동작은 명시적 `Quality.FillNaN` 정책에 속하며 입력 영역을 허용하는 커널에 연결한다.

## 실패와 종료

원시 전이는 `Emit`, `RejectRecord`, `Fault` 결과를 반환한다. Reject는 해당 데이터/창의 품질 결과이며 전체 세션은 다음 사건을 처리한다. Fault는 해당 actor의 데이터를 닫고 supervisor에 원인과 마지막 확정 sequence를 전달한다.

EndStream은 도착·복구가 종료되었다는 source의 확정 사건이다. 시간 창은 그 source에 대해 +∞ frontier를 받을 수 있다. 표본수 창의 마지막 부분 구간은 `Tail.Discard` 또는 `Tail.Pad` 정책으로 처리한다. Pad는 명시한 채움 값과 Synthetic origin을 사용한다. 기본은 Discard이며 `PartialWindow` 사건을 기록한다.
