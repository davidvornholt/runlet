use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DurationParseError {
    #[error("duration must not be empty")]
    Empty,
    #[error("duration {0} is invalid")]
    Invalid(String),
    #[error("duration unit in {0} is unsupported; use s, m, or h")]
    UnsupportedUnit(String),
}

pub fn parse_duration(value: &str) -> Result<Duration, DurationParseError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(DurationParseError::Empty);
    }

    let split_at = value
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(value.len());
    if split_at == 0 || split_at == value.len() {
        return Err(DurationParseError::Invalid(value.to_string()));
    }

    let amount = value[..split_at]
        .parse::<u64>()
        .map_err(|_| DurationParseError::Invalid(value.to_string()))?;
    match &value[split_at..] {
        "s" => Ok(Duration::from_secs(amount)),
        "m" => amount
            .checked_mul(60)
            .map(Duration::from_secs)
            .ok_or_else(|| DurationParseError::Invalid(value.to_string())),
        "h" => amount
            .checked_mul(60 * 60)
            .map(Duration::from_secs)
            .ok_or_else(|| DurationParseError::Invalid(value.to_string())),
        _ => Err(DurationParseError::UnsupportedUnit(value.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_supported_duration_units() {
        assert_eq!(parse_duration("20s").unwrap(), Duration::from_secs(20));
        assert_eq!(parse_duration("20m").unwrap(), Duration::from_secs(1200));
        assert_eq!(parse_duration("2h").unwrap(), Duration::from_secs(7200));
    }

    #[test]
    fn rejects_missing_unit() {
        assert_eq!(
            parse_duration("20").unwrap_err(),
            DurationParseError::Invalid("20".to_string())
        );
    }

    #[test]
    fn rejects_overflowing_duration() {
        let value = format!("{}h", u64::MAX);
        assert_eq!(
            parse_duration(&value).unwrap_err(),
            DurationParseError::Invalid(value)
        );
    }
}
