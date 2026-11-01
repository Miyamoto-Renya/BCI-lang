# 표준 연산자와 수치 계약

근거: [원문 연산자](../../bci_dsl_latex/sections/04_operators.tex). 이 문서의 서명은 registry 수준의 타입 변수 표기를 사용한다. 소스 호출은 [언어 문법](01-language.md)을 따른다.

## registry 항목

모든 연산자는 다음 필드를 제공한다.

```text
OperatorSpec {
  qualified_name, semantic_version, contract_digest,
  input_ports, output_ports, type_constraints, effect,
  parameter_schema, state_schema, state_codec_version,
  initialization, gap_policy, epoch_policy, tail_policy,
  support_rule, quality_rule, provenance_rule,
  token_rates, state_bytes_bound, scratch_bytes_bound,
  numeric_domain, numeric_tolerance, reference_kernel, execution_kernels
}
```

공통 기본값은 `gap: Reset`, `epoch: Reset`, `tail: Discard`, `quality: RequireAll`이다. 각 연산자는 기본값 적용 여부와 재정의 가능한 범위를 registry에 열거한다. stream을 받는 연산자는 첫 위치 인수를 입력으로 사용한다.

연산 결과는 값과 별도로 `ContractFailure`를 반환할 수 있다. 실패 레코드는 연산자·인수·원천 범위·정의역 조건·선택한 policy를 포함한다. 수치 검사를 통과한 refinement만 후속 연산자에 제공한다.

사용자 Rust 연산자는 [13](13-rust-extensions.md)의 pointwise 함수 또는 stateful API로 등록한다. 등록 매크로가 공통 contract profile과 사용자 설정을 병합해 위 OperatorSpec을 생성하고 각 필드의 출처를 보관한다. block wrapper의 자원량과 커널의 상태·scratch 자원량을 함께 실행 계획에 반영한다.

## 값 선택과 조건부 효과

`control.select(condition, when_true: a, when_false: b)`는 같은 tick의 bool 조건과 같은 타입의 두 값을 소비하고 선택한 값 하나를 출력한다. 두 branch 생산자의 실행·상태 갱신은 계획을 따르며, 출력 lineage는 조건과 수락한 입력들을 포함한다. 개별 커널의 조건부 계산은 Rust의 `if`·`match`로 작성한다.

장치 출력을 조건에 따라 수행하는 경우 결정 연산자가 Select/Hold 또는 명령 사건을 생성한다. Hold는 출력 sink의 장치 호출 수 0으로 처리하고, 명령 사건은 capability·dispatch·receipt 계약을 적용한다.

## 스키마와 단위 연산

| 호출 | 전제와 출력 | 상태·시간 |
|---|---|---|
| `signal.select(x, channels: ids)` | 중복 없는 입력 ID 목록, 출력 순서=ids | 무상태, support 유지 |
| `signal.reorder(x, channels: ids)` | 전체 채널의 순열 | 무상태 |
| `signal.convert_unit(x, to: "V")` | 같은 차원·측정량, `y=(s_in*x+o_in−o_out)/s_out` | 수치 커널 반올림 기록 |
| `signal.rereference(x, weights: w)` | 동일 전압·단위, 길이 C, sum(w)=1 | `A=I−1wᵀ`, 새 reference ID |
| `signal.normalize(x, model: stats)` | stats의 입력 schema 일치 | fit 계보와 model 가용 시각 추가 |

재기준화의 가중치 합은 정적 유리수면 정확히 검사하고 float artifact면 명시한 허용오차·정규화 규칙으로 검사한다. valid mask는 각 출력에 실제로 기여하는 입력들의 논리곱이다. 예를 들어 전체 채널 평균 재기준화는 모든 입력 채널의 유효성을 요구한다.

## FIR, IIR와 재표본화

`signal.fir(x, coeffs: artifact, warmup: Warmup.Full)`는 `y[n]=Σ h[l]x[n−l]`을 계산한다. coeff 길이 L≥1, 유한 계수, 채널별 L−1 상태를 요구한다. 초기 상태 값은 0, 초기 origin은 Synthetic이다. Full 정책에서 n<L−1은 invalid다. 출력 index는 입력 n을 유지한다.

