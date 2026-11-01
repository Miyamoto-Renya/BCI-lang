# 시계, 시간 뷰와 창 완결

근거: [원문 시간 계약](../../bci_dsl_latex/sections/06_time.tex). 시간 계산의 구간은 닫힌 `[lo,hi]`, 창은 반열린 `[start,end)`를 사용한다.

## 시간 표현

`ClockDomain`은 ID, 단위, origin, counter 폭, wrap 규칙을 가진다. timestamp는 `(domain, epoch, offset)`이다. 저장 offset은 signed 64-bit ns, 격자 계산은 checked 128-bit 유리수 중간값을 사용한다. 범위 초과는 `E0407 TimeOverflow`로 처리한다.

균일 격자 `u_n=u0+(n−n0)/f`를 유리수로 계산한 뒤 구간 하한을 floor, 상한을 ceil하여 ns로 저장한다. 이 방식으로 30 kHz 같은 비정수 ns 간격의 누적 반올림 오차를 제어한다. 진단과 manifest는 격자의 정확한 분자·분모도 저장한다.

## mapping 레코드

```text
ClockMapping {
  id, from_domain, from_epoch, to_domain, to_epoch,
  u0, a_hat, b_hat, valid_interval,
  delta_a, delta_b, delta_r,
  available_at, evidence_id,
  evidence: Bound(assumptions) | Estimate(dataset, quantile, count)
}
```

`a_hat > delta_a >= 0`, `delta_b,delta_r >= 0`을 검사한다. `phi_hat(u)=b_hat+a_hat*(u−u0)`, `epsilon(u)=delta_b+abs(u−u0)*delta_a+delta_r`로 계산한다. 매핑 결과는 `[phi_hat−epsilon,phi_hat+epsilon]`이다. 입력 자체가 구간이면 전체 입력 구간의 hull을 사용한다.

composition은 중간 구간 전체가 다음 mapping의 valid_interval에 포함되는 경우에 성립한다. 오차는 `abs(a23)*epsilon12 + sup(epsilon23(z))`로 전파한다. 유효 구간 경계에서의 외삽 요청은 `E0402 MappingExpired`를 생성하고 새 mapping을 요구한다.

`Bound`는 명시한 가정 아래의 포함 상한이다. `Estimate`는 데이터셋·분위수·표본 수와 연결한다. hard 시간 조건은 Bound 증거를 요구한다. 관측 중심의 운영 모드는 Estimate를 사용하고 위반율·분포를 기록한다.

## 시간 뷰와 갱신

`time.align(stream, to: clock, mapping: artifact)`는 값·표본 인덱스를 유지하면서 대표 측정 구간을 변환한다. 출력에는 mapping ID와 새 time-view ID를 부여한다. 재표본화는 [05](05-operators.md)의 별도 격자 변환이다.

Live mapping은 해당 입력 처리 전에 available 상태여야 한다. mapping 갱신은 이전 뷰의 frontier를 보존할 수 있는 적용 경계와 새 epoch barrier를 정한다. 과거 구간을 새 보정으로 다시 해석하는 작업은 Record 실행에서 새 time view와 결과 artifact를 생성한다.

시계 wrap은 counter 폭과 연속 관측 증거를 이용해 같은 clock epoch의 확장 counter로 복원한다. wrap 횟수의 해가 여러 개면 index를 Unresolved로 두고 시간 guard를 실패시킨다. 장치 재시작은 새 clock epoch를 만든다.

## pending을 포함한 watermark

source c의 ingress 관측 상한과 측정 구간 하한 사이 지연 상한을 Lc로 정의한다. 현재 기준시각 관측을 `[s−,s+]`로 읽는다.

```text
w_ingress[c] = s− − Lc
p[c] = min(measurement.lo of accepted records awaiting join accounting)
       or +infinity when the pending set is empty
w_join = min over c of min(w_ingress[c], p[c])
```

