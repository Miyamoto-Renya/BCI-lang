# 모델 학습·후보 선택·평가·온라인 예측과 판정

`bci learn fit`은 명시적 Fit·Tune·Eval 분할을 검사하고, valid Fit 특징으로 정규화·다중 클래스 softmax 분류기를 학습한다. Fit/Eval 데이터는 고정 모델을 평가한 뒤 모델·평가 파일을 저장한다. Tune을 포함한 데이터는 후보 모델을 저장한다. `bci learn select`는 Tune 지표로 후보를 선택한 뒤 Eval로 최종 평가한다. BCi 그래프는 모델을 로드해 `Probability`를 계산하고 `decision.threshold`로 `Select` 또는 `Hold`를 기록한다.

## 실행

```sh
cargo build -p bci-cli --offline --locked
./target/debug/bci learn fit examples/learning.dataset.json --output recordings/learning-fit
./target/debug/bci learn evaluate recordings/learning-fit/model.json examples/learning.dataset.json --output recordings/learning-evaluation.json
./target/debug/bci run examples/learning.bci --output recordings/learning-run
```

[learning.dataset.json](../examples/learning.dataset.json)은 특징 2개·클래스 2개·Fit 4개·Eval 4개를 가진 수치 예제다. [learning.model.json](../examples/learning.model.json)은 이 입력으로 생성한 모델이다. [learning.bci](../examples/learning.bci)는 해당 모델을 고정하고 [learning.csv](../examples/learning.csv)의 특징 10개를 읽는다. 특징·확률·판정 포트에서 총 30개 레코드를 기록한다.

새 학습 결과를 그래프에서 사용하려면 `model.load`의 경로를 생성한 model.json으로 지정한다. 상대 경로는 `.bci` 파일의 디렉터리에서 해석한다. CLI의 `--output`은 현재 작업 디렉터리를 기준으로 새 경로를 요구한다.

```bci
use std.model as model;
use std.decision as decision;
const Classifier = model.load("learning.model.json");

// run 본문에서 특징 포트를 연결한다.
let probabilities = features |> model.predict(using: Classifier);
let selected = probabilities |> decision.threshold(minimum: 0.8);
recording.write((probabilities, selected), to: "recordings/predictions");
```

모델 입력은 `Vec<f64,D>`이며 특징 순서·측정량·단위 배율·기준이 학습 schema와 일치해야 한다. feature를 생성하는 `bandpower`·`log_power` 등의 출력 schema를 학습 데이터에 적용한다. 현재 feature table codec은 특징 값·라벨·품질·전처리 조상을 JSON으로 받는다.

## 데이터와 분할

`bci-dataset/0.1`은 다음 항목을 가진다.

| 항목 | 계약 |
|---|---|
| dataset_id | catalog dataset ID |
| schema | dataset 파일 기준의 bci-schema/0.1 경로 |
| classes | 고유한 클래스 이름 2..32개, 배열 순서를 출력 index로 사용 |
| group_by | subject, session 또는 trial |
| training | iterations, learning_rate, l2 |
| rows | 고유 ID·그룹·역할·원천 조상·라벨·특징·valid |

각 행은 `id, subject, session, trial, role, label, values, valid, ancestors`를 가진다. group_by=subject는 subject를, session은 subject/session을, trial은 subject/session/trial을 독립 그룹으로 묶는다. role은 Fit·Tune·Eval 중 하나다. loader는 행을 고유 ID 순서로 정렬한다.

ancestor의 형태는 `{source_id, epoch, first, last}`다. source_id는 파일을 복사해도 유지하는 측정 원천 ID이며 first..last는 닫힌 index 구간이다. 전처리기가 읽은 필터 이력·baseline·창의 전체 구간을 ancestors에 제공한다. 검사기는 제공한 catalog 정체성과 구간을 기준으로 다음을 확인한다.

- 같은 독립 그룹은 하나의 역할에 속한다.
- 같은 source_id·epoch에서 겹치는 구간은 같은 그룹·역할에 속한다.
- 행 ID는 고유하며, 각 특징 벡터·valid·라벨은 schema와 클래스 순서를 따른다.
- 각 클래스에 valid Fit 표본이 한 개 이상 존재한다.
- 모든 후보의 분할·선택 Tune·새 Eval 자료가 같은 분리 조건을 만족한다. 탈락 후보의 Fit 원천도 이 검사에 참여한다.

그룹 충돌과 원천 구간 겹침은 E0701 SplitOverlap으로 행·원천을 표시한다. 모델의 실제 학습 행이 Fit 역할과 충돌하면 E0702 FitRoleViolation을 반환한다. 유효성 정책은 RequireAll이다. 한 채널이라도 valid=0이면 해당 행을 통계·학습·평가 계산에서 제외한다. 분할 검사는 품질 제외 행까지 포함한 전체 행의 계보를 사용한다. 여러 catalog를 함께 검사할 때 오류 위치는 `catalog:<입력 위치>:row:<정렬한 행 위치>`로 표시하며, 원래 행 ID는 각 artifact에 보존한다.

## 학습과 평가 수식

