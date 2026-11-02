//! Host calendar conversion for the predefined date and time macros.
//!
//! `__DATE__`, `__TIME__` and `__TIMESTAMP__` are the only predefined macros
//! whose value depends on something other than the translation unit. Their
//! contracts are easy to state and easy to get subtly wrong:
//!
//! * `__DATE__` and `__TIME__` are the **host's local** wall clock, except that
//!   a set `SOURCE_DATE_EPOCH` replaces both with its **UTC** interpretation
//!   (the reproducible-builds convention, measured on GCC 16.2 and Clang
//!   23.1.4).
//! * `__TIMESTAMP__` is the modification time of the *current source file*, in
//!   the 24-character form `ctime` produces — also local time, falling back to
//!   `"??? ??? ?? ??:??:?? ????"` when the file has no readable mtime.
//! * Every one of those three is a *string literal*, so the day-of-month field
//!   is space padded (`"Nov  2 2026"`), not zero padded.
//!
//! `src/frontend/preprocessor` used to synthesise `__DATE__`/`__TIME__` from a
//! UTC-only civil-date conversion, which is right in a UTC container and wrong
//! everywhere else: with `TZ=America/New_York` GCC and Clang both report the
//! local date, so a translation unit diffed against them differs by a day for
//! part of every day. The local calendar therefore comes from the host's own
//! zone database through `localtime_r`, exactly as GCC's `localtime` call and
//! Clang's `localtime_r` call do; the UTC path stays pure Rust (Howard Hinnant's
//! civil-date algorithm) because it must be usable for `SOURCE_DATE_EPOCH` on
//! every host, including ones without a C library binding.
//!
//! The FFI surface is deliberately tiny and defensive: `localtime_r` is
//! thread-safe by contract, `tzset` is called exactly once behind a `Once`, a
//! null return (out-of-range time for the host's zone) falls back to UTC, and
//! the `struct tm` is declared with the layout glibc, musl and macOS all use.
//! Failure to link at all is impossible — these are libc symbols the Rust
//! runtime already depends on.
//!
//! It is compiled for 64-bit Unix hosts only. A 32-bit host would need `time_t`
//! declared as `c_long` for glibc and as `long long` for musl — which differ,
//! and neither can be detected from `cfg` without also pinning the libc — and a
//! `struct tm` whose trailing `tm_gmtoff`/`tm_zone` pair differs between them
//! too; guessing wrong there is undefined behaviour of the worst kind, writing
//! past the object. Those hosts take the documented UTC fallback instead. Every
//! host this compiler is built and tested on is 64-bit, and the gate that checks
//! this module against the zone database runs on them.

use std::path::Path;
use std::sync::OnceLock;

/// A civil (calendar) timestamp, already resolved into the zone it displays in.
///
/// Fields are post-conversion values, never offsets: `month` is 1-12, `day` is
/// 1-31, `wday` is 0 = Sunday. Keeping the resolved form (instead of a
/// timestamp plus an offset) means the formatters cannot disagree about which
/// zone they are rendering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Civil {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    /// Day of week, 0 = Sunday. Only `__TIMESTAMP__` needs it.
    pub wday: u32,
}

const MONTH_NAMES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const WEEKDAY_NAMES: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// The value GCC and Clang emit when a file's modification time cannot be read.
pub const UNKNOWN_TIMESTAMP: &str = "??? ??? ?? ??:??:?? ????";

impl Civil {
    /// Convert a Unix timestamp into UTC calendar fields (no timezone database).
    ///
    /// Howard Hinnant's `civil_from_days`: exact for the whole `i64` range of
    /// representable days, no branches on era boundaries beyond the documented
    /// one, and no dependency on the host's locale or zone.
    pub fn utc(secs: i64) -> Civil {
        let days = secs.div_euclid(86_400);
        let sod = secs.rem_euclid(86_400);
        let (year, month, day) = Self::civil_from_days(days);
        Civil {
            year,
            month,
            day,
            hour: (sod / 3_600) as u32,
            minute: ((sod % 3_600) / 60) as u32,
            second: (sod % 60) as u32,
            wday: (days + 4).rem_euclid(7) as u32, // 1970-01-01 was a Thursday
        }
    }

