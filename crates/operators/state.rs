use bci_extension::{StateReader, StateWriter};

pub(crate) fn write(
    output: &mut StateWriter,
    values: &[f64],
    valid: &[u8],
    first: &[u64],
    last: &[u64],
) -> Result<(), String> {
    for &v in values {
        output.f64(v)?;
    }
    output.bytes(valid)?;
    for &v in first.iter().chain(last) {
        output.u64(v)?;
    }
    Ok(())
}
pub(crate) fn read(
    input: &mut StateReader<'_>,
    values: &mut [f64],
    valid: &mut [u8],
    first: &mut [u64],
    last: &mut [u64],
) -> Result<(), String> {
    for v in values {
        *v = input.f64()?;
    }
    valid.copy_from_slice(input.bytes(valid.len())?);
    if valid.iter().any(|&v| v > 1) {
        return Err("상태 valid는 0 또는 1을 요구한다.".into());
    }
    for v in first.iter_mut().chain(last.iter_mut()) {
        *v = input.u64()?;
    }
    if first.iter().zip(last).any(|(f, l)| *f > *l) {
        return Err("상태 원천 범위의 순서를 맞춘다.".into());
    }
    Ok(())
}
pub(crate) fn previous(input: &mut StateReader<'_>) -> Result<Option<(u64, u64)>, String> {
    let g = input.optional()?;
    let n = input.optional()?;
    if g.is_some() != n.is_some() {
        return Err("generation·sequence를 함께 복원한다.".into());
    }
    Ok(g.zip(n))
}