profile은 `bci-standardized-softmax/0.1`이다. 정규화는 valid Fit 행만 사용해 Welford 방식으로 mean과 population variance를 계산한다. 상수 특징의 scale은 1이다.

```text
z[j] = (x[j] - mean[j]) / scale[j]
logit[k] = bias[k] + Σ_j weight[k,j] * z[j]
p[k] = exp(logit[k] - max(logit)) / Σ_i exp(logit[i] - max(logit))
```

weight와 bias는 0에서 시작한다. 고정된 행 순서로 전체 Fit batch의 cross-entropy gradient를 누적하고, weight에 L2 항을 더해 지정한 횟수만큼 gradient descent를 적용한다. bias의 regularization 계수는 0이다. RNG는 None이며, Eval 관측은 학습 통계·반복 수·weight 선택과 분리한다.

iterations는 1..10000, learning_rate는 (0,1], l2는 [0,100]이다. dataset은 최대 100000행·256특징, 행마다 최대 64개 ancestor를 가진다. 입력 파일 상한은 16 MiB이며, `valid Fit N × D × K × iterations`의 상한은 200000000이다. 학습과 예측의 중간 수치가 유한 범위를 벗어나면 오류를 반환한다.

평가는 Eval 행에서 label-major confusion matrix, accuracy, 클래스별 recall 평균인 balanced accuracy를 계산한다. 각 클래스의 valid Eval 분모는 양수여야 한다. 분모가 0이면 E0704 MetricUndefined와 클래스 이름을 보고한다. 평가 파일은 모델 digest, 포함·제외 행, 예측/라벨 연결, 원천 계보와 Sample 집계 정책을 보관한다. `learn fit`, `learn select`, `learn evaluate`의 `--protocol FILE`은 최종 Eval에 [계층별 집계·cluster bootstrap](evaluation.md)을 적용한다. 요청한 단계의 그룹 분모·고정 seed·반복별 지표·percentile 구간은 bci-evaluation/0.2 보고서에 저장한다.

softmax 출력의 보정 상태는 Uncalibrated다. 별도의 확률 보정 estimator, CSP·접공간 변환·중첩 평가·Adapt protocol은 [전체 학습 설계](design/07-learning.md)의 확장 항목이다.

## Tune 기반 후보 선택

```sh
./target/debug/bci learn fit examples/selection-early.dataset.json --output recordings/candidate-early
./target/debug/bci learn fit examples/selection.dataset.json --output recordings/candidate-trained
./target/debug/bci learn select examples/selection.dataset.json \
  --candidate recordings/candidate-early/model.json \
  --candidate recordings/candidate-trained/model.json \
  --output recordings/selected
./target/debug/bci run examples/selection.bci --output recordings/selection-run
```

두 dataset은 같은 Fit 4개·Tune 5개·Eval 4개와 특징 schema를 공유한다. solver 반복 수는 각각 1과 400이다. Tune 5개 중 한 개는 valid=0 채널을 포함하여 지표에서 제외한다. valid Tune 4개의 balanced accuracy는 앞선 후보 0, 뒤 후보 1이며 두 번째 후보가 선택된다. 선택 모델의 Eval confusion은 `[2,0,0,2]`다. `selection.bci`는 이 흐름으로 생성한 [selection.model.json](../examples/selection.model.json)을 로드한다. 새 학습 결과를 사용하려면 그래프의 `model.load` 경로를 생성한 model.json으로 지정한다.

선택 계약은 다음과 같다.

- 직접 학습한 고유 모델 2..16개를 명령의 `--candidate` 순서로 받는다. 각 후보는 입력 특징·클래스 순서·group_by를 공유한다.
- 모든 후보를 같은 valid Tune 행으로 평가한다. 지표는 Sample 집계의 balanced accuracy이며, 클래스 순서대로 f64 recall 평균을 누적한다. 최대 지표가 같은 후보 중 명령에서 먼저 지정한 후보를 선택한다.
- 각 클래스의 valid Tune·Eval 분모가 양수여야 한다. 0인 분모는 E0704 MetricUndefined와 역할·클래스로 보고한다.
- 모든 후보의 전체 분할과 선택 dataset의 전체 분할을 검사한다. 기존 Eval 그룹·원천을 Tune으로 재배정하면 E0701 SplitOverlap을 반환한다.
- 선택 모델은 승자 수치, 모든 후보 모델, 품질 제외 행을 포함한 Tune 분할, 실제 Tune 행 목록, 값·라벨·원천의 Tune digest, 후보별 confusion과 승자 index를 보관한다. 이 증거 전체가 새 model_id에 참여한다.
- 선택 결과를 먼저 확정한 뒤 Eval 지표를 계산한다. Eval 값·라벨을 변경해도 선택 model_id와 승자는 유지된다.
- `learn select`는 최종 평가 성공 후 새 디렉터리에 `model.json`, `selection.json`, `evaluation.json`을 저장한다. 평가 실패 시 원인을 수정한 뒤 같은 출력 경로로 다시 실행한다.

