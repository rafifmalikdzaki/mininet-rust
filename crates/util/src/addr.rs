//! IP and MAC address formatting/parsing, ported from the corresponding
//! functions of `mininet/util.py`.
//!
//! IP "numbers" are represented as `i64` (not `u32`) so that the exact
//! two's-complement behavior of the Python original is preserved for
//! out-of-range components, e.g. `ipParse("1.2.3.-4") == -4` and
//! `ipStr(-1) == "255.255.255.255"`.

use crate::numeric::python_int;

/// Default `prefixLen` of Python `ipAdd`.
pub const DEFAULT_PREFIX_LEN: i64 = 8;
/// Default `ipBaseNum` of Python `ipAdd` (10.0.0.0).
pub const DEFAULT_IP_BASE_NUM: i64 = 0x0A00_0000;

/// Port of `_colonHex(val, bytecount)`: generate a colon-hex string.
///
/// `val` is treated as an unsigned/signed integer whose bytes are emitted
/// most-significant first, two lowercase hex digits each, joined by `':'`.
/// Bytes beyond the width of `i64` behave as in Python's two's-complement
/// semantics for the range `i64` covers.
pub fn colon_hex(val: i64, bytecount: usize) -> String {
    let mut pieces = Vec::with_capacity(bytecount);
    for i in (0..bytecount).rev() {
        let shift = i * 8;
        let byte = if shift >= 64 {
            // Python shifts arbitrary-precision ints; for shift >= 64 the
            // result is 0xff for negative values (arithmetic shift of -1)
            // and 0 for non-negative values within i64 range.
            if val < 0 {
                0xff
            } else {
                0
            }
        } else {
            ((val >> shift) & 0xff) as u8
        };
        pieces.push(format!("{byte:02x}"));
    }
    pieces.join(":")
}

/// Port of `macColonHex(mac)`: MAC colon-hex string from an unsigned int.
pub fn mac_colon_hex(mac: i64) -> String {
    colon_hex(mac, 6)
}

/// Port of `ipStr(ip)`: IP address string `w.x.y.z` from an unsigned int of
/// the form `w << 24 | x << 16 | y << 8 | z` (each byte masked with 0xff).
pub fn ip_str(ip: i64) -> String {
    format!(
        "{}.{}.{}.{}",
        (ip >> 24) & 0xff,
        (ip >> 16) & 0xff,
        (ip >> 8) & 0xff,
        ip & 0xff
    )
}

/// Port of `ipNum(w, x, y, z)`: `w << 24 | x << 16 | y << 8 | z`.
pub fn ip_num(w: i64, x: i64, y: i64, z: i64) -> i64 {
    (w << 24) | (x << 16) | (y << 8) | z
}

/// Port of `ipAdd(i, prefixLen=8, ipBaseNum=0x0a000000)`.
///
/// Returns `None` where Python raises: `AssertionError` when
/// `i > 0xffffffff >> prefixLen`, and `ValueError` for a negative
/// `prefixLen` ("negative shift count").
pub fn ip_add(i: i64) -> Option<String> {
    ip_add_with(i, DEFAULT_PREFIX_LEN, DEFAULT_IP_BASE_NUM)
}

/// Port of `ipAdd(i, prefixLen, ipBaseNum)` with explicit arguments.
pub fn ip_add_with(i: i64, prefix_len: i64, ip_base_num: i64) -> Option<String> {
    if prefix_len < 0 {
        // Python: 0xffffffff >> negative -> ValueError: negative shift count
        return None;
    }
    let imax = if prefix_len >= 64 {
        0
    } else {
        0xFFFF_FFFF_i64 >> prefix_len
    };
    if i > imax {
        // Python: assert i <= imax, 'Not enough IP addresses in the subnet'
        return None;
    }
    let mask = 0xFFFF_FFFF_i64 ^ imax;
    let ipnum = (ip_base_num & mask).wrapping_add(i);
    Some(ip_str(ipnum))
}

/// Port of `ipParse(ip)`: parse an IP address into an unsigned int.
///
/// Missing trailing components are filled with `0` just before the last
/// component (so `"1.2"` -> `1.0.0.2`), exactly like the Python original.
/// Returns `None` where Python raises `ValueError` (non-integer component)
/// or `TypeError` (more than four components).
pub fn ip_parse(ip: &str) -> Option<i64> {
    let mut args: Vec<i64> = Vec::new();
    for part in ip.split('.') {
        args.push(python_int(part)?);
    }
    if args.len() > 4 {
        // Python: ipNum(*args) -> TypeError
        return None;
    }
    while args.len() < 4 {
        let n = args.len();
        args.insert(n - 1, 0);
    }
    Some(ip_num(args[0], args[1], args[2], args[3]))
}

