# PSD·대역 전력·로그 특징

`signal.psd`는 Epoch를 단측 전력 스펙트럼 밀도로 변환한다. `signal.bandpower`는 주파수 대역의 전력을 계산하고, `signal.log_power`는 같은 단위의 안정화 전력과 기준 전력을 적용해 무차원 특징을 만든다. 세 연산자는 현재 창 하나를 받아 같은 시각에 출력 하나를 게시한다.

```bci
let spectrum = epochs |> signal.psd(
    taper: Taper.Hann, normalization: Psd.OneSidedDensity
);
let alpha = spectrum |> signal.bandpower(band: #{low: 8[Hz], high: 13[Hz]});
let features = alpha |> signal.log_power(floor: 1e-12[uV^2], reference: 1[uV^2]);
```

## 타입과 주파수 축

| 연산 | 입력 | 출력 | 전압 입력의 단위 예 |
|---|---|---|---|
| psd | `Epoch<f64,C,N,f>` | `Spectrum<f64,C,N,f>` | uV → uV²/Hz |
| bandpower | `Spectrum<f64,C,N,f>` | `Vec<f64,C>` | uV²/Hz → uV² |
| log_power | 대역 전력 `Vec<f64,C>` | `Vec<f64,C>` | uV² → 1 |

Spectrum의 N은 원래 DFT 표본 수다. 저장 shape는 `[C, floor(N/2)+1]`이고 값은 채널 우선으로 배치한다. k번째 bin의 주파수는 `k*f/N`, bin 폭은 `f/N`이다. 홀수 N의 마지막 bin은 Nyquist보다 낮다. 내부 표본률 f와 창의 출력 발생률은 각각 보존한다. 타입 인수에 표본률을 쓰면 `Spectrum<f64,2,250,250[Hz]>`처럼 표현한다.

채널 ID·측정 기준·순서·clock·발생 규칙·품질 어휘를 보존한다. 측정량 문자열에는 PSD의 원래 측정량·N·f·taper·정규화, 대역 경계·상한 포함 정책, 로그 floor·reference를 차례로 기록한다. 다른 대역이나 taper의 특징은 연결 시 스키마 비교 대상이다. 채널·단위·시계 변환은 PSD 입력에 연결하며, Spectrum 소비 연산자는 bandpower와 recording이다.

## PSD 계산

현재 `direct-dft-table/0.1` 커널은 N=1..4096을 처리한다. `Taper.Rectangular`의 계수는 모두 1이다. `Taper.Hann`은 주기적 Hann이며 `w[n]=(1−cos(2πn/N))/2`, N≥2를 요구한다. 정규화 기본값과 명시 값은 `Psd.OneSidedDensity`다. 평균을 포함한 입력 값을 창 함수와 곱해 변환하므로 상수 성분은 DC bin에 기여한다.

```text
X[k] = Σ w[n] x[n] exp(−2πikn/N)
P[k] = eta[k] (Re(X[k])² + Im(X[k])²) / (f Σw[n]²)
eta[k] = 1  (DC, 또는 짝수 N의 Nyquist)
         2  (그 외 단측 bin)
```

prepare는 길이 N의 창·cos·sin 배열을 예약한다. 주파수별 위상은 `(k*n) mod N`으로 이 배열을 조회한다. DC와 짝수 N의 반주기에는 정확한 cos=±1·sin=0을 사용한다. 각 bin은 입력 표본 순서로 f64 합산한다. 유효 출력이 비유한 값이면 수치 guard가 E0903을 반환한다.

창의 해당 채널 mask가 0이면 그 채널의 전체 PSD bin을 0·invalid로 게시한다. 다른 유효 채널은 계산을 진행한다. 시각·sequence·generation·Gap·원천 범위·origin과 기존 flags는 입력에서 전파하고, 무효 출력에 InvalidInput을 표시한다.

## 대역과 로그 전력

대역은 `0 <= low < high <= f/2`를 요구한다. 경계 단위는 Hz 또는 같은 차원의 배율 단위이며, 컴파일러가 정확한 Hz 유리수로 변환한다. 기본 선택은 `[low, high)`다. `include_high: true`는 high와 정확히 같은 주파수의 bin도 포함한다. 포함 bin이 0개인 대역은 구성 오류다.

```bci
let total = spectrum |> signal.bandpower(
    band: #{low: 0[Hz], high: 125[Hz]}, include_high: true
);
```

위 예는 250 Hz 입력의 DC부터 Nyquist까지 합산한다. bandpower는 선택한 밀도의 합에 bin 폭 `f/N`을 곱한다. 주파수 경계의 판정은 정수 bin과 유리수 비교로 수행한다. 경계 사이의 부분 bin 가중치는 이 실행 프로필에서 0 또는 1로 정한다.

