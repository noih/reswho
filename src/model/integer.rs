use serde::Deserialize;
use serde_json::Number;

fn exact_integer<T: TryFrom<i128>>(number: Number) -> Result<T, &'static str> {
    let (negative, digits, position) = crate::settings::decimal(&number);
    if digits.is_empty() {
        return T::try_from(0).map_err(|_| "integer_out_of_range");
    }
    if position < digits.len() as i64 || position > 20 {
        return Err("invalid_integer");
    }
    let mut value = digits.parse::<u64>().map_err(|_| "integer_out_of_range")?;
    for _ in digits.len()..position as usize {
        value = value.checked_mul(10).ok_or("integer_out_of_range")?;
    }
    let value = if negative {
        -i128::from(value)
    } else {
        i128::from(value)
    };
    T::try_from(value).map_err(|_| "integer_out_of_range")
}

pub(crate) fn integer<'de, D: serde::Deserializer<'de>, T: TryFrom<i128>>(
    deserializer: D,
) -> Result<T, D::Error> {
    exact_integer(Number::deserialize(deserializer)?).map_err(serde::de::Error::custom)
}

pub(crate) fn optional_integer<'de, D: serde::Deserializer<'de>, T: TryFrom<i128>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::<Number>::deserialize(deserializer)?
        .map(exact_integer)
        .transpose()
        .map_err(serde::de::Error::custom)
}

#[cfg(test)]
mod tests {
    #[test]
    fn internal_identity_and_resource_numbers_keep_exact_integer_bounds() {
        use crate::{extensions::ProcessIdentity, model::ResourceConfiguration};
        for (number, expected) in [
            ("18446744073709551615", u64::MAX),
            ("9007199254740993.0", 9_007_199_254_740_993),
            ("1e3", 1000),
        ] {
            let value: ProcessIdentity =
                serde_json::from_str(&format!(r#"{{"pid":1,"started_unix_us":{number}}}"#))
                    .unwrap();
            assert_eq!(value.started_unix_us, expected);
        }
        for number in ["-1", "1.5", "18446744073709551616", "1e30"] {
            assert!(
                serde_json::from_str::<ProcessIdentity>(&format!(
                    r#"{{"pid":1,"started_unix_us":{number}}}"#
                ))
                .is_err()
            );
            assert!(
                serde_json::from_str::<ResourceConfiguration>(&format!(
                    r#"{{"scope":"vm","memory_bytes":{number}}}"#
                ))
                .is_err()
            );
        }
        let empty: ResourceConfiguration = serde_json::from_str(r#"{"scope":"vm"}"#).unwrap();
        assert!(empty.memory_bytes.is_none());
    }
}
