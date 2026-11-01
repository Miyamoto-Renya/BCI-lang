# 계층별 평가와 cluster bootstrap

`learn evaluate --protocol`은 고정 모델의 Eval 예측을 시행·세션·피험자 단위로 집계하고, 지정한 독립 단위로 bootstrap 구간을 계산한다. `learn fit`과 `learn select`의 최종 Eval에도 같은 옵션을 적용한다. 후보 선택은 기존 Tune balanced accuracy 계약을 사용하고, 평가 protocol은 선택된 모델의 Eval 보고서에 적용한다.

## 실행

```sh
cargo build -p bci-cli --offline --locked
mkdir -p recordings
./target/debug/bci learn evaluate examples/learning.model.json \
  examples/evaluation.dataset.json \
  --protocol examples/evaluation.protocol.json \
  --output recordings/grouped-evaluation.json
```

출력 파일의 부모 디렉터리를 준비한 뒤 새 파일 경로를 지정한다. protocol 경로와 출력 경로는 현재 작업 디렉터리를 기준으로 해석한다. dataset의 schema 경로는 dataset 파일을 기준으로 해석한다.

[evaluation.dataset.json](../examples/evaluation.dataset.json)은 Eval 14개를 가진다. RequireAll에 따라 12개를 포함하고 2개를 제외한다. 피험자 A는 10개 중 6개를 맞추고, 피험자 B는 2개를 오분류한다. 피험자 C의 표본 2개는 품질 제외로 기록한다.

| 계산 방식 | balanced accuracy |
|---|---:|
| Sample | 0.5 |
| Subject | 0.3 |
| Trial → Session → Subject | 0.25 |

[evaluation.protocol.json](../examples/evaluation.protocol.json)은 마지막 집계 방식과 피험자 bootstrap 1000회·seed 42·confidence 0.95를 지정한다. 이 입력의 balanced accuracy percentile 구간은 `[0, 0.5]`다.

## Protocol

```json
{
  "format": "bci-evaluation-protocol/0.1",
  "aggregation": ["Trial", "Session", "Subject"],
  "weighting": "EqualChildMass",
  "quality": "RequireAll",
  "bootstrap": {
    "unit": "Subject",
    "iterations": 1000,
    "confidence": 0.95,
    "seed": 42,
    "rng": "SplitMix64/0.1",
    "interval": "PercentileLinear",
    "undefined": "Exclude"
  }
}
```

aggregation은 `Trial → Session → Subject`의 부분 수열이다. `[]`는 Sample, `["Subject"]`는 피험자별 표본 분포를 동일 가중한 결과다. bootstrap을 `null`로 지정하면 집계 결과를 저장한다. 반복 수는 100..10000, confidence는 (0,1), seed는 u64 범위를 사용한다. 각 고정 계약 필드는 codec에서 검증한다.

dataset loader는 subject/session/trial의 전체 경로를 평가 catalog에 보존한다. 분할은 기존 group_by의 접두사를 사용한다. 예를 들어 group_by=subject여도 행에 세션·시행 key를 제공하면 세 계층을 모두 집계할 수 있다. 요청한 단위까지의 key는 품질 제외 행에도 제공해야 한다. 세션·시행 이름이 다른 피험자에서 같아도 전체 경로가 그룹을 구분한다.

## 집계 수식과 분모

각 valid 표본은 `(실제 label, 예측 class)` 위치에 1을 둔 K×K confusion을 가진다. 요청한 단계마다 그룹 안의 valid 자식 confusion을 동일 가중 평균한다.

```text
M(group) = Σ M(valid child) / valid child 수
M(final) = Σ M(valid final group) / valid final group 수
accuracy = trace(M(final))
recall[k] = M(final)[k,k] / Σ_j M(final)[k,j]
balanced_accuracy = Σ_k recall[k] / K
```

첫 단계의 자식은 표본이며, 후속 단계의 자식은 바로 앞 단계의 그룹이다. Trial → Session → Subject에서는 시행마다 표본을 평균하고, 세션마다 시행을 평균하고, 피험자마다 세션을 평균한다. 마지막으로 valid 피험자들의 분포를 평균한다. 그룹의 원래 표본 수는 감사 정보로 보존하고, 후속 단계의 가중치는 valid 자식 수로 계산한다.

보고서의 `children`은 실제 평균에 참여한 자식 수, `included`와 `excluded`는 원래 Eval 표본 수다. 모든 표본이 품질 제외된 그룹은 children=0, confusion=0, 지표=null로 저장한다. 상위 집계는 valid 자식을 대상으로 계산한다.

하나의 시행처럼 일부 클래스만 가진 그룹에서는 balanced accuracy를 null로 저장하고 `missing_classes`에 분모가 0인 클래스 index를 남긴다. 상위 그룹은 자식의 confusion을 합친 후 해당 단계의 클래스별 분모를 계산한다. 전체 Eval의 클래스별 valid 표본 분모가 0이면 E0704 MetricUndefined를 반환한다.