log_power는 자연로그 `ln((E+floor)/reference)`를 계산한다. 입력은 bandpower의 전력 벡터이며, floor·reference는 입력과 같은 단위·배율의 양수다. 대역 전력과 로그 커널은 유효 입력의 음수 전력을 E0903으로 진단한다. 무효 채널은 0 payload와 기존 mask로 전파한다. 원시 0 전력도 양의 floor를 더해 유한 로그 값을 만든다.

## 기록·재생·checkpoint

현재 manifest와 JSON Lines 판본은 각각 `bci-run/0.6`, `bci-recording/0.6`다. Spectrum 출력 기술자는 DFT 표본 수, 내부 표본률, bin 수·폭과 OneSidedDensity 정규화를 보관한다. replay는 이 필드의 상호 일치와 PSD 값의 유한성·비음수 조건을 검사한다. 이전 0.5·0.4·0.3·0.2와 합성 기록 0.1도 읽는다.

스펙트럼을 직접 기록한 뒤 `source.replay`로 가져와 다른 대역 특징을 계산할 수 있다. 원래 주파수 축·품질·시각·원천 범위가 재생 타입과 레코드에 보존된다. 실행 manifest에는 세 커널의 정확한 구성과 계산 profile을 기록한다.

세 커널의 호출 간 상태 payload는 0 byte다. 준비된 창·삼각함수·bin 경계는 복원 시 계획에서 재생성한다. upstream 창과 FIR의 ring·품질·origin·격자는 기존 checkpoint로 복원한다. taper·대역·floor·reference 변경은 계획 digest를 바꾸며 새 실행 계보를 만든다.

## 예제와 확인 결과

[spectral.bci](../examples/spectral.bci)는 합성 EEG → 평균 재기준화 → 250표본 창 → Hann PSD → 8..13 Hz 전력 → 로그 특징을 실행한다. 창 hop은 125표본이며, 1000개 입력에서 PSD·대역 전력·로그 특징 각각 7개를 게시한다. 원시 입력과 함께 총 1021개 레코드를 기록한다.

```sh
cargo run --offline --locked -- run examples/spectral.bci --output recordings/spectral-full
cargo run --offline --locked -- run examples/spectral.bci --samples 400 --output recordings/spectral-head --checkpoint-out recordings/spectral-at-400.json
cargo run --offline --locked -- run examples/spectral.bci --samples 600 --resume recordings/spectral-at-400.json --output recordings/spectral-tail
```

합성 입력을 재개할 때 `--samples`는 이번 호출에서 추가로 읽을 수다. 위 두 구간의 합은 1000입력이다.

[통합 검사](../crates/cli/pipeline.rs)의 검사 함수 하나에 다음 근거를 모았다. DC·Nyquist·단일 주파수·홀수 길이·Hann·N=1의 닫힌 식과 커널 값을 절대 오차 1e-12로 비교한다. CSV의 Gap·무효 채널을 포함한 8표본 창에서는 전체 대역 전력과 시간 영역의 창 가중 에너지를 1e-10으로 비교한다. 이 CSV 흐름의 기록 31개와 창 준비 중 checkpoint 재개의 결과, Spectrum 포트 재생 결과도 비교한다.

## 자원과 유지보수

PSD는 고정 배열 3개에 24N byte와 출력 `8*C*(floor(N/2)+1)+C` byte를 사용한다. 실제 예약량에는 Frame과 기존 커널 제어 구조가 더해진다. 대역·로그 커널은 고정 크기 구성과 출력 슬롯을 사용한다. 배열 예약은 prepare에서 수행하며 처리 루프는 기존 배열을 재사용한다.

PSD의 레코드당 연산량은 O(CN²), 대역 합은 O(CB), 로그 특징은 O(C)다. B는 선택한 bin 수다. 직접 DFT 프로필은 N 상한 4096을 계획·독립 verifier·재생 타입에서 검사한다. FFT 가속은 같은 타입·정규화 계약과 별도 수치 커널 정체성으로 연결할 수 있다.

| 파일 | 책임 |
|---|---|
| schema/spectral.rs·types.rs | Spectrum·taper·대역·단위·특징 스키마 |
| compiler/operators.rs·typing.rs·plan.rs | 호출·타입 인수·단위 검사와 구체 커널 계획 |
| verify/sequential.rs | shape·주파수·출력 스키마·예약량 독립 검증 |
| operators/spectral.rs | DFT·밀도 정규화·대역 합·로그 |
| artifact/stream_codec.rs·replay.rs·recording.rs | 주파수 축 codec·정의역 검사·기록 |

11개 패키지와 `crates/<책임>/<파일>.rs` 구조를 유지한다.
