//! 채널 공간 변환의 타입·정확한 계수·기준 식별 계약.
use crate::{
    number::Rational,
    types::{DType, StreamType, ValueType},
    unit::Unit,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

/// Vec·Epoch의 dtype·채널 수·채널당 값 수를 반환한다.
pub fn layout(value: &ValueType) -> Result<(DType, usize, usize), String> {
    match value {
        ValueType::Vector(d, c) => Ok((d.clone(), *c, 1)),
        ValueType::Epoch(d, c, n, _) if *n > 0 => Ok((d.clone(), *c, *n)),
        _ => Err("채널 공간 변환은 Vec 또는 Epoch 입력을 요구한다.".into()),
    }
}

/// 고유한 입력 index로 채널 의미·값 shape를 함께 변환한다.
pub fn select(input: &StreamType, indices: &[usize], reorder: bool) -> Result<StreamType, String> {
    let (dtype, count, _) = layout(&input.value)?;
    if count != input.schema.channels.len()
        || indices.is_empty()
        || indices.iter().any(|&i| i >= count)
        || indices.iter().collect::<BTreeSet<_>>().len() != indices.len()
        || (reorder && indices.len() != count)
    {
        return Err(
            "채널 index는 고유한 입력 범위를 요구하며 reorder는 전체 순열을 요구한다.".into(),
        );
    }
    let mut output = input.clone();
    output.schema.channels = indices
        .iter()
        .map(|&i| input.schema.channels[i].clone())
        .collect();
    output.value = match input.value {
        ValueType::Epoch(_, _, n, rate) => ValueType::Epoch(dtype, indices.len(), n, rate),
        _ => ValueType::Vector(dtype, indices.len()),
    };
    Ok(output)
}

/// 정확히 합이 1인 전압 계수로 새 기준을 만들고 출력 스키마에 결합한다.
pub fn rereference(input: &StreamType, weights: &[Rational]) -> Result<StreamType, String> {
    let (dtype, count, _) = layout(&input.value)?;
    if !matches!(dtype, DType::F32 | DType::F64)
        || count == 0
        || count != input.schema.channels.len()
        || weights.len() != count
    {
        return Err("재기준화는 채널마다 weight 하나를 가진 부동소수 Vec·Epoch를 요구한다.".into());
    }
    let first = &input.schema.channels[0];
    let voltage: Unit = "V".parse()?;
    if !first.unit.same_dimension(&voltage)
        || first.reference.is_empty()
        || input.schema.channels.iter().any(|c| {
            c.quantity != "electrical.voltage"
                || c.unit != first.unit
                || c.reference != first.reference
        })
    {
        return Err(
            "재기준화 채널의 electrical.voltage 측정량·전압 단위 배율·기존 기준을 맞춘다.".into(),
        );
    }
    let sum = weights
        .iter()
        .try_fold(Rational::ZERO, |sum, &w| sum.checked_add(w))?;
    if sum != Rational::ONE {
        return Err(format!(
            "재기준화 weights의 정확한 합은 1을 요구한다. 현재 합: {sum}."
        ));
    }
    for &weight in weights {
        Rational::ONE.checked_add(weight.negate()?)?;
    }
    // ID 순으로 직렬화해 채널 재정렬과 동치 유리수 표기를 같은 기준으로 묶는다.
    let mut terms: Vec<_> = input
        .schema
        .channels
        .iter()
        .zip(weights)
        .filter(|(_, w)| !w.is_zero())
        .collect();
    terms.sort_by(|(a, _), (b, _)| a.id.cmp(&b.id));
    let mut hash = Sha256::new();
    hash.update(b"bci-rereference/0.1");
    let mut field = |value: &str| {
        hash.update((value.len() as u64).to_le_bytes());
        hash.update(value.as_bytes());
    };
    field(&first.reference);
    for (channel, weight) in terms {
        field(&channel.id);
        field(&weight.to_string());
    }
    let reference = format!("rereference:sha256:{:x}", hash.finalize());
    let mut output = input.clone();
    for channel in &mut output.schema.channels {
        channel.reference.clone_from(&reference);
    }
    Ok(output)
}
