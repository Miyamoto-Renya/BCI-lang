# 측정 CSV와 기록 포트 재생

합성·CSV·재생은 `InputSource::read`로 예약된 Frame을 채운다. 모든 reader가 같은 묶음에서 false를 반환하면 런타임은 EOF를 수락하고 기록을 commit한다. `bci run`은 파일 입력에서 기본적으로 EOF까지 실행하며, `--samples N`으로 원천별로 읽을 레코드 수를 제한한다. 여러 원천의 공통 격자·시각·Gap·EOF는 [동기 다중 입력 계약](multi-input.md)을 따른다.

```sh
cargo run --offline --locked -- run examples/file.bci --output recordings/file
```

[file.bci](../examples/file.bci)와 [samples.csv](../examples/samples.csv)는 파일 형식과 오류 정책을 보여 주기 위해 작성한 예제 데이터다. 22행 중 2개 행에 무효 채널을 넣었고, 격자 index 12·13을 생략했다. 실제 실행은 원시 레코드 22개와 공분산 레코드 7개를 기록한다. 공분산 중 유효한 SPD 결과는 2개이며 나머지 5개는 무효 mask와 0 값을 가진다.

## CSV 소스

```bci
let raw = source.csv(
    path: "samples.csv", schema: EEG, clock: Session, rate: 250[Hz],
    phase: 0[s], gap: Gap.Reset, invalid: Invalid.Mark
);
```

path는 `.bci` 파일의 디렉터리 기준이다. schema·clock·rate는 필수, phase의 기본값은 0초다. gap 기본값은 Gap.Reset, invalid 기본값은 Invalid.Fault다.

CSV는 UTF-8의 쉼표 구분 파일이며 한 물리 행이 한 표본이다. 첫 열 이름은 `time_s`, 다음 열들은 스키마 순서의 `채널ID[단위]`다. 선택적인 품질 열을 사용할 때는 모든 채널의 `채널ID.valid`를 같은 순서로 뒤에 둔다. LF·CRLF·UTF-8 BOM과 한 행 안의 인용 필드·이중 따옴표 escape를 처리한다.

```csv
time_s,C3[uV],C4[uV],C3.valid,C4.valid
0.000,0.2,0.5,1,1
0.004,NaN,0.4,1,1
0.012,0.3,0.2,1,0
```

위 세 행은 250 Hz 격자의 index 0·1·3이다. Invalid.Mark에서 NaN은 0·invalid로 바뀌고, index 2의 누락은 새 generation을 시작한다. 마지막 C4 값 0.2는 저장한 값을 유지하며 valid=0을 전달한다.

컴파일과 prepare는 헤더의 채널 순서·ID·정확한 단위 배율을 스키마와 대조한다. 측정량·reference·clock 의미는 `.bci`의 선언에서 가져온다. 수치 셀은 f64, 시각 셀은 정확한 십진·지수 또는 정수 분수로 읽는다. `n=(time-phase)*rate`는 0..i64::MAX의 정수여야 한다. 관측 시각은 엄격히 증가해야 한다.

## 누락·무효 값·시간 오류

| 조건 | 처리 |
|---|---|
| index 누락 + Gap.Reset | 누락 범위를 gap에 기록하고 generation 증가, FIR warmup·FirstValid 창 상태 초기화 |
| index 누락 + Gap.Fault | 파일·행·누락 범위와 E0904, 세션 종료 |
| 빈 셀·변환 실패·NaN·무한대 + Invalid.Mark | 값 0, 해당 채널 valid=0, InvalidInput flag |
| 같은 수치 오류 + Invalid.Fault | 파일·행·채널과 E0904, 세션 종료 |
| 명시적인 valid=0 | 유한 입력값과 무효 mask를 함께 전달 |
| 시간 역전·중복·격자 밖 시각 | 두 정책 공통으로 E0904, 세션 종료 |
| 열 수·CSV 인용 구조·valid 값 오류 | E0904와 파일 행 번호 |

