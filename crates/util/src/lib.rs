//! Pure helper functions ported from `mininet/util.py`.
//!
//! Everything in this crate is free of root privileges, subprocess calls,
//! and kernel interaction so it can be unit- and differential-tested
//! anywhere. Only the *pure* helpers of `mininet/util.py` are ported here:
//!
//! - address formatting/parsing: [`colon_hex`] (Python `_colonHex`),
//!   [`mac_colon_hex`], [`ip_str`], [`ip_num`], [`ip_add`], [`ip_parse`],
//!   [`net_parse`]
//! - numeric checks: [`check_int`], [`check_float`], [`make_numeric`]
//! - natural sort keys: [`natural`], [`natural_seq`]
//! - misc: [`irange`], [`split_args`], [`unit_scale`], [`fmt_bps`],
//!   [`num_cores`], [`retry`], [`is_root`] (the canonical root check,
//!   Python's `ensureRoot()`, re-exported by `mn-net` and used by
//!   `mn-node`)
//!
//! # Fidelity notes / deliberate deviations from CPython
//!
//! - Python raises `ValueError`/`TypeError`/`AssertionError` on bad input;
//!   the Rust ports return `None` (or `false` for [`retry`], whose Python
//!   version calls `exit(1)` when it gives up).
//! - Python integers are arbitrary precision. This crate uses `i64`/`i128`;
//!   inputs whose numeric value exceeds those ranges are not replicated.
//! - Python `int()`/`float()`/`re` accept non-ASCII (Unicode) digits and
//!   whitespace; the ports accept ASCII digits/whitespace only.
//! - [`num_cores`] reads `/proc/cpuinfo` directly instead of shelling out to
//!   `grep -c processor /proc/cpuinfo` (same result, no subprocess).

mod addr;
mod args;
mod misc;
mod natural;
mod numeric;
mod scale;

pub use addr::{
    colon_hex, ip_add, ip_add_with, ip_num, ip_parse, ip_str, mac_colon_hex, net_parse,
    DEFAULT_IP_BASE_NUM, DEFAULT_PREFIX_LEN,
};
pub use args::{split_args, SplitArgs};
pub use misc::{irange, is_root, num_cores, retry};
pub use natural::{natural, natural_seq, NaturalPart};
pub use numeric::{check_float, check_int, make_numeric, python_float, python_int, Numeric};
// `percent_format`/`python_float_repr` stay crate-internal helpers of
// `scale` (only `fmt_bps_with` uses them); `PercentValue` remains public as
// the value type of the %-format engine.
pub use scale::{fmt_bps, fmt_bps_with, unit_scale, PercentValue, FMT_BPS_DEFAULT};