    /// Convert a Unix timestamp into the **host's local** calendar fields.
    ///
    /// Uses the C library's `localtime_r`, so the result follows `TZ`, the
    /// system zone database, DST transitions and historic offsets — the same
    /// source GCC and Clang use. Falls back to [`Civil::utc`] when the host has
    /// no local-time support compiled in (`cfg` below) or when the zone
    /// database cannot represent the instant (a null return).
    pub fn local(secs: i64) -> Civil {
        match localtime_via_host(secs) {
            Some(civil) => civil,
            None => Civil::utc(secs),
        }
    }

    /// Split a day count into `(year, month, day)`; days are relative to
    /// 1970-01-01 and may be negative.
    fn civil_from_days(days: i64) -> (i64, u32, u32) {
        let z = days + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
        let doe = z - era * 146_097; // [0, 146096]
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096).div_euclid(365);
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
        let mp = (5 * doy + 2).div_euclid(153); // [0, 11]
        let d = doy - (153 * mp + 2).div_euclid(5) + 1; // [1, 31]
        let m = mp + if mp < 10 { 3 } else { -9 }; // [1, 12]
        (y + i64::from(m <= 2), m as u32, d as u32)
    }

    /// `__DATE__` form: `"Nov  2 2026"` (day space padded to two columns).
    pub fn date(&self) -> String {
        format!(
            "{} {:>2} {}",
            MONTH_NAMES[(self.month.saturating_sub(1) as usize).min(11)],
            self.day,
            self.year
        )
    }

    /// `__TIME__` form: `"16:25:19"`.
    pub fn time(&self) -> String {
        format!("{:02}:{:02}:{:02}", self.hour, self.minute, self.second)
    }

    /// `__TIMESTAMP__` / `ctime` form: `"Mon Nov  2 16:25:19 2026"`.
    ///
    /// Built field by field rather than by concatenating [`Self::date`] and
    /// [`Self::time`]: the ctime order interleaves them (weekday, month, day,
    /// time, year), so a concatenation would silently put the year before the
    /// time.
    pub fn ctime(&self) -> String {
        format!(
            "{} {} {:>2} {} {}",
            WEEKDAY_NAMES[(self.wday as usize).min(6)],
            MONTH_NAMES[(self.month.saturating_sub(1) as usize).min(11)],
            self.day,
            self.time(),
            self.year
        )
    }
}

/// The latest instant both reference compilers accept for `SOURCE_DATE_EPOCH`:
/// 9999-12-31T23:59:59Z, the last second whose `__DATE__` still has a four-digit
/// year (measured: `253402300799` is accepted by GCC 16.2 and Clang 23.1.4,
/// `253402300800` is rejected by both).
pub const MAX_SOURCE_DATE_EPOCH: i64 = 253_402_300_799;

/// What `SOURCE_DATE_EPOCH` says, decided once per process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceDateEpoch {
    /// Not set at all: sample the wall clock (and, for `__TIMESTAMP__`, the
    /// file's mtime).
    Unset,
    /// Set and valid: this instant, in UTC, replaces the sampled one.
    Valid(i64),
    /// Set but not a non-negative decimal integer below
    /// [`MAX_SOURCE_DATE_EPOCH`]. Both reference compilers reject this; the
    /// caller is expected to diagnose it and fall back to the sampled value, as
    /// GCC does.
    Invalid,
}

