//! SI 차원과 저장량 차원을 포함한 정확한 단위 계산.

use crate::number::Rational;
use std::{fmt, str::FromStr};

/// SI 7차원과 저장량 차원의 선형 단위.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unit {
    /// M,L,T,I,Theta,N,J,Storage 순서의 지수.
    pub dimension: [i16; 8],
    /// 기준 단위에 대한 정확한 배율.
    pub scale: Rational,
}

impl Unit {
    /// 무차원 단위.
    pub fn scalar() -> Self {
        Self {
            dimension: [0; 8],
            scale: Rational::ONE,
        }
    }
    /// 단위 곱셈.
    pub fn multiply(&self, rhs: &Self) -> Result<Self, String> {
        let mut dimension = [0; 8];
        for (index, value) in dimension.iter_mut().enumerate() {
            *value = self.dimension[index]
                .checked_add(rhs.dimension[index])
                .ok_or("단위 지수 범위를 초과했다.")?;
        }
        Ok(Self {
            dimension,
            scale: self.scale.multiply(rhs.scale)?,
        })
    }
    /// 단위의 정수 거듭제곱.
    pub fn power(&self, exponent: i16) -> Result<Self, String> {
        let mut dimension = [0; 8];
        for (index, value) in dimension.iter_mut().enumerate() {
            *value = self.dimension[index]
                .checked_mul(exponent)
                .ok_or("단위 지수 범위를 초과했다.")?;
        }
        Ok(Self {
            dimension,
            scale: self.scale.power(exponent)?,
        })
    }
    /// 배율 변환이 가능한 차원 관계.
    pub fn same_dimension(&self, rhs: &Self) -> bool {
        self.dimension == rhs.dimension
    }
}

fn symbol(name: &str) -> Option<Unit> {
    let (dimension, scale) = match name {
        "1" => ([0; 8], Rational::ONE),
        "V" | "mV" | "uV" => (
            [1, 2, -3, -1, 0, 0, 0, 0],
            match name {
                "mV" => Rational::new(1, 1000).ok()?,
                "uV" => Rational::new(1, 1_000_000).ok()?,
                _ => Rational::ONE,
            },
        ),
        "Hz" | "kHz" => (
            [0, 0, -1, 0, 0, 0, 0, 0],
            Rational::integer(if name == "kHz" { 1000 } else { 1 }),
        ),
        "s" | "ms" | "us" | "ns" => (
            [0, 0, 1, 0, 0, 0, 0, 0],
            Rational::new(
                1,
                match name {
                    "ms" => 1000,
                    "us" => 1_000_000,
                    "ns" => 1_000_000_000,
                    _ => 1,
                },
            )
            .ok()?,
        ),
        "m" | "cm" | "mm" | "nm" => (
            [0, 1, 0, 0, 0, 0, 0, 0],
            Rational::new(
                1,
                match name {
                    "cm" => 100,
                    "mm" => 1000,
                    "nm" => 1_000_000_000,
                    _ => 1,
                },
            )
            .ok()?,
        ),
        "kg" => ([1, 0, 0, 0, 0, 0, 0, 0], Rational::ONE),
        "A" => ([0, 0, 0, 1, 0, 0, 0, 0], Rational::ONE),
        "K" => ([0, 0, 0, 0, 1, 0, 0, 0], Rational::ONE),
        "mol" => ([0, 0, 0, 0, 0, 1, 0, 0], Rational::ONE),
        "cd" => ([0, 0, 0, 0, 0, 0, 1, 0], Rational::ONE),
        "byte" | "KiB" | "MiB" | "MB" => (
            [0, 0, 0, 0, 0, 0, 0, 1],
            Rational::integer(match name {
                "KiB" => 1024,
                "MiB" => 1_048_576,
                "MB" => 1_000_000,
                _ => 1,
            }),
        ),
        _ => return None,
    };
    Some(Unit { dimension, scale })
}

impl FromStr for Unit {
    type Err = String;
    fn from_str(source: &str) -> Result<Self, String> {
        let text: String = source.chars().filter(|c| !c.is_whitespace()).collect();
        let mut result = Self::scalar();
        let mut start = 0;
        let mut divide = false;
        for end in (0..text.len())
            .filter(|&i| matches!(text.as_bytes()[i], b'*' | b'/'))
            .chain(std::iter::once(text.len()))
        {
            let factor = &text[start..end];
            let (name, exponent) = factor.split_once('^').unwrap_or((factor, "1"));
            let exponent: i16 = exponent.parse().map_err(|_| "단위 지수는 정수여야 한다.")?;
            if exponent.unsigned_abs() > 32 {
                return Err("단위 지수의 절댓값은 32 이하여야 한다.".into());
            }
            let unit =
                symbol(name).ok_or_else(|| format!("등록한 단위 기호를 사용한다: {name}"))?;
            result = result.multiply(&unit.power(if divide { -exponent } else { exponent })?)?;
            if end < text.len() {
                divide = text.as_bytes()[end] == b'/';
                start = end + 1;
            }
        }
        Ok(result)
    }
}

impl fmt::Display for Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for name in ["V^2", "uV^2", "mV^2", "V^2/Hz", "uV^2/Hz"] {
            if name.parse::<Unit>().as_ref().ok() == Some(self) {
                return f.write_str(name);
            }
        }
        for name in [
            "1", "V", "uV", "mV", "Hz", "kHz", "s", "ms", "us", "ns", "m", "byte", "KiB", "MiB",
            "MB",
        ] {
            if symbol(name).as_ref() == Some(self) {
                return f.write_str(name);
            }
        }
        let mut parts = Vec::new();
        for (name, exponent) in ["kg", "m", "s", "A", "K", "mol", "cd", "byte"]
            .into_iter()
            .zip(self.dimension)
        {
            if exponent != 0 {
                parts.push(format!("{name}^{exponent}"));
            }
        }
        if self.scale != Rational::ONE {
            parts.insert(0, self.scale.to_string());
        }
        f.write_str(&parts.join("*"))
    }
}