출력 anchor는 t[n], support는 입력 n−L+1…n의 구간 합집합이다. 대칭 계수의 군지연은 `(L−1)/(2f)`다. gap 이후 상태를 초기화하고 L−1개 warmup을 다시 기록한다. 모델 입력 schema는 filter digest를 feature derivation에 포함한다.

`signal.iir(x, realization: artifact)`는 `(A,B,C,D)` 또는 고정 SOS 실현을 사용한다. artifact는 상태 초기값, 계수 정밀도, 안정성 근거, overflow policy를 가진다. `rho(A)<1` 조건은 해당 실현의 수치 검증 의무다. 무한 impulse support는 이전 상태 lineage와 연결한다. Float 비유한 결과는 `NumericFault`로 처리한다.

`signal.resample(x, up: p, down: q, filter: h, phase: phi)`는 필터 설계에서 gcd(p,q)=1로 정규화한 비율을 받고 `f_out=p*f_in/q`를 계산한다. 입력 index 격자와 출력 격자를 정확한 유리수로 유지한다. `y[m]=Σ x[n]h[mq−np]`의 유효 h 범위로 의존 표본을 결정한다. 필터 artifact는 passband, stopband, anti-alias 범위, 위상 anchor와 경계값을 가진다. 미래 표본을 요구하는 support는 lookahead와 출력 대기 조건에 포함한다.

## 창

```text
signal.window(x, size: N, hop: H,
              start: Index(n0) | WindowStart.FirstValid,
              anchor: Anchor.End,
              quality: Quality.RequireAll,
              tail: Tail.Discard) -> Stream<Epoch<D,C,N>, ...>
```

N,H>0이며 시작 index `n0+kH`부터 N개 격자를 선택한다. 출력률은 `f/H`, 마지막 표본 index는 `n0+kH+N−1`이다. `FirstValid`는 epoch 시작 후 최초의 연속 N개 valid 표본을 찾아 그 첫 index를 위상으로 확정한다. gap reset은 새 window generation에서 위상을 다시 정한다. 출력 명목 간격은 세대 내에서 유지한다.

H<N이면 ring buffer가 중첩 자료를 재사용한다. H>N이면 창 사이의 표본을 순서대로 소비하고 사용 범위를 lineage에 표시한다. SDF 계획에서 warmup·첫 창 생성은 prologue, 정상 구간은 H 입력/1 epoch의 발화로 표현한다. 내부 보관 표본 수는 N과 사용 중 출력 lifetime으로 계산한다.

`time.window`와 `time.join`은 시간구간 소속과 watermark로 닫는 event-rate 연산자다. [04](04-time.md)의 경계 규칙을 사용한다. 출력은 `WindowBatch` 또는 feature의 곱이며 정확 N의 epoch가 필요한 소비자는 `signal.project_grid`로 격자·보간 정책을 지정한다.

## PSD와 전력 특징

`signal.psd(epoch, taper: taper, normalization: Psd.OneSidedDensity)`에서 N≥1, `Σw²>0`, real input을 요구한다. output bin k=0…floor(N/2), frequency `k*f/N`다.

```text
DFT[k] = Σ w[n]*x[n]*exp(−2*pi*i*k*n/N)
P[k] = eta[k]*abs(DFT[k])² / (f*Σw²)
eta = 1 at DC and the even-N Nyquist bin; 2 elsewhere
```

입력 단위가 u이면 출력 단위는 u²/Hz다. `signal.bandpower(psd, band: #{low: a, high: b})`는 반열린 `[a,b)` bin을 합산하고 `f/N`을 곱한다. Nyquist bin을 포함하는 선택은 `include_high: true`로 명시한다. 유효 bin 수가 0이면 정의역 오류다.

`signal.log_power(power, floor: E0, reference: Eref)`는 `log((E+E0)/Eref)`다. E0,Eref>0, 단위 u²를 요구한다. 출력은 무차원이고 schema에 대역·정규화·기준 전력을 저장한다.

## 공분산과 공간 특징

`signal.covariance(epoch)`는 N≥2, 동일 측정량·단위 채널을 요구하고 `X H_N Xᵀ/(N−1)`을 계산한다. 출력 타입은 `Cov<D,C>`다. 비대칭 부동소수 오차는 `(S+Sᵀ)/2`로 정규화하고 해당 커널 규칙을 manifest에 기록한다.