/// Port of `netParse(ipstr)`: parse an IP network specification into
/// `(address, prefixLen)`. Defaults to prefix length 24 when no `/` is
/// present. Returns `None` where Python raises `ValueError`.
pub fn net_parse(ipstr: &str) -> Option<(i64, i64)> {
    if ipstr.contains('/') {
        let mut parts = ipstr.split('/');
        let ip = parts.next()?;
        let pf = parts.next()?;
        if parts.next().is_some() {
            // Python: ip, pf = ipstr.split('/') -> ValueError on 3+ parts
            return None;
        }
        let prefix_len = python_int(pf)?;
        Some((ip_parse(ip)?, prefix_len))
    } else {
        Some((ip_parse(ipstr)?, 24))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colon_hex_cases() {
        assert_eq!(colon_hex(0xdeadbeef, 4), "de:ad:be:ef");
        assert_eq!(colon_hex(0x123, 6), "00:00:00:00:01:23");
        assert_eq!(colon_hex(-1, 2), "ff:ff");
        assert_eq!(colon_hex(0xff, 1), "ff");
        assert_eq!(colon_hex(0, 0), "");
        assert_eq!(colon_hex(0xab, 0), "");
        assert_eq!(colon_hex(-256, 2), "ff:00");
    }

    #[test]
    fn mac_colon_hex_cases() {
        assert_eq!(mac_colon_hex(0x000a0b0c0d0e), "00:0a:0b:0c:0d:0e");
        assert_eq!(mac_colon_hex(0), "00:00:00:00:00:00");
        assert_eq!(mac_colon_hex(0xffff_ffff_ffff), "ff:ff:ff:ff:ff:ff");
    }

    #[test]
    fn ip_str_cases() {
        assert_eq!(ip_str(0x0a000001), "10.0.0.1");
        assert_eq!(ip_str(-1), "255.255.255.255");
        assert_eq!(ip_str(1 << 32), "0.0.0.0");
        assert_eq!(ip_str(0), "0.0.0.0");
    }

    #[test]
    fn ip_num_cases() {
        assert_eq!(ip_num(10, 0, 0, 1), 167772161);
        assert_eq!(ip_num(256, 0, 0, 0), 1 << 32);
        assert_eq!(ip_num(1, 2, 3, -4), -4);
    }

    #[test]
    fn ip_add_cases() {
        assert_eq!(ip_add(1).as_deref(), Some("10.0.0.1"));
        assert_eq!(ip_add(255).as_deref(), Some("10.0.0.255"));
        assert_eq!(ip_add(256).as_deref(), Some("10.0.1.0")); // fits in /8
        assert_eq!(ip_add(0x1000000), None); // AssertionError in Python: i > 0xffffff
        assert_eq!(ip_add_with(0, 33, 0x0a000000).as_deref(), Some("10.0.0.0"));
        assert_eq!(ip_add_with(0, 0, 0x0a000000).as_deref(), Some("0.0.0.0"));
        assert_eq!(
            ip_add_with(-1, 8, 0x0a000000).as_deref(),
            Some("9.255.255.255")
        );
        assert_eq!(ip_add_with(1, -1, 0x0a000000), None); // negative shift count
        assert_eq!(ip_add_with(1, 64, 0x0a000000), None); // imax == 0
        assert_eq!(ip_add_with(0, 64, 0x0a000000).as_deref(), Some("10.0.0.0"));
    }

    #[test]
    fn ip_parse_cases() {
        assert_eq!(ip_parse("10.0.0.1"), Some(167772161));
        assert_eq!(ip_parse("1"), Some(1));
        assert_eq!(ip_parse("1.2"), Some(16777218));
        assert_eq!(ip_parse("1.2.3"), Some(16908291));
        assert_eq!(ip_parse(" 10.0.0.1 "), Some(167772161));
        assert_eq!(ip_parse("1_0.2.3.4"), Some(167904004));
        assert_eq!(ip_parse("1.2.3.-4"), Some(-4));
        assert_eq!(ip_parse("1.2.3.4.5"), None); // TypeError in Python
        assert_eq!(ip_parse(""), None); // ValueError
        assert_eq!(ip_parse("a.b"), None);
        assert_eq!(ip_parse("1.2.3.x"), None);
    }

    #[test]
    fn net_parse_cases() {
        assert_eq!(net_parse("10.0.0.0/8"), Some((167772160, 8)));
        assert_eq!(net_parse("10.0.0.0"), Some((167772160, 24)));
        assert_eq!(net_parse("1.2.3.4/-1"), Some((16909060, -1)));
        assert_eq!(net_parse("1.2.3.4/24/5"), None);
        assert_eq!(net_parse("1.2.3.4/"), None);
        assert_eq!(net_parse("abc/8"), None);
        assert_eq!(net_parse("1.2.3.4/0x8"), None);
    }
}