## 재표집과 구간

bootstrap 단위는 분할의 독립 단위 및 최종 집계 단위와 같거나 상위여야 한다. group_by=subject이면 Subject, group_by=session이면 Session 또는 Subject를 재표집할 수 있다. 최종 집계가 Subject인 경우 재표집 단위도 Subject를 요구한다.

valid 자료를 가진 cluster G개를 전체 key의 사전식 순서로 정렬한다. 각 반복은 이 목록에서 G개를 복원 추출한다. 선택된 cluster의 전체 최종 집계 단위를 함께 복제하고, 원래 집계 규칙으로 지표를 다시 계산한다. Sample 집계에서는 cluster의 모든 valid 표본을 함께 복제한다. 품질 제외 행만 가진 cluster는 catalog에 남고, 추출 목록은 valid cluster로 구성한다.

난수 상태는 seed에서 시작하는 SplitMix64/0.1이다. 64비트 정수에서 cluster index로 변환할 때 rejection sampling을 적용한다. cluster 순서·반복 수·seed·집계 순서를 고정하면 동일한 반복 결과를 생성한다.

재표집 결과의 클래스 분모가 0이면 해당 반복의 balanced accuracy는 null이다. accuracy는 그 반복의 전체 정답률을 기록한다. `Exclude` 정책은 정의 가능한 balanced accuracy 반복으로 구간을 계산하고, 정의·결측 반복 수를 함께 저장한다. 따라서 해당 구간은 클래스 분모가 양수인 재표집 분포의 percentile 구간이다.

정렬한 지표 값 m개에서 percentile p의 위치는 `h=(m−1)p`다. floor(h)와 ceil(h)의 값을 선형 보간하고, 양측 p는 `(1−confidence)/2`, `(1+confidence)/2`로 정한다. 각 지표의 구간은 정의 가능한 반복 2개 이상을 요구한다. 추출 목록도 valid 독립 cluster 2개 이상을 요구한다.

## Artifact와 실행 경계

protocol을 지정한 보고서는 `bci-evaluation/0.2`다. 최상위 confusion·accuracy·balanced_accuracy는 기존 Sample 결과를 보존한다. 요청한 집계 결과는 `analysis.estimate`, 단계별 그룹은 `analysis.groups`, 재표집 결과는 `analysis.bootstrap`에 둔다.

analysis는 protocol 원문과 protocol_id, valid Eval 값·라벨·원천의 eval_digest, 전체 Eval 행의 계층·품질 포함 여부·원천 계보를 저장한다. bootstrap 결과는 정렬한 cluster 목록과 최종 집계 단위 수, 반복별 두 지표, 지표별 percentile 구간과 정의·결측 횟수를 저장한다.

report_id는 `bci-evaluation-report/0.2`와 NUL byte 뒤에 report_id를 제외한 보고서의 serde_json compact bytes를 연결한 SHA-256이다. 같은 입력·모델·protocol의 실행 결과를 이 ID로 비교할 수 있다. protocol 생략 시 기존 `bci-evaluation/0.1` Sample 보고서를 생성한다. 모델 파일은 기존 `bci-model/0.2`와 기존 의미 digest를 사용한다.

`learn fit --protocol`은 Fit/Eval 데이터의 평가에 적용한다. Tune 후보를 학습할 때 평가 protocol은 `learn select --protocol`에 지정한다. `learn select`는 Tune 승자를 확정한 뒤 지정 protocol으로 Eval을 계산한다. 분할·계층·지표·자원 검사 실패는 결과 파일 게시 전에 반환한다.

집계 histogram 생성량의 합계는 최대 1000000개 성분이다. bootstrap의 `cluster 수 × K × K × iterations` 상한은 200000000이다. 각 JSON 파일은 읽기·쓰기 모두 16 MiB 상한을 적용한다. 계산은 기존 원천·그룹 겹침 검사와 RequireAll 품질 정책을 공유한다.

## 코드 책임과 검증

schema/evaluation.rs는 protocol·보고서 계약, operators/evaluation.rs는 계층 집계·난수·재표집, artifact/evaluation.rs는 protocol codec과 보고서 구조를 소유한다. CLI는 기존 learn 명령에서 이 구성 요소를 조립한다. 새 Rust 파일은 기존 패키지의 형제 파일로 배치한다.

통합 시나리오 하나에서 불균등 표본 수의 집계를 독립 계산값과 대조하고, seed 재현성·결측 클래스 반복·품질 제외 그룹·계층 key·분할 충돌·protocol 검증·fit/select/evaluate의 파일 출력을 확인한다. 선택 모델 ID는 최종 평가 protocol을 적용한 흐름과 기존 선택 흐름에서 대조한다.
