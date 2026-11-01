# 장치 프로필, 바인딩과 어댑터

근거: [원문 하드웨어 계약](../../bci_dsl_latex/sections/05_hardware.tex), [A01–A18 프로필](../../bci_dsl_latex/sections/appendix_profiles.tex). 아래 장치 목록은 원문 판본의 프로필 분류를 상세 설계의 시험 대상으로 연결한다. 실행별 지원 판정은 고정한 버전·구성·조회 증거에서 산출한다.

## 프로필 구조

```text
HardwareProfile {
  profile_id, profile_version,
  model, hardware_revision, firmware,
  receiver, route, acquisition_version, sdk_version, os_arch,
  configuration_schema, valid_configurations,
  capabilities_by_configuration, concurrency_constraints,
  descriptors, timestamp_contract, decoder_contract,
  access_requirements, evidence_refs
}
```

route는 `DirectProtocol`, `DirectSdk`, `AcquisitionHost`, `Recording`이다. profile template의 미정 필드는 prepare가 값을 조회하는 required field다. profile instance는 실제 조회값·evidence hash를 포함한다. 프로필 문서와 실제 장치의 불일치는 `E0602 ProfileMismatch`로 판정한다.

configuration은 채널, ADC rate, 장치 방출률, 어댑터 출력률, gain, reference, filter, event I/O mode를 분리한다. 최대 표본률과 실제 선택 가능한 이산 sampling mode를 각각 저장한다. 같은 명칭의 모델도 route와 SDK 버전이 다르면 별도 instance다.

## 기능과 바인딩

capability는 `AcquireElectrical`, `AcquireOptical`, `AcquireAuxiliary`, `Configure`, `MeasureImpedance`, `ReceiveEvent`, `EmitEvent`, `InsertMarker`, `MapClock`, `Stimulate`다. capability의 매개변수에는 지원 채널·범위·시각 anchor·동시 사용 조건을 둔다.

바인딩은 원문의 다음 조건을 평가한다.

```text
theta in Omega(H)
program.required_capabilities subset capabilities(H, theta)
SAT(device_constraints AND environment)
device_constraints AND environment entails program_contracts
```

제약 증거의 필드 소유자는 `Document`, `DeviceQuery`, `AdapterCheck`, `ExecutionEvidence`다. 각 절은 Pass/Fail/Pending을 반환한다. contradictory 설정은 Fail, 조회가 필요한 필드는 Pending이다. 실행 시작 시 Pending 항목은 required field와 조회 방법을 포함한 `E0603 BindingPending`으로 보고한다.

문서가 최대값만 제공하는 경우 실제 선택률은 DeviceQuery로 확정한다. 유효 configuration과 capability의 조합을 함께 검사한다. 예를 들어 impedance 모드와 EEG acquisition의 동시 사용은 해당 장치의 concurrency predicate를 따른다.

## 어댑터 인터페이스

다음은 Rust 호스트 API의 설계 서명이다. 구체 SDK 타입은 각 어댑터 crate가 소유한다.

```rust
trait DeviceAdapter {
    fn probe(&mut self, request: ProbeRequest) -> Result<Descriptor, DeviceFault>;
    fn prepare(&mut self, config: ValidatedConfig) -> Result<PreparedDevice, DeviceFault>;
    fn start(&mut self, epoch: AcquisitionEpoch) -> Result<(), DeviceFault>;
    fn poll(&mut self, budget: PollBudget) -> Result<Vec<IngressEvent>, DeviceFault>;
    fn command(&mut self, cmd: ValidatedCommand) -> Result<CommandReceipt, DeviceFault>;
    fn stop(&mut self) -> Result<StopReceipt, DeviceFault>;
    fn close(&mut self) -> Result<(), DeviceFault>;
}
```

각 method는 단일 device actor가 호출한다. PollBudget은 최대 event 수·byte 수·호출 시간 조건을 가진다. callback SDK는 고정 크기 ingress ring에 바이트 또는 소유 buffer handle을 전달하고 actor가 디코딩한다. callback의 재진입성과 SDK thread affinity는 adapter contract에 명시한다.

FFI boundary는 포인터·길이·정렬·endianness·stride·버퍼 재사용 시각을 검증한다. SDK가 반환 직후 메모리를 재사용하면 actor가 소유 pool로 복사한다. release callback을 제공하는 SDK는 lifetime token을 붙여 immutable block으로 게시한다.