Gap.Reset은 원천의 원래 index·시각을 보존하고 다음 generation에서 처리를 이어 간다. 실행기는 각 generation의 격자를 검사한다. FirstValid 창의 명목 위상은 generation마다 첫 완성 창으로 확정한다. manifest는 generation 수와 첫·마지막 generation의 anchor를 요약하고, 개별 레코드는 자신의 generation·시각을 보관한다.

## 기록 포트 선택

실행 완료 메시지에 나온 디렉터리의 `sink-ID.jsonl`을 지정한다. 같은 디렉터리의 manifest에서 포트 타입과 commit 관측값을 읽는다.

```bci
let raw = source.replay(
    path: "recordings/session/run-ID/sink-6.jsonl", port: 0,
    gap: Gap.Reset, expect: EEG, clock: Session, rate: 250[Hz]
);
```

path·port가 필수다. gap 기본값은 Gap.Reset이다. expect·clock·rate는 선택적인 일치 조건이다. 생략한 조건도 원래 기록의 스키마·clock·발생 규칙을 포트 타입으로 복원한다. 재생 clock을 graph 타입에 명시할 때는 원래 clock ID를 사용한다. 예제의 `time.session_clock(name: "measurement.session")`은 파일·재생 그래프가 같은 명목 시계를 공유하도록 지정한다.

재생은 f64 Vec·Epoch·Spectrum·Cov·SPD 포트를 읽는다. 값, valid mask, flags, 정확한 time_seconds, sequence, source_range, generation, origin, gap을 보존한다. JSON f64 읽기는 float_roundtrip 설정을 사용한다. 같은 raw 포트를 같은 수치 그래프에 연결한 재생은 이 필드들의 동일성을 통합 검사로 확인한다.

재생 대상은 성공적으로 commit된 Closed·Stopped 실행이다. reader는 manifest/header의 entry·포트·타입, 값·품질 배열 크기, 유한 수치, 시간 순서·격자, generation, 선택 포트의 EOF 레코드 수·유효 수를 검사한다. EOF의 관측 수가 commit 값과 다르면 잘린 기록으로 진단한다. `--samples`·stop으로 EOF 전에 끝낸 경우 Input 의무는 Guarded이며 검사한 레코드 수를 기록한다.

Gap.Fault는 재생 레코드의 명시적 gap 또는 generation 전환에서 중단한다. Gap.Reset은 기록된 generation과 상태 초기화 경계를 그대로 적용한다. 현재 데이터 형식은 bci-recording/0.6이다. 0.5·0.4·0.3·0.2 기록도 읽으며, 기존 합성 기록 0.1은 Synthetic origin·generation 0으로 복원한다.

## 자원과 확장 위치

CSV·재생은 한 행씩 읽는다. 행 상한은 포트 값 수·품질 크기로 계산하고 최대 1 MiB로 제한한다. 재생 sink에 여러 포트가 있으면 가장 큰 행을 기준으로 예약한다. 입력 buffer 64 KiB와 행 문자열·JSON 임시 공간 예약량을 실행 memory 예산에 더한다. 기록 manifest의 읽기 상한은 8 MiB이며 compiler 기술자와 함께 호스트 메모리로 관리한다.

schema/input.rs가 SourceSpec·InputSource·SourceFactory·GridCursor를 소유한다. artifact의 csv.rs·replay.rs가 파일 reader, operators/source.rs가 합성 reader를 구현한다. CLI가 SourceKind에 맞는 factory를 조립한다. devices는 같은 InputSource 계약을 노출하며, 향후 adapter는 장치 binding과 이 read·EOF·오류 경계를 연결한다.

파일 검증은 기존 CLI 통합 검사 파일에 함수 하나를 추가했다. 같은 CSV 예제로 기록·재생의 전체 데이터 일치, 공분산 포트 직접 재생, EOF·상한 종료, 시간·품질·누락·잘린 기록의 핵심 오류를 확인한다.

Spectrum 포트의 DFT 표본 수·bin 폭·단측 정규화와 비음수 밀도 검사 규칙은 [PSD 안내](spectral.md)에 정의한다.
