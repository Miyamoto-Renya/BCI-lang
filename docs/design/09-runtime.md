# Rust 런타임과 유한 자원 실행

근거: [원문 실행 구조](../../bci_dsl_latex/sections/08_runtime.tex). 실행기는 검증된 계획의 actor·토큰·사건·소유권 관계를 수행한다.

현재 구현의 합성·CSV·재생 원천, 순차 커널·자원 계량·종료 기록은 [런타임 사용법](../runtime.md)과 [파일 입력](../inputs.md)에 정의한다. 아래 내용은 장치·병렬·SDF 실행을 포함한 전체 목표 계약이다.

## 실행 수명

세션은 `Created → Preparing → Ready → Running → Draining → Closed`로 이동한다. 각 상태는 시작 시각·원인 사건·plan digest를 기록한다. Preparing은 장치 조회, binding 완료, artifact 검증, buffer 예약, callback 등록을 수행한다. Ready는 모든 필수 obligation Pass와 시작 barrier 수락을 요구한다.

Running의 fault는 supervisor에 `Faulted(last_committed_sequence, cause)`를 전달한다. supervisor는 계획의 fault policy로 해당 branch 또는 세션을 Draining으로 이동한다. Draining은 신규 작업 수락 종료, 남은 의존 작업의 완료 또는 취소, EndEpoch, recorder commit, native 자원 해제를 수행한다.

stop 요청은 control event다. 종료 deadline에 도달하면 미완료 범위를 `AbortedRange`로 기록한다. SDK가 반환을 지연하는 경우 adapter profile의 timeout·격리 정책을 적용한다. 격리된 bridge process는 종료·재시작으로 새 acquisition epoch를 만든다.

## actor와 buffer 소유권

- Device actor는 native handle, 설정, callback과 획득 세대를 소유한다.
- Compute actor는 필터·창·예측의 가변 상태를 소유한다.
- Training actor는 optimizer·후보 모델을 소유한다.
- Clock actor는 mapping 관측과 활성화 사건을 생성한다.
- Recorder actor는 chunk와 commit 순서를 소유한다.
- Supervisor는 fault 전이와 resource 상태를 조정한다.

DataBlock은 값 buffer와 immutable metadata를 가진다. 논리 배열은 C×N이며 실제 layout은 channel stride·sample stride로 기술한다. kernel 요구 layout이 다르면 명시적 layout conversion을 계획한다. raw buffer는 adapter가 게시한 뒤 immutable 상태를 유지한다.

단일 소비자는 ownership move, 다중 소비자는 Arc와 pool lease를 사용한다. 공유 객체는 Send/Sync 계약을 충족한다. pool 반환 시점은 마지막 lease 종료다. 느린 branch가 보유하는 lease도 pool 최대 점유량 계산에 포함한다.

## 고정률 영역과 SDF

token은 간선별로 표본·고정 블록·epoch 중 하나를 선택하고 크기를 고정한다. startup prologue가 FIR warmup과 첫 창을 준비한다. steady 영역은 고정 생산·소비량을 가지며 event-rate 연산자와 명시적 경계로 연결한다.

간선 e에 생산 p_e, 소비 c_e, 초기 token d_e를 둔다. 양의 정수 반복벡터 nu는 `p_e*nu_src = c_e*nu_dst`를 만족한다. 유리수 해를 정수로 올린 후 최대공약수로 최소화한다.

발화열 omega의 접두 j에서 `m_e(j)=d_e+p_e*N_src(j)−c_e*N_dst(j)`다. verifier는 매 발화 전 입력 token≥c_e, 주기 마지막 m_e=d_e, buffer B_e≥max m_e를 검사한다. 순환 영역의 초기 token은 의미론의 지연값과 대응한다.

예를 들어 A가 2개 생산하고 B가 3개 소비하면 `nu=(3,2)`다. `A,A,B,A,B`의 점유는 `0,2,4,1,3,0`이며 capacity 4가 해당 순차 일정의 상한이다. `A,B,...`는 첫 B의 입력 부족으로 실패한다.

병렬 계획은 입력 소비·출력 예약·완료 게시를 구분한다. 발화 시작 전에 출력 슬롯과 scratch를 예약한다. 예약된 출력과 실행 중 보유 입력을 메모리 계량에 포함한다. 허용한 시작/완료 사건열의 모든 접두에서 토큰·capacity 조건을 검증한다. 이 검증의 입력은 thread 수와 허용 동시 실행 관계다.

## event actor와 역압력

event queue는 최대 사건 수·payload bytes·timer 밀도·burst를 선언한다. event actor의 발화는 입력 사건, 상태와 현재 timer를 이용하며 출력 수 상한을 registry에서 받는다. 시간 창은 최대 보관 시간·최대 입력률로 상태 크기를 계산한다.

