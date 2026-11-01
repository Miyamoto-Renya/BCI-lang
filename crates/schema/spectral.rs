//! 단측 PSD·대역·로그 전력의 주파수 축과 단위 계약.
use crate::{
    number::Rational,
    types::{DType, StreamType, ValueType},
    unit::Unit,
};

/// PSD에 곱하는 고정 창 함수.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Taper {
    /// 모든 계수가 1인 직사각 창.
    Rectangular,
    /// `w[n] = (1 − cos(2πn/N))/2`인 주기적 Hann 창.
    Hann,
}

/// 정확한 Hz 경계와 상한 bin 포함 정책.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Band {
    /// 포함할 하한 Hz.
    pub low: Rational,
    /// 상한 Hz.
    pub high: Rational,
    /// 상한과 정확히 일치하는 bin을 포함한다.
    pub include_high: bool,
}

/// 구현된 직접 DFT의 표본 수 상한.
pub const MAX_DFT_SAMPLES: usize = 4096;

/// Epoch를 N/2+1 bin의 단측 PSD 타입으로 바꾼다.
pub fn psd(input: &StreamType, taper: Taper) -> Result<StreamType, String> {
    let ValueType::Epoch(DType::F64, c, n, rate) = input.value else {
        return Err("psd는 Epoch<f64,C,N> 입력을 요구한다.".into());
    };
    if !(1..=MAX_DFT_SAMPLES).contains(&n)
        || !rate.is_positive()
        || (n == 1 && taper == Taper::Hann)
    {
        return Err(
            "PSD 표본 수는 1..4096, 표본률은 양수이며 Hann은 두 표본 이상을 요구한다.".into(),
        );
    }
    let mut output = input.clone();
    output.value = ValueType::Spectrum(DType::F64, c, n, rate);
    let seconds: Unit = "s".parse()?;
    for channel in &mut output.schema.channels {
        channel.unit = channel.unit.power(2)?.multiply(&seconds)?;
        channel.quantity = format!(
            "psd({};N={n};Hz={rate};taper={taper:?};OneSidedDensity)",
            channel.quantity
        );
    }
    Ok(output)
}

/// Spectrum의 주파수 경계를 정수 bin 구간 [first,end)로 해석한다.
pub fn bins(n: usize, rate: Rational, band: &Band) -> Result<(usize, usize), String> {
    if !(1..=MAX_DFT_SAMPLES).contains(&n)
        || !rate.is_positive()
        || band.low.compare(Rational::ZERO)?.is_lt()
        || !band.low.compare(band.high)?.is_lt()
        || band
            .high
            .compare(rate.divide(Rational::integer(2))?)?
            .is_gt()
    {
        return Err("대역은 0 <= low < high <= Nyquist 범위와 유효한 PSD 격자를 요구한다.".into());
    }
    let step = rate.divide(Rational::integer(n as i64))?;
    let mut first = None;
    let mut end = 0;
    for k in 0..=n / 2 {
        let frequency = step.multiply(Rational::integer(k as i64))?;
        let upper = frequency.compare(band.high)?;
        if !frequency.compare(band.low)?.is_lt()
            && (upper.is_lt() || (band.include_high && upper.is_eq()))
        {
            first.get_or_insert(k);
            end = k + 1;
        }
    }
    first
        .map(|first| (first, end))
        .ok_or_else(|| "선택 대역에 PSD bin을 한 개 이상 포함한다.".into())
}

/// PSD 밀도에 bin 폭을 곱한 전력 벡터의 타입을 만든다.
pub fn bandpower(input: &StreamType, band: &Band) -> Result<StreamType, String> {
    let ValueType::Spectrum(DType::F64, c, n, rate) = input.value else {
        return Err("bandpower는 단측 Spectrum<f64,C,N> 입력을 요구한다.".into());
    };
    bins(n, rate, band)?;
    let mut output = input.clone();
    output.value = ValueType::Vector(DType::F64, c);
    let hz: Unit = "Hz".parse()?;
    for channel in &mut output.schema.channels {
        channel.unit = channel.unit.multiply(&hz)?;
        channel.quantity = format!(
            "bandpower({};low={};high={};include_high={})",
            channel.quantity, band.low, band.high, band.include_high
        );
    }
    Ok(output)
}

/// 양의 floor·reference와 같은 단위의 대역 전력을 무차원 특징으로 바꾼다.
pub fn log_power(
    input: &StreamType,
    floor: Rational,
    reference: Rational,
    unit: &Unit,
) -> Result<StreamType, String> {
    if !matches!(input.value, ValueType::Vector(DType::F64, _))
        || !floor.is_positive()
        || !reference.is_positive()
        || input
            .schema
            .channels
            .iter()
            .any(|c| c.unit != *unit || !c.quantity.starts_with("bandpower("))
    {
        return Err(
            "log_power는 대역 전력 Vec와 같은 단위의 양의 floor·reference를 요구한다.".into(),
        );
    }
    let mut output = input.clone();
    for channel in &mut output.schema.channels {
        channel.quantity = format!(
            "log_power({};floor={floor};reference={reference};unit={unit})",
            channel.quantity
        );
        channel.unit = Unit::scalar();
    }
    Ok(output)
}