/// `SOURCE_DATE_EPOCH`, parsed once per process.
///
/// The rule is GCC's, because GCC's is the one the reproducible-builds
/// convention is implemented against: `strtoll` semantics (leading ASCII
/// whitespace and a `+` sign are accepted), the *whole* remaining string must be
/// digits, and the value must not exceed [`MAX_SOURCE_DATE_EPOCH`]. Clang is
/// stricter -- it rejects the leading whitespace and the sign as well -- but the
/// two only differ on inputs that are already malformed, and the permissive
/// reading is the one a build that worked under GCC keeps working under.
///
/// Unlike the first version of this function there is no `trim`: a value with
/// trailing garbage is *invalid*, not quietly accepted, so `__DATE__` can never
/// show an instant no reference compiler would produce (`-1` used to expand to
/// the 1969 date).
pub fn source_date_epoch() -> SourceDateEpoch {
    static VALUE: OnceLock<SourceDateEpoch> = OnceLock::new();
    *VALUE.get_or_init(|| match std::env::var("SOURCE_DATE_EPOCH") {
        Ok(raw) => parse_source_date_epoch(&raw),
        Err(_) => SourceDateEpoch::Unset,
    })
}

/// Parse one `SOURCE_DATE_EPOCH` value. Pure, so the acceptance rule is a unit
/// test rather than a comment.
fn parse_source_date_epoch(raw: &str) -> SourceDateEpoch {
    let bytes = raw.as_bytes();
    let mut index = 0;
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    let mut negative = false;
    if index < bytes.len() && (bytes[index] == b'+' || bytes[index] == b'-') {
        negative = bytes[index] == b'-';
        index += 1;
    }
    let digits_start = index;
    let mut value: i64 = 0;
    let mut overflow = false;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        let digit = i64::from(bytes[index] - b'0');
        value = match value.checked_mul(10).and_then(|v| v.checked_add(digit)) {
            Some(v) => v,
            None => {
                overflow = true;
                0
            }
        };
        index += 1;
    }
    // Empty, truncated (`12x`), negative, or beyond the four-year range: all
    // four are rejected by GCC, and an out-of-i64 value is rejected by both.
    if index != bytes.len() || index == digits_start || negative || overflow {
        return SourceDateEpoch::Invalid;
    }
    if value > MAX_SOURCE_DATE_EPOCH {
        return SourceDateEpoch::Invalid;
    }
    SourceDateEpoch::Valid(value)
}

/// The current wall clock as Unix seconds, floored the way `stat` reports mtimes
/// (a clock set before 1970 yields a negative value rather than 0).
pub fn now_secs() -> i64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(after) => after.as_secs() as i64,
        Err(before) => {
            -(before.duration().as_secs() as i64) - i64::from(before.duration().subsec_nanos() > 0)
        }
    }
}

/// Modification time of `path` as Unix seconds, or `None` when the file cannot
/// be examined (a pipe, a virtual buffer, a deleted file).
///
/// Sub-second precision is discarded the way `stat`'s `st_mtime` discards it:
/// `__TIMESTAMP__` has one-second granularity by definition.
pub fn file_mtime_secs(path: &Path) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    match modified.duration_since(std::time::UNIX_EPOCH) {
        Ok(after) => Some(after.as_secs() as i64),
        // Pre-epoch mtimes are legal (`touch -d`, a checkout with restored
        // timestamps), and rounding *up* to the epoch would misreport them by a
        // second.
        Err(before) => Some(
            -(before.duration().as_secs() as i64) - i64::from(before.duration().subsec_nanos() > 0),
        ),
    }
}

/// Append `s` as a C string literal (a **quoted** replacement list).
pub fn push_quoted(out: &mut String, s: &str) {
    out.push('"');
    out.push_str(s);
    out.push('"');
}

/// Append the quoted "no modification time" form.
pub fn push_unknown_timestamp(out: &mut String) {
    push_quoted(out, UNKNOWN_TIMESTAMP);
}

#[cfg(all(target_family = "unix", target_pointer_width = "64"))]
mod host {
    use super::Civil;
    use std::ffi::c_char;
    use std::ffi::c_int;
    use std::ffi::c_long;