`signal.regularize(cov, shrinkage: lambda, floor: sigma0_sq)`는 `0<lambda<=1`, `sigma0_sq>0`을 요구한다. `s²=max(trace(cov)/C,sigma0_sq)`, `Sigma=(1−lambda)*cov+lambda*s²*I`를 계산한다. 결과의 Cholesky 검사를 통과하면 `SPD<D,C>` refinement를 부여한다. 실패 시 `NumericFault`를 생성한다. floor의 단위는 입력 공분산 단위다.

`learn.csp_fit`은 두 클래스의 SPD 공분산으로 generalized eigenproblem을 풀고 선택한 벡터·정렬 규칙·부호 정규화·class 순서를 저장한다. 반복 고유값의 basis 선택은 kernel digest와 결과 artifact로 고정한다. `signal.csp_features`는 `e_j=w_jᵀ Sigma w_j`, `log(e_j/Σe)`를 출력한다.

`signal.tangent(cov, reference: model)`은 reference의 SPD R, 단위와 schema를 검사한다. `Z=log(R^-1/2 Sigma R^-1/2)`를 구하고 `svec`을 출력한다. svec 순서는 행별 상삼각 `(0,0),(0,1),…,(1,1),…`이며 비대각 원소에 sqrt(2)를 곱한다. feature 수는 C(C+1)/2다. basis의 digest는 feature schema에 포함한다.

## 광학 변환

`optical.density(intensity, baseline: model)`은 양의 I와 I0에 대해 `−ln(I/I0)`를 계산한다. source·detector·wavelength 대응과 baseline 학습 계보를 검사한다. 포화·0 이하 값은 해당 채널 invalid와 DomainViolation 원인을 생성한다.

`optical.concentration(od, path_model: artifact)`는 파장·거리·차등 광경로 인자·흡광 계수·로그 밑·출력 단위를 artifact에 고정한다. `W≻0`, `rank(A)=성분 수`를 확인하고 weighted least squares를 QR 또는 SVD로 푼다. 원문의 `(AᵀWA)^-1 AᵀW`는 참조 수학적 계약이다. 구현 커널은 선택한 분해와 rank tolerance를 기록한다.

기기에서 이미 제공한 농도 변화량은 명목 schema의 quantity로 식별한다. 해당 입력을 optical.density에 연결하면 quantity mismatch를 진단한다.

## 결정과 수치 검증

`decision.threshold(probabilities, minimum: eta)`는 0≤eta≤1을 요구한다. 최대 확률이 eta 이상이면 Select, 나머지는 Hold를 생성한다. 동점은 class schema 순서로 결정한다. probability는 유한 성분, 비음수, 합의 허용오차를 검사하고 명시한 normalize 정책을 적용한다.

float kernel은 입력 영역·norm·dtype별 `||actual−reference|| <= abs_tol + rel_tol*||reference||`를 제공한다. 정수 decoder와 enum·모델 버전·사건 순서는 exact 관계로 비교한다. NaN·무한대·subnormal·rounding policy를 kernel profile에 명시한다. kernel 선택은 기능과 numeric contract를 함께 만족하는 후보에서 수행한다.

현재 순차 실행의 동기 원천 1..64개와 `signal.mix` 계약은 [동기 다중 입력 구현](../multi-input.md)에 정의한다. 공통 격자 검증·품질 합성·원천 계보·전체 checkpoint 복원을 함께 연결한다.

현재 `signal.select`·`signal.reorder`·`signal.rereference`의 Vec·Epoch 실행, 정확한 계수·기준 ID와 품질 규칙은 [채널 변환 구현](../channels.md)에 정의한다.

현재 PSD·대역 전력·로그 특징의 직접 DFT 실행, Spectrum 타입·단측 정규화·주파수 경계·재생 계약은 [주파수 특징 구현](../spectral.md)에 정의한다.

현재 고정 SOS IIR의 유리수 정규화·Schur 안정성·채널별 초기화·warmup·checkpoint 계약은 [IIR 구현](../iir.md)에 정의한다.

현재 실행 profile의 인과 계수·격자·버퍼·초기화 계약은 [재표본화 구현](../resampling.md)에 정의한다.