선택 증거는 한 층이며 후보 입력은 직접 학습 모델을 요구한다. 통합 계보 검사 상한은 catalog별 행을 합쳐 100000개다. Tune 계산 작업량 `전체 Tune N × 후보 수 × D × K`, 단일 지표 계산 작업량 `해당 역할 N × D × K`의 상한은 각각 200000000이다. 각 JSON 파일은 쓰기·읽기 모두 16 MiB를 적용한다.

`selection.json`의 판본은 `bci-selection/0.1`이며 metric=`BalancedAccuracy`, tie_break=`FirstCandidate`, quality=`RequireAll`, aggregation=`Sample`을 고정한다. loader는 저장한 confusion의 클래스별 분모·지표·승자와 선택 모델의 수치를 대조한다. 실제 Tune 값의 재계산에는 동일 dataset과 후보 모델을 `learn select`에 제공한다.

## 모델 파일과 실행 고정

`bci-model/0.2`는 다음을 보관한다. loader는 기존 `bci-model/0.1`도 읽으며, 직접 학습 모델의 의미 digest를 유지한다.

- 입력 특징 schema와 고정 클래스 순서.
- mean·scale·class-major weight·bias와 solver 설정의 정확한 f64 bit.
- dataset ID·group_by·분할 manifest와 전체 ancestor 구간.
- 실제 valid Fit 행 목록과 값·라벨·schema의 fit digest.
- profile·수치 계약·모델 내용의 SHA-256 model_id.
- 직접 학습 모델의 `selection: null` 또는 전체 후보와 Tune 근거를 포함한 선택 증거.

loader는 shape·유한 수치·양의 scale·클래스·역할 계보·model_id를 대조한다. compiler와 독립 verify는 입력 특징 계약을 다시 검사한다. 모델 schema 불일치는 E0703 ModelSchemaMismatch로 보고한다. 새 파일은 임시 파일 쓰기·sync 이후 hard link로 게시하고, 기존 경로 충돌은 E0902를 반환한다.

모델은 실행 준비에서 고정된다. 해당 실행의 예측은 같은 immutable 모델을 사용하고, 모델 교체는 새 실행에 적용한다. 실행 manifest의 predict 커널은 전체 모델과 `activation: Prepare`를 저장한다. 확률·판정 채널의 reference는 `model:<model_id>`이며 원천 시각·범위·origin은 입력에서 이어받는다. 출력의 학습·선택 계보는 이 model_id를 통해 전체 후보의 분할과 Tune 증거에 연결한다.

## 확률·판정 타입과 품질

`Probability<Classes>`는 클래스 순서의 f64 벡터다. 유효 출력은 각 성분이 0..1이고 합이 1±1e-9를 만족한다. 무효 특징은 전체 확률 값 0·전체 mask 0·InvalidInput으로 전달한다.

`Decision<Classes>`의 값은 `[selection, confidence]`다. selection은 0부터 시작하는 class index 또는 Hold를 나타내는 -1이며 confidence는 최대 확률이다. valid 최대 확률이 minimum 이상이면 Select를 생성하고, 나머지는 Hold를 생성한다. minimum은 0..1의 무차원 상수다. 동점은 class schema의 앞선 항목을 선택한다.

| 입력 상태 | 판정 값 | valid |
|---|---|---|
| 최대 확률 ≥ minimum | `[class_index, maximum]` | `[1,1]` |
| 최대 확률 < minimum | `[-1, maximum]` | `[1,1]` |
| 무효 특징·확률 | `[-1,0]`, InvalidInput | `[0,0]` |

runtime과 replay는 확률 합·클래스 범위·무효 payload를 검사한다. 기록 판본은 bci-run/0.6·bci-recording/0.6이며, 클래스 배열과 고유 value_kind를 저장한다. 기존 0.5·0.4·0.3·0.2·합성 0.1 재생도 지원한다. checkpoint는 모델을 포함한 계획 digest를 대조하고, 원천 위치·큐·격자를 복원한다.

## 구현과 검증

schema/learning.rs는 역할·계보·타입·고정 모델 계약, schema/selection.rs는 선택 증거·통합 계보·지표 계약, operators/learning.rs는 학습·선택·평가, operators/prediction.rs는 온라인 예측·판정, artifact/learning.rs는 codec과 게시, cli/learn.rs는 명령 조립을 소유한다. 기존 11개 패키지와 파일 깊이를 유지한다.

CLI 통합 시나리오 하나에서 한 번의 gradient 갱신을 독립 수식과 대조하고, Eval 변경에 대한 모델 불변성·분할 충돌·평가 분모·모델 bit 복원·확률·동점·Hold·기록 재생·checkpoint 연결을 확인한다. 수치 예제의 valid Eval 4개에서 confusion matrix는 `[2,0,0,2]`이며 balanced accuracy는 1이다.

후보 선택 통합 시나리오는 Tune 품질 제외·후보 순서 동점·Eval 변경에 대한 선택 불변성·탈락 후보 Fit 및 Tune과 Eval의 겹침·Eval 재배정·artifact 변조·CLI 결과·온라인 기록과 checkpoint 복원을 함께 확인한다. 선택 모델로 기록한 30개 레코드를 연속 실행과 4표본 이후 복원 실행에서 비교한다.
