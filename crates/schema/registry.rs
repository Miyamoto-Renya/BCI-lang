//! 표준 구성 함수와 연산자의 공개 호출 서명.

use crate::types::Mode;

/// 이름 인수와 위치 인수의 공통 계약.
#[derive(Debug)]
pub struct Parameter {
    /// 호출에 사용하는 이름.
    pub name: &'static str,
    /// 필수 인수 여부.
    pub required: bool,
}

/// 이름 해석과 인수 검사에 사용하는 표준 서명.
#[derive(Debug)]
pub struct Signature {
    /// 정규 모듈 경로.
    pub name: &'static str,
    /// 위치 순서와 이름을 정의하는 매개변수.
    pub parameters: &'static [Parameter],
    /// 구성 함수 여부.
    pub descriptor: bool,
    /// 연산 자체의 처리 효과.
    pub mode: Mode,
}

macro_rules! params { ($($name:literal => $required:literal),* $(,)?) => { &[$(Parameter {name:$name,required:$required}),*] }; }
macro_rules! sig { ($name:literal,$descriptor:literal,$mode:ident,[$($arg:literal => $req:literal),* $(,)?]) => { Signature{name:$name,descriptor:$descriptor,mode:Mode::$mode,parameters:params![$($arg=>$req),*]} }; }

/// 컴파일러가 의미 규칙을 제공하는 표준 서명 목록.
pub static STANDARD_SIGNATURES: &[Signature] = &[
    sig!("std.model.load",true,Live,["path"=>true]),
    sig!("std.model.predict",false,Live,["input"=>true,"using"=>true]),
    sig!("std.decision.threshold",false,Live,["input"=>true,"minimum"=>true]),
    sig!("std.schema.electrical",true,Live,["channels"=>true,"unit"=>true,"reference"=>true,"name"=>false]),
    sig!("std.schema.load",true,Live,["path"=>true]),
    sig!("std.time.session_clock",true,Live,["name"=>false]),
    sig!("std.time.uniform",true,Live,["rate"=>true,"phase"=>true]),
    sig!("std.time.device_clock",true,Live,["device"=>true]),
    sig!("std.time.mapping",true,Live,["path"=>true]),
    sig!("std.device.bind",true,Live,["profile"=>true,"config"=>true]),
    sig!("std.signal.sos",true,Live,["path"=>true]),
    sig!("std.signal.coefficients",true,Live,["path"=>true]),
    sig!("std.runtime.budget",true,Live,["memory"=>true,"queue_policy"=>true,"profile"=>false]),
    sig!("std.runtime.configure",false,Live,["mode"=>true,"resources"=>false]),
    sig!("std.source.synthetic",false,Live,["schema"=>true,"clock"=>true,"rate"=>true,"phase"=>false]),
    sig!("std.source.csv",false,Live,["path"=>true,"schema"=>true,"clock"=>true,"rate"=>true,"phase"=>false,"gap"=>false,"invalid"=>false]),
    sig!("std.source.replay",false,Live,["path"=>true,"port"=>true,"gap"=>false,"expect"=>false,"clock"=>false,"rate"=>false]),
    sig!("std.source.acquire",false,Live,["device"=>true,"expect"=>true,"rate"=>true]),
    sig!("std.signal.convert_unit",false,Live,["input"=>true,"to"=>true]),
    sig!("std.signal.scale",false,Live,["input"=>true,"factor"=>true]),
    sig!("std.signal.mix",false,Live,["input"=>true,"weights"=>true]),
    sig!("std.signal.iir",false,Live,["input"=>true,"realization"=>true]),
    sig!("std.signal.fir",false,Live,["input"=>true,"coeffs"=>true,"warmup"=>false]),
    sig!("std.signal.filtfilt",false,Record,["input"=>true,"coeffs"=>true]),
    sig!("std.signal.window",false,Live,["input"=>true,"size"=>true,"hop"=>true,"start"=>false,"anchor"=>false,"quality"=>false,"tail"=>false]),
    sig!("std.signal.psd",false,Live,["input"=>true,"taper"=>true,"normalization"=>false]),
    sig!("std.signal.bandpower",false,Live,["input"=>true,"band"=>true,"include_high"=>false]),
    sig!("std.signal.log_power",false,Live,["input"=>true,"floor"=>true,"reference"=>true]),
    sig!("std.signal.covariance",false,Live,["input"=>true]),
    sig!("std.signal.regularize",false,Live,["input"=>true,"shrinkage"=>true,"floor"=>true]),
    sig!("std.signal.resample",false,Live,["input"=>true,"up"=>true,"down"=>true,"filter"=>true,"phase"=>true]),
    sig!("std.signal.rereference",false,Live,["input"=>true,"weights"=>true]),
    sig!("std.signal.select",false,Live,["input"=>true,"channels"=>true]),
    sig!("std.signal.reorder",false,Live,["input"=>true,"channels"=>true]),
    sig!("std.time.align",false,Live,["input"=>true,"to"=>true,"mapping"=>true]),
    sig!("std.source.assert_schema",false,Live,["input"=>true,"expect"=>true,"rate"=>true]),
    sig!("std.recording.write",false,Live,["input"=>true,"to"=>true]),
];

/// 정규 이름으로 표준 서명을 조회한다.
pub fn signature(name: &str) -> Option<&'static Signature> {
    STANDARD_SIGNATURES.iter().find(|s| s.name == name)
}

/// 표준 모듈의 import 가능 여부를 확인한다.
pub fn contains_module(name: &str) -> bool {
    STANDARD_SIGNATURES.iter().any(|s| {
        s.name
            .rsplit_once('.')
            .is_some_and(|(module, _)| module == name)
    })
}
