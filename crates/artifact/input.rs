//! 한 행씩 읽는 측정 CSV·BCi 기록 입력과 파일 위치 진단.

use bci_schema::input::{InputError, InputSource, SourceFactory, SourceKind, SourceSpec};
use std::{
    fs::File,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
};

pub use crate::csv::inspect_csv;
pub use crate::replay::inspect_replay;

/// CSV·JSON Lines 입력 구현체를 생성한다.
pub struct FileSources;
impl SourceFactory for FileSources {
    fn open(&self, spec: &SourceSpec) -> Result<Box<dyn InputSource>, InputError> {
        match spec.kind {
            SourceKind::Csv { .. } => Ok(Box::new(crate::csv::CsvSource::open(spec)?)),
            SourceKind::Replay { .. } => Ok(Box::new(crate::replay::ReplaySource::open(spec)?)),
            SourceKind::Synthetic => Err(InputError::contract(
                "합성 입력을 SyntheticSources에 연결한다.",
            )),
        }
    }
}

pub(crate) struct Lines {
    reader: BufReader<File>,
    pub path: PathBuf,
    pub number: usize,
    text: String,
}
impl Lines {
    pub fn open(path: &Path) -> Result<Self, InputError> {
        let file =
            File::open(path).map_err(|e| InputError::io(format!("{}: {e}", path.display())))?;
        if !file
            .metadata()
            .map_err(|e| InputError::io(e.to_string()))?
            .is_file()
        {
            return Err(InputError::contract(format!(
                "{}: 일반 파일을 입력으로 지정한다.",
                path.display()
            )));
        }
        Ok(Self {
            reader: BufReader::with_capacity(65_536, file),
            path: path.into(),
            number: 0,
            text: String::new(),
        })
    }
    pub fn next(&mut self, limit: usize, newline: bool) -> Result<Option<&str>, InputError> {
        self.text.clear();
        self.number += 1;
        let count = (&mut self.reader)
            .take(limit as u64 + 1)
            .read_line(&mut self.text)
            .map_err(|e| self.io(e.to_string()))?;
        if count == 0 {
            return Ok(None);
        }
        if count > limit {
            return Err(self.error(format!("행 길이가 예약 상한 {limit} bytes를 초과했다.")));
        }
        if newline && !self.text.ends_with('\n') {
            return Err(self.error("완결된 JSON Lines 레코드의 줄바꿈을 요구한다."));
        }
        Ok(Some(self.text.trim_end_matches(['\r', '\n'])))
    }
    pub fn release_header(&mut self) {
        self.text = String::new();
    }
    pub fn error(&self, message: impl std::fmt::Display) -> InputError {
        InputError::contract(format!(
            "{}:{}: {message}",
            self.path.display(),
            self.number
        ))
    }
    pub fn io(&self, message: impl std::fmt::Display) -> InputError {
        InputError::io(format!(
            "{}:{}: {message}",
            self.path.display(),
            self.number
        ))
    }
}