    /// `struct tm` as glibc, musl and macOS lay it out.
    ///
    /// `tm_gmtoff`/`tm_zone` are at the end and are the only members whose
    /// presence varies by libc; declaring them (and never reading them) means
    /// the callee always has room to write everything it writes, on every one
    /// of those libcs.
    #[repr(C)]
    #[allow(dead_code)] // `tm_yday`, `tm_isdst` and the GNU tail exist for the
    // callee's benefit: truncating the struct would let `localtime_r` write
    // past its end. They are never read here.
    struct Tm {
        tm_sec: c_int,
        tm_min: c_int,
        tm_hour: c_int,
        tm_mday: c_int,
        tm_mon: c_int,
        tm_year: c_int,
        tm_wday: c_int,
        tm_yday: c_int,
        tm_isdst: c_int,
        tm_gmtoff: c_long,
        tm_zone: *const c_char,
    }

    impl Tm {
        fn zeroed() -> Tm {
            Tm {
                tm_sec: 0,
                tm_min: 0,
                tm_hour: 0,
                tm_mday: 0,
                tm_mon: 0,
                tm_year: 0,
                tm_wday: 0,
                tm_yday: 0,
                tm_isdst: 0,
                tm_gmtoff: 0,
                tm_zone: std::ptr::null(),
            }
        }
    }

    unsafe extern "C" {
        fn localtime_r(time: *const i64, out: *mut Tm) -> *mut Tm;
        fn tzset();
    }

    pub(super) fn localtime(secs: i64) -> Option<Civil> {
        /// `TZ` is read once per process: the zone cannot change under a running
        /// compile, and `tzset` is not thread-safe, so calling it on every
        /// expansion would be both wasteful and a data race the moment
        /// preprocessing is parallelised. `Once` makes the first caller do it and
        /// every later caller pay one relaxed load and a branch.
        static TZ_LOADED: std::sync::Once = std::sync::Once::new();

        let mut tm = Tm::zeroed();
        // SAFETY, for both calls: `call_once` runs `tzset` exactly once and
        // before any `localtime_r` in this process, which is the ordering the
        // local-time API requires and the reason the two calls share one block;
        // `secs` and `tm` are valid for the call, `tm` is a full `struct tm`
        // including the GNU tail fields so the callee's writes stay inside the
        // object, and the returned pointer is either null or `&mut tm`.
        let ok = unsafe {
            TZ_LOADED.call_once(|| tzset());
            localtime_r(&secs, &mut tm)
        };
        if ok.is_null() {
            return None;
        }
        // A year outside the four-digit range would make the fixed-width
        // `__TIMESTAMP__` form ambiguous and is what `localtime_r` itself
        // rejects on 32-bit `time_t` builds; treat it the same way.
        if !(0..=9999).contains(&(tm.tm_year as i64 + 1900)) {
            return None;
        }
        Some(Civil {
            year: tm.tm_year as i64 + 1900,
            month: (tm.tm_mon.clamp(0, 11) as u32) + 1,
            day: tm.tm_mday.clamp(1, 31) as u32,
            hour: tm.tm_hour.clamp(0, 23) as u32,
            minute: tm.tm_min.clamp(0, 59) as u32,
            second: tm.tm_sec.clamp(0, 60) as u32,
            wday: tm.tm_wday.clamp(0, 6) as u32,
        })
    }
}

#[cfg(all(target_family = "unix", target_pointer_width = "64"))]
fn localtime_via_host(secs: i64) -> Option<Civil> {
    host::localtime(secs)
}