| queue policy | 허용 조건 | 초과 처리 |
|---|---|---|
| Fault | 모든 경로, 기본값 | Overflow 원인과 누락 가능 범위 기록, 해당 경로 정지 |
| Block | 상류가 pause를 지원하고 대기 상한을 검증한 경로 | timeout 시 Fault |
| DropOldest | 표시·관측 등 누락 허용이 명시된 분기 | 가장 오래된 대기 data 제거, Gap 발생 |
| DropNewest | 최신 누락 허용을 선언한 분기 | 신규 data 범위를 Gap으로 기록 |

제어 사건은 별도 예약 용량과 sequence fence를 가진다. control 예약량 초과는 Fault로 처리한다. Drop 정책은 현재 처리 중인 block과 이미 게시한 frontier의 순서를 보존한다. Gap 수락 후 downstream이 quality/reset 규칙을 수행한다.

## 메모리 계산

독립 buffer 계획은 `M=sum(B_e*s_e)+sum(actor_working_bytes)+metadata_bytes`를 사용한다. actor_working_bytes에는 보유 중 입력·출력·state·scratch를 포함한다. pool 계획은 공유 allocation을 한 번 계량하고 최대 동시 lease에 따른 pool count를 산출한다.

metadata 예산에는 provenance node queue, watermark, gap, recorder staging, model 교체 중 두 버전의 동시 보유량이 들어간다. lineage는 chunk로 디스크에 저장하고 resident index/cache의 상한을 둔다. recorder의 처리율·디스크 공간도 resource monitor가 검사한다.

prepare는 plan의 총 예약량과 process budget을 비교한다. running 중 고정 영역은 예약 pool을 사용하고, 동적 영역은 bounded allocator와 초과 policy를 사용한다. 큐 byte 상한과 record 수 상한을 함께 검사하여 작은 사건의 과다 발생을 처리한다.

## 시간·처리량 계산

순차 주기 P에서 완료 상한은 `f[k,0]=kP`, `f[k,j]=max(f[k,j−1],dependency_available[k,j])+C_actor+overhead[j]`다. 각 발화의 deadline과 마지막 `f[k,M] <= (k+1)P`를 검사한다.

도착 곡선 `alpha(u)=b+r*u`, 서비스 곡선 `beta(u)=R*max(u−T,0)`, R>r를 사용하는 FIFO의 backlog 상한은 `b+r*T`, delay 상한은 `T+b/R`다. 값은 같은 payload 계량·완료 경계·기준 시계를 요구한다. 고정 블록 capacity는 `ceil(backlog/block_bytes)`로 올린다.

공유 CPU·디스크·네트워크의 경합은 해당 흐름에 배정한 R,T 또는 호출시간 증거에 반영한다. 보고서에는 환경 가정·evidence kind를 표시한다. 실제 호출시간과 queue peak는 trace의 관측값으로 별도 기록한다.

## 설정 변경과 재연결

설정 변경 transaction은 다음 상태를 가진다.

1. PrepareChange: 새 descriptor·BoundGraph·plan·상태 migration을 검증한다.
2. Quiesce: 이전 epoch의 ingress 경계를 확정하고 EndEpoch fence를 게시한다.
3. Drain: 기존 상태에 속한 데이터·창·명령 결과를 처리하고 부분 창 policy를 적용한다.
4. Install: `R_Z: J_old → J_new`를 만족하는 migration 또는 초기 상태를 적용한다.
5. Resume: 새 schema/acquisition/clock epoch 중 변경 대상의 ID를 생성하고 스트리밍한다.

prepare 실패는 기존 실행 상태를 유지한다. Quiesce 이후 장치 변경 실패는 Faulted로 이동한다. 복구는 실제 장치 설정 조회와 새 epoch로 수행한다. 재연결은 acquisition epoch를 갱신하고, 시계 reset은 clock epoch, 채널·단위·배열 변경은 schema epoch를 갱신한다.

## 모델·checkpoint·외부 명령

예측은 immutable active model을 작업 시작 시 획득한다. 교체 중 old/new 모델과 실행 중 참조를 memory plan에 포함한다. checkpoint는 actor state, source cursor, pending queue, frontier, epoch, RNG, active model을 하나의 barrier cut에서 저장한다.

재시작은 checkpoint cursor 이후 기록을 replay하여 상태를 복원한다. 외부 명령은 dispatch 전 command ID를 journal에 commit하고, receipt를 이어 기록한다. crash 복구에서 결과가 불명확한 비멱등 명령은 OutcomeUnknown으로 전환하고 device query를 수행한다.

callback 해제는 stop 응답, callback 등록 해제, in-flight callback=0 확인, native buffer 반환, handle close 순서를 따른다. 완료 시 Closed 사건과 최종 recorder commit ID를 반환한다.

현재 순차 실행의 동기 원천 1..64개와 `signal.mix` 계약은 [동기 다중 입력 구현](../multi-input.md)에 정의한다. 공통 격자 검증·품질 합성·원천 계보·전체 checkpoint 복원을 함께 연결한다.