## 디코딩과 schema 구성

decoder 입력은 raw bytes, 수신 시각, connection epoch, profile instance다. 출력은 하나 이상의 역할별 stream과 packet provenance다. EEG·IMU·trigger·timestamp 행은 각 stream으로 분리한다. 원래 row/byte 범위와 변환식은 lineage에 남긴다.

packet counter와 sample index를 구분한다. counter wrap은 프로필 폭과 연속성으로 판정한다. 재정렬 허용 범위 밖의 counter 변화는 Gap 또는 DecoderResync를 생성한다. index 근거가 부족하면 Unresolved로 표현하고 연속 표본 index를 요구하는 소비자의 guard를 실패시킨다.

설정 응답에서 채널 배열·gain·codec을 받아 schema digest를 만든다. 단위 변환 전 raw integer 표현과 환산 metadata를 보존한다. raw payload를 보관할지 여부는 recording policy에 지정한다.

## 원문 프로필의 어댑터 요구

| ID | 대상·경로 | 필수 검증 지점 |
|---|---|---|
| A01 | Cyton / serial | 33-byte frame, signed 24-bit, counter, aux mode, gain |
| A02 | Cyton+Daisy / serial | bank 교대 위상, 2표본 평균, ADC/output rate |
| A03 | Ganglion / BLE | firmware별 codec, packet ID 영역, 압축 상태 reset |
| A04 | actiCHamp / SDK | module 수와 sampling mode 조합, AUX, SDK 버전 |
| A05 | Brain Products / RDA | Recorder 세션, channel descriptor, marker layout |
| A06 | Brain Products / LSL | connector 버전, reference, timestamp boundary |
| A07 | Unicorn / C API | active channel, scan stride, counter/validation 채널 |
| A08 | g.USBamp / g.NEEDaccess | monopolar/bipolar mode, DSP filter, server 설정 |
| A09 | ANT eego / SDK·BrainFlow | 모델, SDK package, 채널 map, timestamp origin |
| A10 | BioSemi ActiveTwo/2.5 | speedmode별 word 수, 상위 24-bit, status layout |
| A11 | BioSemi ActiveThree | 전용 schema, wire/output rate, decimation |
| A12 | Intan RHD / Rhythm | chip map, FPGA image, API와 frame layout |
| A13 | Intan RHS / RhythmStim | sampling mode, stim state, trigger mode |
| A14 | Intan / RHX TCP | channel selection, filter chain, reference |
| A15 | Neuropixels / SpikeGLX | AP/LFP별 schema·rate·gain, sync edge |
| A16 | Blackrock / CereLink | 연결 mode, sample group, shared-memory schema |
| A17 | NIRSport2 / Aurora LSL | source-detector-wavelength tuple, actual rate, quantity |
| A18 | Artinis / OxySoft LSL | 모델, quantity, export 설정, 복구 데이터 availability |

각 행은 공통 conformance corpus에 profile fixture, decoder vector, descriptor fixture, 전이 trace, evidence manifest를 제출한다. SDK 배포 조건과 scope는 access_requirements에 기록하고 prepare가 현재 세션의 접근 상태를 조회한다.

## 장치 제어와 상태 전이

수명은 `Created → Prepared → Streaming → Draining → Closed`다. probe·prepare 실패는 Faulted로 이동한다. Streaming 중 재설정은 `Draining → Prepared(new_config) → Streaming(new_epoch)`를 거친다. [09](09-runtime.md)가 epoch barrier와 callback 종료를 정의한다.

명령은 command ID, capability, target, 단위·범위, deadline, 예정 시각, response policy를 가진다. `InsertMarker`는 소프트웨어 marker, `EmitEvent`는 장치 I/O 명령이다. `Stimulate`는 장치별 범위·interlock·arming 상태·동시 사용 조건의 검증을 요구한다. profile이 정한 fault 시 출력 상태로 복귀하고 receipt를 기록한다.

명령 재전송은 command의 idempotency 분류를 따른다. 비멱등 명령의 응답이 유실되면 `OutcomeUnknown`을 기록하고 status query·명시적 복구 정책을 수행한다. 물리 output의 성공 판정은 SDK receipt와 실제 edge 관측을 별도 증거로 보관한다.
