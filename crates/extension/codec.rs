//! 상한을 검사하는 little-endian 상태 codec.

/// 예약한 byte 상한으로 상태를 쓰는 writer.
pub struct StateWriter {
    bytes: Vec<u8>,
    limit: usize,
}
impl StateWriter {
    /// 최대 payload 공간을 예약한다.
    pub fn new(limit: usize) -> Result<Self, String> {
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(limit).map_err(|e| e.to_string())?;
        Ok(Self { bytes, limit })
    }
    /// 고정 길이 byte열을 추가한다.
    pub fn bytes(&mut self, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() > self.limit - self.bytes.len() {
            return Err("checkpoint payload가 계약 상한을 초과했다.".into());
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
    /// 정수를 little-endian으로 추가한다.
    pub fn u64(&mut self, value: u64) -> Result<(), String> {
        self.bytes(&value.to_le_bytes())
    }
    /// f64의 정확한 bit를 추가한다.
    pub fn f64(&mut self, value: f64) -> Result<(), String> {
        self.u64(value.to_bits())
    }
    /// 선택 정수를 tag와 함께 추가한다.
    pub fn optional(&mut self, value: Option<u64>) -> Result<(), String> {
        self.bytes(&[u8::from(value.is_some())])?;
        if let Some(value) = value {
            self.u64(value)?;
        }
        Ok(())
    }
    /// 예약 layout의 레코드를 저장한다.
    pub fn frame(&mut self, frame: &bci_schema::execution::Frame) -> Result<(), String> {
        for n in [
            frame.sequence,
            frame.generation,
            frame.source_first,
            frame.source_last,
        ] {
            self.u64(n)?;
        }
        self.bytes(&[frame.flags, frame.origin as u8])?;
        self.optional(frame.gap.map(|g| g.first))?;
        self.optional(frame.gap.map(|g| g.last))?;
        let time = frame.time.to_string();
        self.u64(time.len() as u64)?;
        self.bytes(time.as_bytes())?;
        for &value in &frame.values {
            self.f64(value)?;
        }
        self.bytes(&frame.valid)
    }
    /// 완성한 payload를 반환한다.
    pub fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

/// 입력 범위와 완전 소비를 검사하는 reader.
pub struct StateReader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}
impl<'a> StateReader<'a> {
    /// 불변 payload를 연결한다.
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }
    /// 지정한 길이의 byte열을 읽는다.
    pub fn bytes(&mut self, count: usize) -> Result<&'a [u8], String> {
        let end = self
            .cursor
            .checked_add(count)
            .filter(|&n| n <= self.bytes.len())
            .ok_or("checkpoint payload가 잘렸다.")?;
        let result = &self.bytes[self.cursor..end];
        self.cursor = end;
        Ok(result)
    }
    /// 정수를 읽는다.
    pub fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(
            self.bytes(8)?.try_into().expect("8 bytes"),
        ))
    }
    /// 유한 f64를 복원한다.
    pub fn f64(&mut self) -> Result<f64, String> {
        let value = f64::from_bits(self.u64()?);
        if value.is_finite() {
            Ok(value)
        } else {
            Err("상태 수치는 유한 f64를 요구한다.".into())
        }
    }
    /// 선택 정수 tag를 검사해 복원한다.
    pub fn optional(&mut self) -> Result<Option<u64>, String> {
        match self.bytes(1)?[0] {
            0 => Ok(None),
            1 => Ok(Some(self.u64()?)),
            _ => Err("선택 상태 tag는 0 또는 1을 요구한다.".into()),
        }
    }
    /// 준비한 layout에 레코드를 복원하고 수치·품질·범위를 검사한다.
    pub fn frame(&mut self, frame: &mut bci_schema::execution::Frame) -> Result<(), String> {
        frame.sequence = self.u64()?;
        frame.generation = self.u64()?;
        frame.source_first = self.u64()?;
        frame.source_last = self.u64()?;
        frame.flags = self.bytes(1)?[0];
        frame.origin = bci_schema::execution::Origin::from_code(self.bytes(1)?[0])?;
        frame.gap = match (self.optional()?, self.optional()?) {
            (None, None) => None,
            (Some(first), Some(last)) if first <= last => {
                Some(bci_schema::execution::Gap { first, last })
            }
            _ => return Err("레코드 Gap 경계의 순서를 맞춘다.".into()),
        };
        let length = self.u64()?;
        if length > 80 {
            return Err("레코드 유리수 시각 길이는 80 bytes 이하여야 한다.".into());
        }
        frame.time = bci_schema::input::exact_time(
            std::str::from_utf8(self.bytes(length as usize)?).map_err(|e| e.to_string())?,
        )?;
        for value in &mut frame.values {
            *value = self.f64()?;
        }
        let length = frame.valid.len();
        frame.valid.copy_from_slice(self.bytes(length)?);
        if frame.flags > 7
            || frame.valid.iter().any(|&v| v > 1)
            || frame.source_first > frame.source_last
        {
            return Err("레코드 품질·원천 범위를 맞춘다.".into());
        }
        Ok(())
    }
    /// payload의 모든 byte를 소비했는지 확인한다.
    pub fn finish(&self) -> Result<(), String> {
        if self.cursor == self.bytes.len() {
            Ok(())
        } else {
            Err("codec이 checkpoint 전체를 소비해야 한다.".into())
        }
    }
}
