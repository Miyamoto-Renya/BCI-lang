# 표면 언어와 모듈

이 문서는 `bci/0.1`의 소스 표현을 정의한다. 타입 의미는 [02](02-types.md), 그래프 실행 의미는 [03](03-semantics.md)가 소유한다.

현재 구현의 지원 범위·진단·직접 파일 모드는 [소스 검사 안내](../compiler.md)에 기록한다. 일반 알고리즘은 Rust 연산자, BCi 본문은 구성값과 처리 그래프를 소유한다.

## 파일과 어휘

- 소스는 UTF-8, 확장자는 `.bci`다. 줄 끝은 LF로 정규화한다. 진단의 원본 byte offset은 원본 파일을 기준으로 보관한다.
- 식별자는 ASCII `[A-Za-z_][A-Za-z0-9_]*`를 사용한다. 설명과 표시 이름은 Unicode 문자열로 작성한다.
- `//`는 줄 주석, `/* ... */`는 중첩 가능한 블록 주석이다.
- 문자열은 JSON의 escape 규칙을 사용한다. 문자열 원문과 디코딩 값을 AST에 저장한다.
- 숫자는 십진 정수와 소수·지수 표기를 사용한다. `_`는 숫자 사이에만 허용한다. 부호는 단항 연산자로 처리한다.
- 상수 소수는 유리수로 해석한다. `0.1`은 정확히 `1/10`이다. 커널 인수의 `f32`/`f64` 변환 시 반올림한다.
- 물리량은 `250[Hz]`, `4[ms]`, `1[uV^2]`처럼 표기한다. 단위 식은 등록한 기호의 곱·나눗셈·정수 거듭제곱으로 구성한다. `1`은 무차원 단위다.
- 예약어는 `language module use as pub const schema clock device dataset model protocol resource graph run let return live record true false`다.

스키마·타입·enum·함수 이름은 이름 공간으로 해석한다. `Anchor.End`는 표준 라이브러리에 선언한 enum 값이다. `device` 같은 선언 키워드를 이름 공간 경로의 머리로 사용할 수 있도록 path 문맥에서는 이름 토큰으로 수용한다.

## 핵심 문법

아래 EBNF의 `{}`는 반복, `[]`는 선택, 따옴표는 실제 토큰을 뜻한다. lexical token `identifier`, `number`, `string`, `unit_expr`는 위 규칙을 따른다.

```ebnf
program       = "language", string, ";", "module", path, ";", { item } ;
item          = import | binding | graph_decl | run_decl ;
import        = "use", path, [ "as", identifier ], ";" ;
binding       = [ "pub" ], binding_kind, identifier, [ ":", type ], "=", expr, ";" ;
binding_kind  = "const" | "schema" | "clock" | "device" | "dataset"
              | "model" | "protocol" | "resource" ;
graph_decl    = [ "pub" ], "graph", identifier, "(", [ params ], ")",
                "->", type, "!", mode, graph_body ;
params        = param, { ",", param }, [ "," ] ;
param         = identifier, ":", type ;
mode          = "live" | "record" ;
graph_body    = "{", { let_stmt | expr_stmt }, "return", expr, ";", "}" ;
run_decl      = "run", identifier, "{", { let_stmt | expr_stmt }, "}" ;
let_stmt      = "let", identifier, [ ":", type ], "=", expr, ";" ;
expr_stmt     = expr, ";" ;
type          = path, [ "<", type_arg, { ",", type_arg }, ">" ]
              | "(", type, ",", type, { ",", type }, ")" ;
type_arg      = type | number | quantity ;
expr          = pipe_expr ;
pipe_expr     = or_expr, { "|>", call_target } ;
call_target   = path, "(", [ arguments ], ")" ;
or_expr       = and_expr, { "||", and_expr } ;
and_expr      = equality, { "&&", equality } ;
equality      = compare, [ ( "==" | "!=" ), compare ] ;
compare       = sum, [ ( "<" | "<=" | ">" | ">=" ), sum ] ;
sum           = product, { ( "+" | "-" ), product } ;
product       = unary, { ( "*" | "/" ), unary } ;
unary         = [ "-" | "!" ], postfix ;
postfix       = atom, { "(", [ arguments ], ")" | ".", identifier } ;
arguments     = argument, { ",", argument }, [ "," ] ;
argument      = [ identifier, ":" ], expr ;
atom          = quantity | number | string | "true" | "false" | identifier
              | "[", [ expr, { ",", expr }, [ "," ] ], "]"
              | "#{", [ field, { ",", field }, [ "," ] ], "}"
              | "(", expr, [ ",", expr, { ",", expr } ], ")" ;
field         = identifier, ":", expr ;
quantity      = number, "[", unit_expr, "]" ;
path          = identifier, { ".", identifier } ;
```

