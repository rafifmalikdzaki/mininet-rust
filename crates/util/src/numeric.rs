//! Numeric string checks/conversion, ported from `checkInt`, `checkFloat`
//! and `makeNumeric` of `mininet/util.py`.
//!
//! [`python_int`] and [`python_float`] replicate the acceptance rules of
//! CPython's `int(s)` / `float(s)` for `str` input: surrounding whitespace,
//! an optional sign, and single underscores *between* digits are allowed
//! (e.g. `"1_0"` -> 10, `"1_0.5"` -> 10.5). Non-ASCII (Unicode) digits,
//! which CPython also accepts, are not supported.

/// A value produced by [`make_numeric`]: an `int`, a `float`, or the
/// original string when it is neither (Python's `int | float | str`).
#[derive(Debug, Clone, PartialEq)]
pub enum Numeric {
    Int(i64),
    Float(f64),
    Text(String),
}

/// Parse a string with CPython `int()` semantics (ASCII digits, optional
/// sign, surrounding whitespace, single underscores between digits).
/// Returns `None` where Python raises `ValueError`, or when the value does
/// not fit in an `i64` (CPython would return a big int).
pub fn python_int(s: &str) -> Option<i64> {
    let t = s.trim();
    let body = t.strip_prefix(|c| c == '+' || c == '-').unwrap_or(t);
    if !valid_int_body(body) {
        return None;
    }
    let cleaned: String = t.chars().filter(|&c| c != '_').collect();
    cleaned.parse::<i64>().ok()
}

/// `body` is the string after the optional sign.
fn valid_int_body(body: &str) -> bool {
    let mut prev_digit = false;
    let mut prev_underscore = false;
    for c in body.chars() {
        if c == '_' {
            if !prev_digit || prev_underscore {
                return false;
            }
            prev_underscore = true;
            prev_digit = false;
        } else if c.is_ascii_digit() {
            prev_digit = true;
            prev_underscore = false;
        } else {
            return false;
        }
    }
    // rejects empty bodies and trailing underscores
    prev_digit
}

/// Parse a string with CPython `float()` semantics: like [`python_int`] but
/// also allowing a decimal point, an exponent, and the case-insensitive
/// literals `inf`, `infinity` and `nan`. Overflow yields `inf` exactly like
/// CPython (`float("1e999")`). Returns `None` where Python raises
/// `ValueError`.
pub fn python_float(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    let body = t.strip_prefix(|c| c == '+' || c == '-').unwrap_or(t);
    let lower = body.to_ascii_lowercase();
    if lower == "inf" || lower == "infinity" || lower == "nan" {
        // no underscores can be present in these literals
        return t.parse::<f64>().ok();
    }
    if !valid_float_body(body) {
        return None;
    }
    let cleaned: String = t.chars().filter(|&c| c != '_').collect();
    cleaned.parse::<f64>().ok()
}

/// `body` is the string after the optional sign: a decimal mantissa with at
/// least one digit and at most one `.`, optionally followed by an
/// exponent. Underscores are only allowed between digits.
fn valid_float_body(body: &str) -> bool {
    let chars: Vec<char> = body.chars().collect();
    let mut i = 0usize;
    let mut mantissa_digits = 0usize;
    let mut seen_dot = false;
    // previous char category: 'd' digit, '_' underscore, '.' dot, ' ' start
    let mut prev = ' ';
    while i < chars.len() && chars[i] != 'e' && chars[i] != 'E' {
        let c = chars[i];
        if c == '.' {
            if seen_dot || prev == '_' {
                return false;
            }
            seen_dot = true;
            prev = '.';
        } else if c == '_' {
            if prev != 'd' {
                return false;
            }
            prev = '_';
        } else if c.is_ascii_digit() {
            mantissa_digits += 1;
            prev = 'd';
        } else {
            return false;
        }
        i += 1;
    }
    if mantissa_digits == 0 || prev == '_' {
        return false;
    }
    if i < chars.len() {
        // exponent part
        i += 1; // skip e/E
        if i < chars.len() && (chars[i] == '+' || chars[i] == '-') {
            i += 1;
        }
        let mut exp_digits = 0usize;
        let mut prev = ' ';
        while i < chars.len() {
            let c = chars[i];
            if c == '_' {
                if prev != 'd' {
                    return false;
                }
                prev = '_';
            } else if c.is_ascii_digit() {
                exp_digits += 1;
                prev = 'd';
            } else {
                return false;
            }
            i += 1;
        }
        if exp_digits == 0 || prev == '_' {
            return false;
        }
    }
    true
}

/// Port of `checkInt(s)`: is `s` an int?
pub fn check_int(s: &str) -> bool {
    python_int(s).is_some()
}

