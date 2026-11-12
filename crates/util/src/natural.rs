//! Natural ("human") sort keys, ported from `natural` and `naturalSeq` of
//! `mininet/util.py`.
//!
//! Python splits on `re.split(r'(\d+)', str(text))` and converts digit runs
//! with `int()`. The port splits on runs of ASCII digits (CPython's `\d`
//! also matches non-ASCII Unicode digits, which are not replicated) and
//! preserves the empty text segments at the boundaries exactly, e.g.
//! `natural("a1") == [Text("a"), Num(1), Text("")]`.
//!
//! Note: comparing a `Num` with a `Text` raises `TypeError` in Python 3;
//! here [`NaturalPart`] has a total order with `Num` sorting before `Text`.

/// One segment of a [`natural`] key: either an integer (a run of digits)
/// or a text fragment.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum NaturalPart {
    Num(i128),
    Text(String),
}

/// Port of `natural(text)`: sort key for sane/alphabetical sorting,
/// e.g. `sorted(l, key=natural)`.
pub fn natural<T: ToString>(text: T) -> Vec<NaturalPart> {
    let s = text.to_string();
    let mut parts = Vec::new();
    let mut rest = s.as_str();
    loop {
        match rest.find(|c: char| c.is_ascii_digit()) {
            None => {
                parts.push(NaturalPart::Text(rest.to_string()));
                break;
            }
            Some(start) => {
                parts.push(NaturalPart::Text(rest[..start].to_string()));
                let run_len = rest[start..]
                    .find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(rest.len() - start);
                let end = start + run_len;
                parts.push(NaturalPart::Num(parse_num(&rest[start..end])));
                rest = &rest[end..];
            }
        }
    }
    parts
}

/// Digit runs are parsed as integers; runs too large for `i128` saturate
/// (CPython would produce an exact big int).
fn parse_num(digits: &str) -> i128 {
    digits.parse::<i128>().unwrap_or(i128::MAX)
}

/// Port of `naturalSeq(t)`: natural sort key for a sequence of items.
pub fn natural_seq<T: ToString>(items: impl IntoIterator<Item = T>) -> Vec<Vec<NaturalPart>> {
    items.into_iter().map(natural).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> NaturalPart {
        NaturalPart::Text(s.to_string())
    }

    #[test]
    fn natural_cases() {
        assert_eq!(
            natural("a1b2"),
            vec![
                text("a"),
                NaturalPart::Num(1),
                text("b"),
                NaturalPart::Num(2),
                text("")
            ]
        );
        assert_eq!(
            natural("1a"),
            vec![text(""), NaturalPart::Num(1), text("a")]
        );
        assert_eq!(
            natural("a1"),
            vec![text("a"), NaturalPart::Num(1), text("")]
        );
        assert_eq!(natural(""), vec![text("")]);
        assert_eq!(natural(5), vec![text(""), NaturalPart::Num(5), text("")]);
        assert_eq!(
            natural("a01b002"),
            vec![
                text("a"),
                NaturalPart::Num(1),
                text("b"),
                NaturalPart::Num(2),
                text("")
            ]
        );
        assert_eq!(
            natural("10"),
            vec![text(""), NaturalPart::Num(10), text("")]
        );
    }

    #[test]
    fn natural_ordering() {
        let mut v = vec!["a10", "a2", "a1", "b", "10", "9"];
        v.sort_by_key(|s| natural(s));
        assert_eq!(v, vec!["9", "10", "a1", "a2", "a10", "b"]);
    }

    #[test]
    fn natural_seq_cases() {
        assert_eq!(
            natural_seq(["a1", "a10", "a2"]),
            vec![
                vec![text("a"), NaturalPart::Num(1), text("")],
                vec![text("a"), NaturalPart::Num(10), text("")],
                vec![text("a"), NaturalPart::Num(2), text("")],
            ]
        );
        // non-string items are stringified first, like Python's str(text)
        assert_eq!(
            natural_seq([5]),
            vec![vec![text(""), NaturalPart::Num(5), text("")]]
        );
    }
}
