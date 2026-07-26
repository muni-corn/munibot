use crate::value::ConfigValue;

/// Checks a numeric `ConfigValue` (`Int`, `Float`, or `Duration`) against
/// optional bounds, returning an error message if either is violated.
///
/// Non-numeric values (including `Unset`, e.g. an `Option<T>` field left
/// unset) are never in violation -- there's nothing to check.
///
/// This is what `#[derive(Configurable)]`'s generated `validate()` calls
/// for a field with `#[config(min = ...)]`/`#[config(max = ...)]`, rather
/// than embedding the comparison in generated code directly: it keeps the
/// actual logic in one normal, testable Rust function instead of
/// duplicated per field in a proc macro's token stream.
pub fn validate_numeric_bounds(
    value: &ConfigValue,
    min: Option<f64>,
    max: Option<f64>,
) -> Option<String> {
    let n = match value {
        ConfigValue::Int(n) => *n as f64,
        ConfigValue::Float(f) => *f,
        ConfigValue::Duration(secs) => *secs as f64,
        _ => return None,
    };

    if let Some(min) = min
        && n < min
    {
        return Some(format!("must be at least {min}"));
    }
    if let Some(max) = max
        && n > max
    {
        return Some(format!("must be at most {max}"));
    }

    None
}

/// Checks a `ConfigValue::Text`'s character count against a maximum,
/// returning an error message if it's violated.
///
/// Non-text values are never in violation.
pub fn validate_text_length(value: &ConfigValue, max_len: Option<usize>) -> Option<String> {
    let ConfigValue::Text(s) = value else {
        return None;
    };

    if let Some(max_len) = max_len {
        let len = s.chars().count();
        if len > max_len {
            return Some(format!("must be at most {max_len} characters, is {len}"));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{validate_numeric_bounds, validate_text_length};
    use crate::value::ConfigValue;

    #[test]
    fn test_numeric_bounds_accepts_a_value_within_range() {
        assert_eq!(
            validate_numeric_bounds(&ConfigValue::Int(50), Some(0.0), Some(100.0)),
            None
        );
    }

    #[test]
    fn test_numeric_bounds_rejects_a_value_below_min() {
        let message = validate_numeric_bounds(&ConfigValue::Int(-1), Some(0.0), None);
        assert!(message.is_some());
    }

    #[test]
    fn test_numeric_bounds_rejects_a_value_above_max() {
        let message = validate_numeric_bounds(&ConfigValue::Duration(999999), None, Some(604800.0));
        assert!(message.is_some());
    }

    #[test]
    fn test_numeric_bounds_treats_float_and_duration_the_same_as_int() {
        assert!(validate_numeric_bounds(&ConfigValue::Float(-0.5), Some(0.0), None).is_some());
        assert!(validate_numeric_bounds(&ConfigValue::Duration(10), Some(60.0), None).is_some());
    }

    #[test]
    fn test_numeric_bounds_ignores_non_numeric_values() {
        assert_eq!(
            validate_numeric_bounds(&ConfigValue::Unset, Some(100.0), None),
            None
        );
        assert_eq!(
            validate_numeric_bounds(&ConfigValue::Bool(true), Some(100.0), None),
            None
        );
    }

    #[test]
    fn test_numeric_bounds_with_no_constraints_never_fails() {
        assert_eq!(
            validate_numeric_bounds(&ConfigValue::Int(i64::MIN), None, None),
            None
        );
    }

    #[test]
    fn test_text_length_accepts_a_short_string() {
        assert_eq!(
            validate_text_length(&ConfigValue::Text("hi".to_owned()), Some(10)),
            None
        );
    }

    #[test]
    fn test_text_length_rejects_a_too_long_string() {
        let message = validate_text_length(&ConfigValue::Text("hello world".to_owned()), Some(5));
        assert!(message.is_some());
    }

    #[test]
    fn test_text_length_ignores_non_text_values() {
        assert_eq!(validate_text_length(&ConfigValue::Int(1), Some(1)), None);
    }

    #[test]
    fn test_text_length_counts_unicode_scalar_values_not_bytes() {
        // 5 characters, more than 5 bytes -- must not reject this at max_len 5.
        assert_eq!(
            validate_text_length(&ConfigValue::Text("münib".to_owned()), Some(5)),
            None
        );
    }
}
