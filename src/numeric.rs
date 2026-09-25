use std::error::Error;
use std::fmt;
use std::io::{Error as IoError, ErrorKind};
use std::str::FromStr;

use bytes::BytesMut;
use postgres_types::{Format, FromSql, IsNull, ToSql, Type, to_sql_checked};
use serde::{Deserialize, Deserializer, Serialize};

use crate::schema::SqlType;

/// A decimal string backed by PostgreSQL `numeric`, without a fixed scale or
/// floating-point conversion.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct NumericText(String);

impl NumericText {
    pub fn new(value: impl Into<String>) -> Result<Self, &'static str> {
        let mut value = value.into();
        let unsigned = value.strip_prefix('-').unwrap_or(&value);
        let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));

        if whole.is_empty()
            || !whole.bytes().all(|byte| byte.is_ascii_digit())
            || (unsigned.contains('.') && fraction.is_empty())
            || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err("expected a plain decimal string");
        }

        if value.starts_with('-')
            && whole.bytes().chain(fraction.bytes()).all(|byte| byte == b'0')
        {
            value.remove(0);
        }

        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for NumericText {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for NumericText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for NumericText {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl SqlType for NumericText {
    const SQL_TYPE: &'static str = "numeric";
    const NULLABLE: bool = false;
}

impl ToSql for NumericText {
    fn to_sql(&self, _: &Type, output: &mut BytesMut) -> Result<IsNull, Box<dyn Error + Sync + Send>> {
        let negative = self.0.starts_with('-');
        let unsigned = self.0.strip_prefix('-').unwrap_or(&self.0);
        let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
        let whole_padding = (4 - whole.len() % 4) % 4;
        let fraction_padding = (4 - fraction.len() % 4) % 4;
        let mut padded = String::with_capacity(
            whole_padding + whole.len() + fraction.len() + fraction_padding,
        );
        padded.extend(std::iter::repeat_n('0', whole_padding));
        padded.push_str(whole);
        padded.push_str(fraction);
        padded.extend(std::iter::repeat_n('0', fraction_padding));

        let mut digits = padded
            .as_bytes()
            .chunks_exact(4)
            .map(|chunk| std::str::from_utf8(chunk)
                .map_err(|_| IoError::new(ErrorKind::InvalidData, "invalid decimal digits"))?
                .parse::<i16>()
                .map_err(|_| IoError::new(ErrorKind::InvalidData, "invalid decimal digits")))
            .collect::<Result<Vec<_>, _>>()?;
        let whole_groups = (whole_padding + whole.len()) / 4;
        let mut weight = whole_groups as i32 - 1;

        while digits.first() == Some(&0) {
            digits.remove(0);
            weight -= 1;
        }

        while digits.last() == Some(&0) {
            digits.pop();
        }

        let count = i16::try_from(digits.len())
            .map_err(|_| IoError::new(ErrorKind::InvalidInput, "decimal has too many base-10000 digits"))?;
        let weight = if digits.is_empty() {
            0_i16
        } else {
            i16::try_from(weight)
                .map_err(|_| IoError::new(ErrorKind::InvalidInput, "decimal magnitude is unsupported"))?
        };
        let scale = u16::try_from(fraction.len())
            .map_err(|_| IoError::new(ErrorKind::InvalidInput, "decimal scale is unsupported"))?;
        let sign = if negative && digits.iter().any(|digit| *digit != 0) {
            0x4000_u16
        } else {
            0x0000_u16
        };

        output.extend_from_slice(&count.to_be_bytes());
        output.extend_from_slice(&weight.to_be_bytes());
        output.extend_from_slice(&sign.to_be_bytes());
        output.extend_from_slice(&scale.to_be_bytes());

        for digit in digits {
            output.extend_from_slice(&digit.to_be_bytes());
        }

        Ok(IsNull::No)
    }

    fn accepts(ty: &Type) -> bool {
        *ty == Type::NUMERIC
    }

    fn encode_format(&self, _: &Type) -> Format {
        Format::Binary
    }

    to_sql_checked!();
}

impl<'a> FromSql<'a> for NumericText {
    fn from_sql(_: &Type, raw: &'a [u8]) -> Result<Self, Box<dyn Error + Sync + Send>> {
        let invalid = || IoError::new(ErrorKind::InvalidData, "invalid PostgreSQL numeric payload");

        if raw.len() < 8 {
            return Err(invalid().into());
        }

        let count = i16::from_be_bytes([raw[0], raw[1]]);
        let weight = i16::from_be_bytes([raw[2], raw[3]]) as i32;
        let sign = u16::from_be_bytes([raw[4], raw[5]]);
        let scale = u16::from_be_bytes([raw[6], raw[7]]) as usize;

        if count < 0 || raw.len() != 8 + count as usize * 2 {
            return Err(invalid().into());
        }

        if !matches!(sign, 0x0000 | 0x4000) {
            return Err(invalid().into());
        }

        let digits = raw[8..]
            .chunks_exact(2)
            .map(|pair| i16::from_be_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();

        if digits.iter().any(|digit| !(0..10000).contains(digit)) {
            return Err(invalid().into());
        }

        let group = |exponent: i32| -> i16 {
            let index = weight - exponent;

            if index < 0 {
                return 0;
            }

            digits.get(index as usize).copied().unwrap_or(0)
        };
        let mut whole = String::new();

        if weight >= 0 {
            for exponent in (0..=weight).rev() {
                whole.push_str(&format!("{:04}", group(exponent)));
            }
        }

        let whole = whole.trim_start_matches('0');
        let mut result = if whole.is_empty() { "0".to_string() } else { whole.to_string() };

        if scale > 0 {
            result.push('.');
            let mut fraction = String::new();

            for exponent in 1..=scale.div_ceil(4) {
                fraction.push_str(&format!("{:04}", group(-(exponent as i32))));
            }

            result.push_str(&fraction[..scale]);
        }

        if sign == 0x4000 && digits.iter().any(|digit| *digit != 0) {
            result.insert(0, '-');
        }

        Ok(Self(result))
    }

    fn accepts(ty: &Type) -> bool {
        *ty == Type::NUMERIC
    }
}
