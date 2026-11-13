//! Argument-string splitting, ported from `splitArgs` of `mininet/util.py`.

use crate::numeric::{make_numeric, Numeric};

/// Result of [`split_args`]: the function name, positional args and keyword
/// args. `kwargs` preserves Python dict semantics: a repeated key keeps its
/// first insertion position but takes the last value.
#[derive(Debug, Clone, PartialEq)]
pub struct SplitArgs {
    pub func: String,
    pub args: Vec<Numeric>,
    pub kwargs: Vec<(String, Numeric)>,
}

impl SplitArgs {
    /// Look up a keyword argument.
    pub fn kwarg(&self, key: &str) -> Option<&Numeric> {
        self.kwargs.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
}

/// Port of `splitArgs(argstr)`: split `fn,arg2,kw1=arg3...` into
/// `(fn, args, kwargs)` where args/kwargs values pass through
/// [`make_numeric`]. Any parameter containing `=` is a keyword argument;
/// the value is split at the *first* `=` only.
pub fn split_args(argstr: &str) -> SplitArgs {
    let mut parts = argstr.split(',');
    let func = parts.next().unwrap_or("").to_string();
    let mut args = Vec::new();
    let mut kwargs: Vec<(String, Numeric)> = Vec::new();
    for p in parts {
        if p.contains('=') {
            let (key, val) = p.split_once('=').expect("contains '='");
            let val = make_numeric(val);
            if let Some(slot) = kwargs.iter_mut().find(|(k, _)| k == key) {
                slot.1 = val;
            } else {
                kwargs.push((key.to_string(), val));
            }
        } else {
            args.push(make_numeric(p));
        }
    }
    SplitArgs { func, args, kwargs }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::numeric::Numeric::{Float, Int, Text};

    #[test]
    fn split_args_cases() {
        let r = split_args("fn,1,2.5,kw=3,x=y");
        assert_eq!(r.func, "fn");
        assert_eq!(r.args, vec![Int(1), Float(2.5)]);
        assert_eq!(
            r.kwargs,
            vec![
                ("kw".to_string(), Int(3)),
                ("x".to_string(), Text("y".to_string()))
            ]
        );

        // repeated key: last value wins, first position kept
        let r = split_args("fn,a=1,a=2");
        assert_eq!(r.kwargs, vec![("a".to_string(), Int(2))]);

        assert_eq!(split_args("").func, "");
        assert!(split_args("").args.is_empty());
        assert!(split_args("").kwargs.is_empty());

        let r = split_args("fn,=x");
        assert_eq!(r.kwargs, vec![("".to_string(), Text("x".to_string()))]);

        // value split at first '=' only
        let r = split_args("fn,a=b=c");
        assert_eq!(r.kwargs, vec![("a".to_string(), Text("b=c".to_string()))]);

        let r = split_args("fn, 1 ,x= 2 ");
        assert_eq!(r.args, vec![Int(1)]);
        assert_eq!(r.kwargs, vec![("x".to_string(), Int(2))]);

        // trailing comma produces an empty positional arg (stays text)
        let r = split_args("fn,");
        assert_eq!(r.args, vec![Text(String::new())]);

        let r = split_args("fn");
        assert!(r.args.is_empty() && r.kwargs.is_empty());
    }

    #[test]
    fn kwarg_lookup() {
        let r = split_args("fn,1,kw=3");
        assert_eq!(r.kwarg("kw"), Some(&Int(3)));
        assert_eq!(r.kwarg("missing"), None);
    }
}
