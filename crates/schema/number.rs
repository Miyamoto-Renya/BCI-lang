//! 표본률과 정적 상수를 위한 checked 유리수 산술.

use std::{cmp::Ordering, fmt, str::FromStr};

/// 분모가 양수인 기약 유리수.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rational {
    numerator: i128,
    denominator: i128,
}

impl Rational {
    /// 0 이상 유리수를 올림한 usize를 반환한다.
    pub fn ceil_natural(self) -> Result<usize, String> {
        if self.numerator < 0 {
            return Err("올림 대상은 0 이상의 수치를 요구한다.".into());
        }
        let value =
            self.numerator / self.denominator + i128::from(self.numerator % self.denominator != 0);
        usize::try_from(value).map_err(|_| "올림 정수 범위를 초과했다.".into())
    }
    /// 영점.
    pub const ZERO: Self = Self::integer(0);
    /// 곱셈의 항등원.
    pub const ONE: Self = Self::integer(1);

    /// i64 정수를 정확히 표현한다.
    pub const fn integer(value: i64) -> Self {
        Self {
            numerator: value as i128,
            denominator: 1,
        }
    }

    /// 분수의 부호와 공약수를 정규화한다.
    pub fn new(mut numerator: i128, mut denominator: i128) -> Result<Self, String> {
        if denominator == 0 {
            return Err("분모는 0 이외의 값을 요구한다.".into());
        }
        if numerator == i128::MIN || denominator == i128::MIN {
            return Err("유리수 표현 범위를 초과했다.".into());
        }
        if denominator < 0 {
            numerator = -numerator;
            denominator = -denominator;
        }
        let divisor = gcd(numerator.abs(), denominator);
        Ok(Self {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        })
    }

    /// 양수 여부.
    pub fn is_positive(self) -> bool {
        self.numerator > 0
    }
    /// 0 여부.
    pub fn is_zero(self) -> bool {
        self.numerator == 0
    }
    /// 수치 커널 경계에서 IEEE 754 binary64로 반올림한다.
    pub fn to_f64(self) -> f64 {
        self.numerator as f64 / self.denominator as f64
    }
    /// 음수를 포함한 덧셈을 checked 산술로 계산한다.
    pub fn checked_add(self, other: Self) -> Result<Self, String> {
        let divisor = gcd(self.denominator, other.denominator);
        let left = other.denominator / divisor;
        let right = self.denominator / divisor;
        Self::new(
            checked(self.numerator.checked_mul(left).and_then(|x| {
                other
                    .numerator
                    .checked_mul(right)
                    .and_then(|y| x.checked_add(y))
            }))?,
            checked(self.denominator.checked_mul(left))?,
        )
    }
    /// 부호를 반전한다.
    pub fn negate(self) -> Result<Self, String> {
        Self::new(checked(self.numerator.checked_neg())?, self.denominator)
    }
    /// 곱셈 전에 교차 약분한다.
    pub fn multiply(self, other: Self) -> Result<Self, String> {
        let a = gcd(self.numerator.abs(), other.denominator);
        let b = gcd(other.numerator.abs(), self.denominator);
        Self::new(
            checked((self.numerator / a).checked_mul(other.numerator / b))?,
            checked((self.denominator / b).checked_mul(other.denominator / a))?,
        )
    }
    /// 0 이외의 값으로 나눈다.
    pub fn divide(self, other: Self) -> Result<Self, String> {
        self.multiply(Self::new(other.denominator, other.numerator)?)
    }
    /// 정수 거듭제곱을 계산한다.
    pub fn power(self, exponent: i16) -> Result<Self, String> {
        let mut value = Self::ONE;
        let base = if exponent < 0 {
            Self::ONE.divide(self)?
        } else {
            self
        };
        for _ in 0..exponent.unsigned_abs() {
            value = value.multiply(base)?;
        }
        Ok(value)
    }
    /// 정수 상수의 크기를 꺼낸다.
    pub fn natural(self) -> Result<usize, String> {
        if self.denominator != 1 || self.numerator < 0 {
            return Err("0 이상의 정수 인수를 요구한다.".into());
        }
        usize::try_from(self.numerator).map_err(|_| "정수 인수의 범위를 초과했다.".into())
    }
    /// 정확한 비교를 수행하고 중간 산술 범위를 검사한다.
    pub fn compare(self, other: Self) -> Result<Ordering, String> {
        let difference = self.checked_add(other.negate()?)?;
        Ok(difference.numerator.cmp(&0))
    }
}

fn gcd(mut a: i128, mut b: i128) -> i128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.max(1)
}

fn checked(value: Option<i128>) -> Result<i128, String> {
    value.ok_or_else(|| "유리수 계산 범위를 초과했다.".into())
}

impl FromStr for Rational {
    type Err = String;
    fn from_str(source: &str) -> Result<Self, String> {
        let text = source.replace('_', "");
        let (mantissa, exponent) = text.split_once(['e', 'E']).unwrap_or((&text, "0"));
        let exponent: i32 = exponent
            .parse()
            .map_err(|_| "숫자의 지수 형식을 확인한다.")?;
        let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
        let digits = format!("{whole}{fraction}");
        let numerator: i128 = digits
            .parse()
            .map_err(|_| "숫자의 형식 또는 표현 범위를 확인한다.")?;
        let power = i32::try_from(fraction.len())
            .map_err(|_| "소수 길이를 초과했다.")?
            .checked_sub(exponent)
            .ok_or("숫자의 지수 범위를 초과했다.")?;
        if power.unsigned_abs() > 38 {
            return Err("십진 지수의 절댓값은 38 이하여야 한다.".into());
        }
        let scale = checked(10_i128.checked_pow(power.unsigned_abs()))?;
        if power >= 0 {
            Self::new(numerator, scale)
        } else {
            Self::new(checked(numerator.checked_mul(scale))?, 1)
        }
    }
}

impl fmt::Display for Rational {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.denominator == 1 {
            write!(f, "{}", self.numerator)
        } else {
            write!(f, "{}/{}", self.numerator, self.denominator)
        }
    }
}