#[cfg(not(all(target_family = "unix", target_pointer_width = "64")))]
fn localtime_via_host(_secs: i64) -> Option<Civil> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_matches_known_instants() {
        // 2001-02-03T04:05:06Z was a Saturday.
        let c = Civil::utc(981_173_106);
        assert_eq!((c.year, c.month, c.day), (2001, 2, 3));
        assert_eq!((c.hour, c.minute, c.second), (4, 5, 6));
        assert_eq!(c.wday, 6);
        assert_eq!(c.date(), "Feb  3 2001");
        assert_eq!(c.time(), "04:05:06");
        assert_eq!(c.ctime(), "Sat Feb  3 04:05:06 2001");

        // The epoch itself is a Thursday, and 0 must render as day 1.
        let e = Civil::utc(0);
        assert_eq!((e.year, e.month, e.day), (1970, 1, 1));
        assert_eq!(e.wday, 4);
        assert_eq!(e.date(), "Jan  1 1970");

        // Pre-epoch: 1969-12-31T23:59:59Z, and one second earlier must not
        // alias it (floor division, never truncation toward zero).
        let p = Civil::utc(-1);
        assert_eq!((p.year, p.month, p.day), (1969, 12, 31));
        assert_eq!((p.hour, p.minute, p.second), (23, 59, 59));
        assert_eq!(Civil::utc(-86_400).date(), "Dec 31 1969");
    }

    #[test]
    fn leap_days_and_century_rules() {
        // 2000-02-29 (divisible by 400: a leap year) and 1900-03-01 (not one).
        assert_eq!(Civil::utc(951_782_400).date(), "Feb 29 2000");
        assert_eq!(Civil::utc(-2_203_891_200).date(), "Mar  1 1900");
        // The 2038 boundary a 32-bit time_t cannot cross, computed in i64.
        let y2038 = Civil::utc(2_147_483_647);
        assert_eq!(y2038.date(), "Jan 19 2038");
        assert_eq!(y2038.time(), "03:14:07");
        assert_eq!(y2038.wday, 2);
    }

    #[test]
    fn mtime_distinguishes_pre_epoch_from_epoch() {
        // A pre-epoch mtime must not be reported as zero: `-1` and `0` are
        // different seconds and render as different years.
        assert_ne!(Civil::utc(-1).date(), Civil::utc(0).date());
    }

    #[test]
    fn missing_file_has_no_mtime() {
        assert_eq!(
            file_mtime_secs(Path::new("/nonexistent/lccc/no-such-file")),
            None
        );
        let mut quoted = String::new();
        push_unknown_timestamp(&mut quoted);
        assert_eq!(quoted, format!("\"{}\"", UNKNOWN_TIMESTAMP));
    }

    /// The acceptance rule for `SOURCE_DATE_EPOCH` is GCC's, and every case in
    /// this matrix was measured against GCC 16.2 and Clang 23.1.4 first: the
    /// values both accept are `Valid`, the values both reject are `Invalid`, and
    /// the two where they disagree (`+1700000000`, ` 1700000000`) follow GCC.
    #[test]
    fn source_date_epoch_acceptance_matches_the_reference_rule() {
        for accepted in [
            ("0", 0),
            ("1", 1),
            ("1700000000", 1_700_000_000),
            ("01700000000", 1_700_000_000),
            ("+1700000000", 1_700_000_000),
            (" 1700000000", 1_700_000_000),
            ("\t1700000000", 1_700_000_000),
            ("253402300799", MAX_SOURCE_DATE_EPOCH),
        ] {
            assert_eq!(
                parse_source_date_epoch(accepted.0),
                SourceDateEpoch::Valid(accepted.1),
                "{:?}",
                accepted.0
            );
        }
        for rejected in [
            "",
            " ",
            "abc",
            "1700000000x",
            "x1700000000",
            "1700000000 ",
            "-1",
            "+-1",
            "253402300800",
            "99999999999999999999",
            "9223372036854775807",
        ] {
            assert_eq!(
                parse_source_date_epoch(rejected),
                SourceDateEpoch::Invalid,
                "{rejected:?}"
            );
        }
        // An unset variable is not the same answer as a malformed one: only the
        // latter is diagnosable.
        assert_ne!(parse_source_date_epoch(""), SourceDateEpoch::Unset);
    }

    #[test]
    fn the_wall_clock_is_after_the_epoch_it_was_measured_in() {
        // 2026-01-01T00:00:00Z, a floor rather than a guess: the clock of the
        // machine running the tests is ahead of the year this file was written.
        assert!(now_secs() > 1_767_225_600);
    }
}