타입 인수의 path는 kind 환경에서 값 타입·스키마·시계·발생 규칙 중 하나로 해석한다. 타입 문맥의 `>>`는 두 개의 닫는 토큰으로 읽는다. EBNF의 path와 postfix가 겹치는 부분은 이름 해석 시 가장 긴 모듈 경로를 먼저 선택한다.

`unit_expr`는 `unit_factor { ("*" | "/") unit_factor }`, `unit_factor`는 `unit_symbol ["^" signed_integer]`다. unit_symbol은 등록한 식별자 또는 무차원 기호 `1`이다. `V^2/Hz`, `s^-1`, `MiB`가 각각 유효하다. 물리량의 `[]`는 숫자 바로 뒤에서만 단위 접미사로 해석한다.

함수 인수는 위치 인수 뒤에 이름 인수를 배치한다. 같은 매개변수에 두 값을 주면 `E0104`를 생성한다. 레코드 필드와 이름 인수의 중복도 같은 규칙을 적용한다. 배열의 요소 타입과 길이는 상수 평가 또는 타입 추론으로 정한다.

## 선언과 이름 해석

`language "bci/0.1";`은 문법과 표준 라이브러리의 의미 판본을 고정한다. 한 파일은 한 모듈을 정의한다. 패키지의 모듈 경로는 `src/` 상대 경로와 대응한다. `use std.signal as signal;`은 잠금 파일에서 선택한 표준 모듈을 가져온다.

`const`, `schema`, `clock`, `device`, `dataset`, `model`, `protocol`, `resource`는 정적인 기술자를 선언한다. `device` 선언의 평가는 연결 계획을 생성한다. 실제 장치 열기는 runtime prepare 전이에 속한다. artifact 내용 조회는 build의 고정 입력 집합에 속한다.

최상위 이름은 모듈 전체에서 해석하고, 상수 의존 그래프는 위상 정렬한다. 순환 상수·모듈 import·재귀 graph 호출은 해당 순환 경로와 함께 `E0107`을 생성한다. 지역 `let`은 앞에서 선언한 이름을 참조한다. 동일 scope의 재선언은 `E0102`를 생성한다. 지역 이름과 import alias의 충돌도 진단한다.

`pub` 선언은 외부 모듈의 API다. 나머지는 모듈 scope에 속한다. `run`은 실행 진입점이며 CLI가 이름을 선택한다. 한 진입점의 stream은 sink 또는 반환값에 연결되어야 한다. 연결이 끝나는 stream에는 `E0110`과 가능한 sink 목록을 표시한다.

표준 prelude는 기본 값 타입, Stream·Vec·Epoch 등의 타입 생성자, StandardQuality와 registry에 고정한 enum 이름을 제공한다. 예제의 Anchor, Warmup, WindowStart, Quality, Queue, Mode, Replay, Membership, Missing, Group, Metric은 이 prelude에서 해석한다. prelude 판본은 std dependency lock에 포함한다.

## 그래프 구체화

