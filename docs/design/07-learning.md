# 학습, 분할, 평가와 모델 활성화

근거: [원문 학습 규격](../../bci_dsl_latex/sections/07_learning.tex).

현재 구현은 [학습·예측 안내](../learning.md)의 명시적 Fit·Tune·Eval 분할, Fit 정규화·softmax, 전체 후보 계보를 포함한 Tune 선택, 모델 저장·계층별 평가·cluster bootstrap, 준비 시 모델 고정과 Select·Hold 판정 경로를 제공한다. 아래 API·동적 활성화·적응 절은 전체 목표 계약이다.

## 데이터 정체성과 분할

데이터 카탈로그의 항목은 `dataset_id, subject_key, session_key, trial_key, source_id, acquisition_epoch, sample_range, label_ref`를 가진다. 파생 창은 이 원천 범위들을 보존한다. 같은 자료를 다른 파일로 복사한 경우 원천 catalog identity로 중복을 판정한다.

역할은 Fit, Tune, Eval, Adapt다. split manifest는 그룹 함수, 선택한 원천 범위, role, fold, seed, generator version, parent split digest를 가진다. 그룹 함수는 subject/session/trial 또는 catalog에서 검증한 명시적 key다.

split 구성 시 다음 검사를 수행한다.

1. 같은 독립 그룹의 역할 간 분리 조건을 검사한다.
2. 원천 범위가 겹치는 파생 창을 같은 그룹으로 묶는다. 사용자가 지정한 그룹이 이 조건을 깨면 `E0701 SplitOverlap`을 생성한다.
3. filter 상태·baseline·정규화가 읽은 범위까지 ancestors에 포함한다.
4. 각 class의 지표 계산 분모가 양수인지 검사한다.

window 생성 후 무작위 분할을 요청해 원천 구간이 겹치면 overlap 관계를 진단한다. split은 원천 catalog에 먼저 적용하고 전처리 상태는 fold·role 경계에서 초기화한다. 고정 계수 FIR도 초기 상태에 들어간 다른 role의 표본을 provenance에 포함하므로 경계 검사가 가능하다.

## fit 효과와 학습 API

```text
learn.fit(estimator, data: Dataset<...,Fit,...>, protocol) -> Model
learn.select(candidates, tune: Dataset<...,Tune,...>, metric) -> SelectedModel
learn.evaluate(model, data: Dataset<...,Eval,...>, protocol) -> Evaluation
learn.adapt(model, input: DatasetOrStream<...,Adapt,...>, policy) -> CandidateModel
```

표준 estimator에는 normalization, CSP, tangent reference, classifier, probability calibration을 둔다. filter 설정·특징 선택·decision threshold의 데이터 기반 선택도 fit effect다. 정적인 상수는 const artifact를 참조한다.

```text
ModelArtifact {
  model_id, estimator_id, estimator_version, parameters_digest,
  input_schema, output_schema, numeric_contract,
  fit_lineage, selection_lineage, split_digest,
  available_at, training_environment, rng_algorithm, seed
}
```

모델의 `fit_lineage`는 수치 매개변수와 선택 과정의 조상을 포함하는 closure다. `selected_from`은 탈락 후보·평가에 사용한 Tune 자료까지 연결한다. 고정 평가의 계약은 `fit_ancestors ⊆ ancestors(Fit∪Tune)`과 `g(fit_ancestors) ∩ g(Eval)=∅`다. violation에는 충돌 원천 ID와 연산 경로를 제공한다.

모델 파일을 읽을 때 compiler는 schema·estimator digest·lineage completeness를 확인한다. 외부 모델은 import manifest에 원천·통계·선택 절차 증거를 제공한다. 증거가 필요한 평가 계약에는 Pending obligation을 생성한다.

## 중첩 평가

outer fold j마다 독립 모델 lineage를 만든다. inner Fit/Tune은 outer Train의 부분집합이며 outer Eval과 그룹이 분리된다. 선택 결과를 이용한 outer Train 재학습은 새 모델 버전과 refit lineage를 생성한다.