/// Port of `checkFloat(s)`: is `s` a float?
pub fn check_float(s: &str) -> bool {
    python_float(s).is_some()
}

/// Port of `makeNumeric(s)`: convert to int or float if numeric, else return
/// the original string.
pub fn make_numeric(s: &str) -> Numeric {
    if let Some(v) = python_int(s) {
        Numeric::Int(v)
    } else if let Some(v) = python_float(s) {
        Numeric::Float(v)
    } else {
        Numeric::Text(s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_int_cases() {
        assert_eq!(python_int("1"), Some(1));
        assert_eq!(python_int("-1"), Some(-1));
        assert_eq!(python_int("+1"), Some(1));
        assert_eq!(python_int(" 12 "), Some(12));
        assert_eq!(python_int("\t12\n"), Some(12));
        assert_eq!(python_int("1_0"), Some(10));
        assert_eq!(python_int("007"), Some(7));
        assert_eq!(python_int("-0"), Some(0));
        assert_eq!(python_int("9223372036854775807"), Some(i64::MAX));
        assert_eq!(python_int("-9223372036854775808"), Some(i64::MIN));
        assert_eq!(python_int("1__0"), None);
        assert_eq!(python_int("_1"), None);
        assert_eq!(python_int("1_"), None);
        assert_eq!(python_int("0x10"), None);
        assert_eq!(python_int(""), None);
        assert_eq!(python_int("+ 1"), None);
        assert_eq!(python_int("1.5"), None);
        assert_eq!(python_int("abc"), None);
    }

    #[test]
    fn python_float_cases() {
        assert_eq!(python_float("1.5"), Some(1.5));
        assert_eq!(python_float(".5"), Some(0.5));
        assert_eq!(python_float("5."), Some(5.0));
        assert_eq!(python_float("1.e5"), Some(100000.0));
        assert_eq!(python_float("1e5"), Some(100000.0));
        assert_eq!(python_float("1E5"), Some(100000.0));
        assert_eq!(python_float("1e+5"), Some(100000.0));
        assert_eq!(python_float("1.5e-3"), Some(0.0015));
        assert_eq!(python_float("1_0.5"), Some(10.5));
        assert_eq!(python_float("1_0e1_0"), Some(10e10));
        assert_eq!(python_float(" 1.5 "), Some(1.5));
        assert_eq!(python_float("+.5"), Some(0.5));
        assert_eq!(python_float("inf"), Some(f64::INFINITY));
        assert_eq!(python_float("Inf"), Some(f64::INFINITY));
        assert_eq!(python_float("-INF"), Some(f64::NEG_INFINITY));
        assert_eq!(python_float("infinity"), Some(f64::INFINITY));
        assert!(python_float("+nan").unwrap().is_nan());
        assert!(python_float("NaN").unwrap().is_nan());
        assert_eq!(python_float("1e999"), Some(f64::INFINITY));
        assert_eq!(python_float("-1e999"), Some(f64::NEG_INFINITY));
        assert_eq!(python_float("00.5"), Some(0.5));
        assert_eq!(python_float("0.1e1"), Some(1.0));
        assert_eq!(python_float("1.5_"), None);
        assert_eq!(python_float("1_.5"), None);
        assert_eq!(python_float("1._5"), None);
        assert_eq!(python_float("1.5_e3"), None);
        assert_eq!(python_float("1e"), None);
        assert_eq!(python_float("1e+"), None);
        assert_eq!(python_float("1e_10"), None);
        assert_eq!(python_float("e5"), None);
        assert_eq!(python_float("."), None);
        assert_eq!(python_float("1.2.3"), None);
        assert_eq!(python_float(""), None);
        assert_eq!(python_float("+ 1"), None);
        assert_eq!(python_float("i_nf"), None);
    }

    #[test]
    fn check_cases() {
        assert!(check_int("1_0"));
        assert!(check_int(" 12 "));
        assert!(check_int("+3"));
        assert!(!check_int("1.5"));
        assert!(!check_int("abc"));
        assert!(check_float("1_0.5"));
        assert!(check_float("inf"));
        assert!(check_float("1e999"));
        assert!(check_float("nan"));
        assert!(!check_float("1__0.5"));
    }

    #[test]
    fn make_numeric_cases() {
        assert_eq!(make_numeric("1_0"), Numeric::Int(10));
        assert_eq!(make_numeric("-0"), Numeric::Int(0));
        assert_eq!(make_numeric(" 1.5 "), Numeric::Float(1.5));
        assert_eq!(make_numeric("1e999"), Numeric::Float(f64::INFINITY));
        assert_eq!(make_numeric("abc"), Numeric::Text("abc".to_string()));
        assert_eq!(make_numeric(""), Numeric::Text(String::new()));
        // int wins over float
        assert_eq!(make_numeric("5"), Numeric::Int(5));
    }
}