`graph`는 형식화된 부분그래프를 생성한다. 각 호출은 독립 상태 인스턴스를 만든다. 불변 모델 artifact와 계수는 내용 해시를 통해 공유한다. `run` 내부 `let`은 stream·모델 핸들·기술자를 바인딩한다.

`x |> signal.fir(coeffs: h)`는 `signal.fir(x, coeffs: h)`로 전개한다. pipe는 왼쪽 결합이며 각 전개가 원래 소스 범위를 보유한다. 튜플 `(a, b)`는 병렬 포트 묶음이다. 두 stream의 시간 결합은 `time.join` 호출로 표현한다.

일반 산술은 정적 상수와 수치 매개변수에 적용한다. stream의 값별 산술·조건 선택은 등록한 `signal.scale`, `control.select` 등의 연산자를 사용한다. `graph` 본문은 유한한 DAG 구체화를 수행한다. 순환 실행은 `flow.feedback`의 전용 계약을 통해 표현한다.

## 일반 프로그래밍과의 분담

BCi 실행문은 선언·상수식·graph/연산자 호출로 구성한다. 일반 함수·`if`·`match`·`for`·`while`·재귀·closure는 Rust 확장 crate에 작성한다. Rust 함수의 등록 방식과 상태 API는 [13 Rust 확장](13-rust-extensions.md)이 정의한다.

구성에 따른 그래프 선택은 모듈·`run` 진입점과 고정 artifact로 표현한다. 데이터에 따른 조건 판단은 실행 연산자의 Rust 본문으로 표현한다. `control.select`는 같은 tick의 두 branch 출력과 조건을 소비하며 두 branch의 생산자 상태는 계획대로 진행한다. 한 조건에서만 수행할 계산은 Rust 커널 내부 분기로 묶는다.

BCi의 `let`은 그래프 포트·불변 값의 이름을 바인딩한다. 호출을 넘어서 유지되는 가변 상태는 등록한 stateful actor가 소유한다. runtime 데이터로 static descriptor를 계산하려는 호출은 `E0211 PhaseMismatch`, Rust 범용 실행문을 BCi 실행문 위치에 사용하는 경우는 `E0112 HostSyntaxRequired`로 진단하고 Rust 연산자 작성 위치를 안내한다.

## 표기 예시

```bci
language "bci/0.1";
module demo.features;

use std.signal as signal;
use std.schema as schema;
use std.time as time;

schema EEG = schema.load("schemas/eeg8.schema.json");
clock Session = time.session_clock();
const Sampling = time.uniform(rate: 250[Hz], phase: 0[s]);
```

발생 규칙은 위의 값 생성자로 선언하고, 타입 인수에서는 선언 이름을 참조한다.

```bci
graph Bandpass(
    x: Stream<Vec<f64, 8>, EEG, Session, Sampling, StandardQuality>
) -> Stream<Vec<f64, 8>, EEG, Session, Sampling, StandardQuality> ! live {
    let h = signal.coefficients("filters/bandpass63.coeff");
    return x |> signal.fir(coeffs: h, warmup: Warmup.Full);
}
```

실행 사례의 완전한 소스는 [12](12-examples.md)에 둔다.

## 확장 경계

원시 연산자의 Rust 코드는 registry에 서명, 효과, 상태 codec, 수치 계약, 커널 식별자를 등록한다. BCi 소스는 registry의 이름을 모듈 import로 참조한다. 사용자 graph는 이 원시 연산자를 합성한다. registry 버전 변경은 잠금 파일과 manifest의 digest를 바꾼다.

등록 매크로는 Rust 서명과 계약 파일에서 wrapper와 registry를 생성한다. Rust 컴파일러는 함수 본문을 검사하고, BCi 컴파일러는 등록한 signature·effect와 그래프 연결을 검사한다. 함수의 수치·상태·시간 의미는 원시 연산자의 적합성 증거에 연결한다.