pending에는 수신 큐, 디코딩 후 전송 중 블록, join 입력 큐, join 반영 대기 자료가 포함된다. 각 boundary는 자신이 수락한 sequence와 frontier를 함께 전달한다. 자료를 join 상태에 반영한 뒤 해당 pending 항목을 해제한다. fan-out 경로는 각 분기의 pending을 독립 관리한다.

기본 구현은 데이터와 Watermark/Gap/EndEpoch를 같은 순서 있는 mailbox에 넣는다. 제어용 예약 용량은 메모리 예산에 포함한다. 별도 제어 채널 구현은 `after_sequence` fence를 사용해 앞선 데이터의 반영을 확인한다.

watermark는 같은 epoch와 time view에서 단조 증가한다. `max(old,new)`로 단조화를 적용할 수 있는 조건은 lateness·mapping 가정이 계속 성립하는 경우다. 가정 위반은 `LateBeyondContract`를 발생시키고 현재 뷰를 fault 처리한다. 완료된 창의 수정은 새로운 Record 뷰의 결과로 표현한다.

## 창 소속과 완결

시간 창 W=[u,v)는 `w_join >= v`에서 완결한다. 대표 측정 구간 I=[l,h]에 대해 다음 규칙을 사용한다.

- `Definite`: `u <= l && h < v`.
- `Possible`: `h >= u && l < v`.
- Possible에만 속하는 항목에는 `BoundaryAmbiguous`를 부여한다. Possible 정책에서 겹치는 창의 중복 소속은 원천 ID로 추적한다.

시간 창의 anchor는 v, 표본수 창의 anchor는 마지막 표본의 측정 구간이다. 시간 창의 멤버 수는 실제 소속으로 결정한다. 정확히 N개의 표본을 요구하는 `Epoch<C,N>` 연산자는 표본수 창 또는 명시적 격자 투영을 입력으로 받는다.

사건 기준 구간은 marker의 선언한 대표 중심값 `t_e`를 기준으로 `[t_e+a,t_e+b)`를 구성하고, marker 오차를 창 경계 오차로 함께 전파한다. 보수적인 definite 소속은 모든 가능한 marker 시각에 대해 포함되는 표본을 선택한다. possible 소속은 가능한 marker 시각 중 하나에서 겹치는 표본을 선택한다. 완료에는 가장 늦은 오른쪽 경계 `marker.hi+b`를 사용한다.

source silence 동안에는 Lc 증거가 살아 있는 Timer가 진행을 생성한다. heartbeat 상실·장치 연결 손실은 Fault 또는 확정 Gap 구간을 생성한다. 복구 데이터는 새 availability를 가지며 과거 live 결과와 분리한 replay 뷰에 연결한다.

## 지연과 응답시간

ingress→join 처리 상한 Dc, timer 갱신 상한 G가 주어지면 `a_close <= v + max(Lc+Dc) + G`를 사용한다. 구간 timer의 하한 지연은 G에 포함한다. 실제 커널·출력 비용은 여기에 추가한다.

FIR 군지연, 창의 물리적 span, 입력 대기, 계산시간, 장치 command→edge 지연은 각각 별도 필드다. marker `[lm,hm]`부터 D 이내 응답의 충분조건은 `output_available_upper <= lm + D`다.

소프트웨어 명령의 예측 edge는 `command_interval + path_delay_interval`로 계산한다. 실제 edge 관측은 장치 입력 stream의 독립 레코드이며 command ID로 연결한다.

## 경계 예

현재 시각 하한 1.020 s, L=10 ms이면 ingress frontier는 1.010 s다. pending 레코드의 하한이 0.999 s이면 join frontier는 0.999 s다. `[0,1.000)` 창은 그 레코드를 반영한 후 frontier가 1.000 s에 도달할 때 완결한다. 관측 오차가 창 경계를 가로지르면 선택한 소속 정책과 ambiguity flag가 결과를 결정한다.
