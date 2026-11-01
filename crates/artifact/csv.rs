use crate::input::Lines;
use bci_schema::{
    execution::{Frame, GAP, Gap, INVALID_INPUT, Origin},
    input::{
        GapPolicy, InputError, InputSource, InvalidPolicy, SourceKind, SourceSpec, exact_time,
        grid_index,
    },
    types::{DType, Occurrence, ValueType},
    unit::Unit,
};

/// 헤더의 순서·채널 ID·단위를 기대 스키마와 대조한다.
pub fn inspect_csv(spec: &SourceSpec) -> Result<(), InputError> {
    CsvSource::open(spec).map(|_| ())
}

pub(crate) struct CsvSource {
    lines: Lines,
    spec: SourceSpec,
    valid_columns: bool,
    previous: Option<u64>,
    generation: u64,
}
impl CsvSource {
    pub fn open(spec: &SourceSpec) -> Result<Self, InputError> {
        let SourceKind::Csv { path, .. } = &spec.kind else {
            return Err(InputError::contract("CSV 원천 계약을 지정한다."));
        };
        if !matches!(spec.stream.value, ValueType::Vector(DType::F64, _))
            || !matches!(spec.stream.occurrence, Occurrence::Uniform { .. })
        {
            return Err(InputError::contract(
                "CSV 원천은 f64 Vec와 확정한 Uniform 격자를 요구한다.",
            ));
        }
        let mut lines = Lines::open(path)?;
        let header = lines.next(1_048_576, false)?.ok_or_else(|| {
            InputError::contract(format!("{}: CSV 헤더를 지정한다.", path.display()))
        })?;
        let fields = fields(header.trim_start_matches('\u{feff}')).map_err(|e| lines.error(e))?;
        let channels = &spec.stream.schema.channels;
        if fields.first().map(String::as_str) != Some("time_s")
            || (fields.len() != channels.len() + 1 && fields.len() != channels.len() * 2 + 1)
        {
            return Err(lines
                .error("헤더는 time_s, 채널[단위] 목록과 선택적인 채널.valid 목록을 요구한다."));
        }
        for (index, channel) in channels.iter().enumerate() {
            let (id, unit) = fields[index + 1]
                .strip_suffix(']')
                .and_then(|s| s.rsplit_once('['))
                .ok_or_else(|| lines.error("채널 헤더는 C3[uV] 형식을 요구한다."))?;
            let unit: Unit = unit.parse().map_err(|e| lines.error(e))?;
            if id != channel.id || unit != channel.unit {
                return Err(lines.error(format!(
                    "열 {}의 {}[{}]와 기대 채널 {}[{}]를 맞춘다.",
                    index + 2,
                    id,
                    unit,
                    channel.id,
                    channel.unit
                )));
            }
        }
        let valid_columns = fields.len() == channels.len() * 2 + 1;
        if valid_columns
            && channels
                .iter()
                .enumerate()
                .any(|(c, ch)| fields[1 + channels.len() + c] != format!("{}.valid", ch.id))
        {
            return Err(
                lines.error("품질 열의 이름·순서는 채널.valid 형식과 입력 스키마를 따른다.")
            );
        }
        lines.release_header();
        Ok(Self {
            lines,
            spec: spec.clone(),
            valid_columns,
            previous: None,
            generation: 0,
        })
    }
}
impl InputSource for CsvSource {
    fn read(&mut self, output: &mut Frame) -> Result<bool, InputError> {
        let Some(line) = self.lines.next(self.spec.row_bytes, false)? else {
            return Ok(false);
        };
        let values = fields(line).map_err(|e| self.lines.error(e))?;
        let count = output.valid.len();
        if values.len() != 1 + count * (if self.valid_columns { 2 } else { 1 }) {
            return Err(self.lines.error("CSV 행의 열 수를 헤더와 맞춘다."));
        }
        let time = exact_time(values[0].trim()).map_err(|e| self.lines.error(e))?;
        let Occurrence::Uniform { rate, phase } = self.spec.stream.occurrence else {
            unreachable!("checked CSV occurrence")
        };
        let index = grid_index(time, rate, phase).map_err(|e| {
            self.lines.error(format!(
                "시각 {}s가 {rate}Hz, 위상 {phase}s 격자와 다르다: {e}",
                time
            ))
        })?;
        if self.previous.is_some_and(|previous| index <= previous) {
            return Err(self
                .lines
                .error("시각 역전·중복을 발견했다. 입력 행의 시간 순서를 맞춘다."));
        }
        let expected = self.previous.map_or(0, |n| n + 1);
        let SourceKind::Csv { gap, invalid, .. } = self.spec.kind else {
            unreachable!("CSV source")
        };
        output.gap = None;
        output.flags = 0;
        if index > expected {
            if gap == GapPolicy::Fault {
                return Err(self.lines.error(format!("누락 표본 {expected}..{}. Gap.Reset으로 상태 초기화를 허용하거나 입력을 복구한다.",index-1)));
            }
            self.generation += 1;
            output.gap = Some(Gap {
                first: expected,
                last: index - 1,
            });
            output.flags = GAP;
        }
        output.sequence = index;
        output.generation = self.generation;
        output.origin = Origin::Measurement;
        output.time = time;
        output.source_first = index;
        output.source_last = index;
        for channel in 0..count {
            let number = values[channel + 1]
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|n| n.is_finite());
            let mut valid = 1;
            if self.valid_columns {
                valid = match values[1 + count + channel].trim() {
                    "0" => 0,
                    "1" => 1,
                    _ => return Err(self.lines.error("valid 열은 0 또는 1을 요구한다.")),
                };
            }
            if number.is_none() {
                if invalid == InvalidPolicy::Fault {
                    return Err(self.lines.error(format!("{} 채널은 유한 수치를 요구한다. Invalid.Mark로 무효 값 게시를 선택하거나 입력을 복구한다.",self.spec.stream.schema.channels[channel].id)));
                }
                valid = 0;
            }
            output.values[channel] = number.unwrap_or(0.0);
            output.valid[channel] = valid;
            if valid == 0 {
                output.flags |= INVALID_INPUT;
            }
        }
        self.previous = Some(index);
        Ok(true)
    }
}

fn fields(line: &str) -> Result<Vec<String>, String> {
    let mut values = Vec::new();
    let mut input = line.chars().peekable();
    loop {
        let mut value = String::new();
        if input.peek() == Some(&'"') {
            input.next();
            loop {
                match input.next() {
                    Some('"') if input.peek() == Some(&'"') => {
                        input.next();
                        value.push('"');
                    }
                    Some('"') => break,
                    Some(c) => value.push(c),
                    None => return Err("CSV 인용 필드의 닫는 따옴표를 요구한다.".into()),
                }
            }
            if input.peek().is_some_and(|&c| c != ',') {
                return Err("닫는 따옴표 다음에는 쉼표 또는 행 끝을 요구한다.".into());
            }
        } else {
            while input.peek().is_some_and(|&c| c != ',') {
                let c = input.next().expect("peeked");
                if c == '"' {
                    return Err("CSV 따옴표는 필드 시작에서 사용한다.".into());
                }
                value.push(c);
            }
        }
        values.push(value);
        if input.next().is_none() {
            break;
        }
    }
    Ok(values)
}