각 fold의 전처리 통계·CSP·접공간 reference·classifier는 같은 protocol 인스턴스를 공유한다. pipeline의 모든 fit node에 role와 split ID를 전파한다. split을 넘나드는 cache key는 `(operator_digest, parameters_digest, split_digest, role, source_ranges)`를 포함한다.

## 준비, 활성화와 예측

학습·artifact 검증 완료 시 모델은 `Candidate`가 된다. `model.activate`는 actor 입력 사건을 만들고, 해당 actor가 사건을 수락한 시각을 b_v로 기록한다.

```text
max(availability of all fit and label ancestors) <= a_v <= b_v
prediction.start >= b_selected
prediction.lineage = input.lineage union selected_model.fit_lineage
```

예측 actor는 작업 시작 시 active pointer를 읽고 immutable model reference를 보유한다. 실행 중 새 모델을 활성화하면 다음 작업부터 적용한다. 종료 시점이 늦은 기존 작업은 시작 시 선택한 버전을 결과에 기록한다. event 순서가 같은 시각에서 충돌하면 [03](03-semantics.md)의 수락 순서로 결정한다.

schema·클래스 순서·numeric contract가 같은 모델은 같은 graph epoch에서 교체할 수 있다. 입력/출력 schema가 바뀌는 모델은 새 실행 계획과 epoch barrier를 요구한다. activation 실패 시 기존 active model을 유지하고 실패 사건을 기록한다.

## 적응 프로토콜

prequential의 순서는 `Predict → ObserveLabel → Score → Update → Candidate → Activate`다. label event에는 해당 prediction ID와 실제 availability가 필요하다. 지연 label을 받으면 원래 예측에 연결하여 평가하고 이후 업데이트에 반영한다.

Adapt 입력은 기존 fit lineage와 합쳐 새 모델을 생성한다. 비지도 adaptation도 입력 특징과 정책을 조상으로 기록한다. 예측을 만든 모델·그 예측의 label·후속 모델은 서로 다른 immutable node이므로 provenance graph는 DAG를 유지한다.

적응 queue는 크기와 업데이트 빈도·최대 지연을 resource 계약에 둔다. 초과 시 `SkipUpdate` 또는 `Fault`를 명시하고 skip의 원천 범위를 기록한다. 학습 actor의 가변 optimizer 상태는 해당 actor 한 개가 소유한다.

## 평가 artifact

평가는 predictions/labels의 연결 표, model digest, split manifest, 품질 선택 정책, 포함·제외 수, 지표 정의, 집계 단위를 저장한다. balanced accuracy는 class별 recall 평균이며 각 class의 TP+FN>0을 요구한다. 조건 실패는 `MetricUndefined`와 해당 class를 반환한다.

[현재 평가 구현](../evaluation.md)은 시행·세션·피험자별 aggregation을 명시한 순서로 수행한다. bootstrap은 재표집 단위, 반복 수, 신뢰수준, RNG와 seed를 포함한다. label 결측과 RejectedWindow의 분모 처리 정책은 protocol에 고정한다. 여러 모델 비교 결과를 이용한 추가 선택은 새로운 selection lineage를 만든다.

## 적합성 사례

| 입력 | 기대 판정 |
|---|---|
| Eval 표본으로 계산한 global mean을 Fit 정규화에 사용 | E0702 FitRoleViolation, mean 경로 표시 |
| 같은 시행의 겹치는 두 창을 Fit/Eval에 배정 | E0701 SplitOverlap |
| label availability=12, update request=10 | 12 이후로 의존 대기, a_v≥12 |
| predict start=10, activate=11, predict end=12 | 예측은 10에 선택한 모델 digest 유지 |
| class 순서가 바뀐 모델을 같은 epoch에서 활성화 | E0703 ModelSchemaMismatch |
