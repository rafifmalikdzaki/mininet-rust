//! Miscellaneous pure helpers from `mininet/util.py`: `irange`, `numCores`
//! and `retry`.

use std::ops::RangeInclusive;
use std::sync::OnceLock;
use std::time::Duration;

/// Port of `irange(start, end)`: inclusive range from `start` to `end`
/// (`irange(1, 5)` yields 1, 2, 3, 4, 5). Empty when `start > end`,
/// exactly like Python's `range(start, end + 1)`.
pub fn irange(start: i64, end: i64) -> RangeInclusive<i64> {
    start..=end
}

/// Port of `numCores()`: number of CPU cores based on `/proc/cpuinfo`.
///
/// The Python original shells out to `grep -c processor /proc/cpuinfo` and
/// returns 0 when that fails; this port reads the file directly (no
/// subprocess, no root needed) and counts the lines containing `processor`.
/// The result is cached like the Python function attribute cache.
pub fn num_cores() -> usize {
    static NCORES: OnceLock<usize> = OnceLock::new();
    *NCORES.get_or_init(|| {
        std::fs::read_to_string("/proc/cpuinfo")
            .map(|content| content.lines().filter(|l| l.contains("processor")).count())
            .unwrap_or(0)
    })
}

/// Port of `retry(retries, delaySecs, fn, *args, **kwargs)`.
///
/// Calls `f` until it returns `true` or `retries` attempts have been made
/// (note: like the Python original, `f` is always called at least once, and
/// `retries <= 0` always "gives up"). Returns `true` on success; returns
/// `false` where the Python original prints an error and calls `exit(1)`.
/// `f` returns `bool` here, whereas Python accepts any truthy value.
pub fn retry<F: FnMut() -> bool>(retries: i64, delay: Duration, mut f: F) -> bool {
    let mut tries: i64 = 0;
    while !f() && tries < retries {
        std::thread::sleep(delay);
        tries += 1;
    }
    // Python: if tries >= retries: error(...); exit(1)
    tries < retries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn irange_cases() {
        assert_eq!(irange(1, 5).collect::<Vec<_>>(), vec![1, 2, 3, 4, 5]);
        assert_eq!(irange(5, 1).collect::<Vec<_>>(), Vec::<i64>::new());
        assert_eq!(irange(3, 3).collect::<Vec<_>>(), vec![3]);
        assert_eq!(irange(-3, 1).collect::<Vec<_>>(), vec![-3, -2, -1, 0, 1]);
    }

    #[test]
    fn num_cores_matches_proc_cpuinfo() {
        let expected = std::fs::read_to_string("/proc/cpuinfo")
            .map(|c| c.lines().filter(|l| l.contains("processor")).count())
            .unwrap_or(0);
        assert_eq!(num_cores(), expected);
        // cached: second call returns the same value
        assert_eq!(num_cores(), expected);
    }

    #[test]
    fn retry_cases() {
        // succeeds on the 3rd call
        let mut calls = 0;
        let ok = retry(3, Duration::ZERO, || {
            calls += 1;
            calls > 2
        });
        assert!(ok);
        assert_eq!(calls, 3);

        // always fails: f is called retries + 1 times, then gives up
        let mut calls = 0;
        let ok = retry(2, Duration::ZERO, || {
            calls += 1;
            false
        });
        assert!(!ok);
        assert_eq!(calls, 3);

        // retries = 0 gives up even when f returns true (Python exit(1))
        let mut calls = 0;
        let ok = retry(0, Duration::ZERO, || {
            calls += 1;
            true
        });
        assert!(!ok);
        assert_eq!(calls, 1);

        // negative retries also give up after the first call
        let mut calls = 0;
        let ok = retry(-1, Duration::ZERO, || {
            calls += 1;
            true
        });
        assert!(!ok);
        assert_eq!(calls, 1);

        // immediate success
        let ok = retry(5, Duration::ZERO, || true);
        assert!(ok);
    }
}
