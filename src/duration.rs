//! Human-readable duration parsing/formatting for the seconds-based form
//! fields (check/project overrides, settings scan/nag intervals). Storage and
//! the scheduler are unaffected — everything is still persisted as plain
//! seconds; this module only sits at the form boundary.

/// Parse a duration into whole seconds. Accepts a bare integer (raw seconds,
/// for back-compat with what the forms used to take) or one or more
/// unit-suffixed components: `s`, `m`, `h`, `d` — combinable and
/// whitespace/case tolerant (`1h30m`, `1H 30M`). Returns `None` for anything
/// that is not fully consumed by that grammar.
pub fn parse_duration(s: &str) -> Option<i64> {
    let cleaned: String = s.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    let cleaned = cleaned.to_ascii_lowercase();
    if cleaned.is_empty() {
        return None;
    }

    let (neg, rest) = match cleaned.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, cleaned.as_str()),
    };
    if rest.is_empty() {
        return None;
    }

    // Back-compat: a bare integer is raw seconds.
    if rest.bytes().all(|b| b.is_ascii_digit()) {
        let v: i64 = rest.parse().ok()?;
        return if neg { v.checked_neg() } else { Some(v) };
    }

    let mut total: i64 = 0;
    let mut chars = rest.chars().peekable();
    while chars.peek().is_some() {
        let mut digits = String::new();
        while let Some(c) = chars.peek() {
            if c.is_ascii_digit() {
                digits.push(*c);
                chars.next();
            } else {
                break;
            }
        }
        if digits.is_empty() {
            return None;
        }
        let unit = chars.next()?;
        let mult: i64 = match unit {
            's' => 1,
            'm' => 60,
            'h' => 3600,
            'd' => 86400,
            _ => return None,
        };
        let n: i64 = digits.parse().ok()?;
        let component = n.checked_mul(mult)?;
        total = total.checked_add(component)?;
    }
    if neg {
        total.checked_neg()
    } else {
        Some(total)
    }
}

/// Render whole seconds back into the compact canonical form
/// `parse_duration` accepts, losslessly (`5430` -> `"1h30m30s"`, `300` -> `"5m"`,
/// `0` -> `"0s"`). Distinct from `view::fmt_secs`, which is a lossy *display*
/// format (`"1h 30m"` drops the seconds) and must stay unchanged.
pub fn fmt_duration(secs: i64) -> String {
    use std::fmt::Write;
    let s = secs.max(0);
    if s == 0 {
        return "0s".to_string();
    }
    let days = s / 86400;
    let hours = (s % 86400) / 3600;
    let mins = (s % 3600) / 60;
    let rem_secs = s % 60;
    let mut out = String::new();
    if days > 0 {
        let _ = write!(out, "{days}d");
    }
    if hours > 0 {
        let _ = write!(out, "{hours}h");
    }
    if mins > 0 {
        let _ = write!(out, "{mins}m");
    }
    if rem_secs > 0 {
        let _ = write!(out, "{rem_secs}s");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bare_seconds() {
        assert_eq!(parse_duration("0"), Some(0));
        assert_eq!(parse_duration("30"), Some(30));
        assert_eq!(parse_duration("3600"), Some(3600));
    }

    #[test]
    fn parses_each_unit() {
        assert_eq!(parse_duration("45s"), Some(45));
        assert_eq!(parse_duration("5m"), Some(300));
        assert_eq!(parse_duration("2h"), Some(7200));
        assert_eq!(parse_duration("3d"), Some(259_200));
    }

    #[test]
    fn parses_combos() {
        assert_eq!(parse_duration("1h30m"), Some(5400));
        assert_eq!(parse_duration("1d2h3m4s"), Some(93784));
        assert_eq!(parse_duration("1h1h"), Some(7200));
    }

    #[test]
    fn tolerates_whitespace_and_uppercase() {
        assert_eq!(parse_duration("1H 30M"), Some(5400));
        assert_eq!(parse_duration("  1h30m  "), Some(5400));
    }

    #[test]
    fn rejects_blank() {
        assert_eq!(parse_duration(""), None);
        assert_eq!(parse_duration("   "), None);
    }

    #[test]
    fn rejects_invalid() {
        assert_eq!(parse_duration("1x"), None);
        assert_eq!(parse_duration("1h30"), None);
        assert_eq!(parse_duration("abc"), None);
        assert_eq!(parse_duration("m5"), None);
        assert_eq!(parse_duration("-"), None);
        assert_eq!(parse_duration("h"), None);
        assert_eq!(parse_duration("1hh"), None);
    }

    #[test]
    fn parses_negative() {
        assert_eq!(parse_duration("-1h"), Some(-3600));
    }

    #[test]
    fn rejects_overflow() {
        assert_eq!(parse_duration("9223372036854775807d"), None);
    }

    #[test]
    fn formats_durations() {
        assert_eq!(fmt_duration(0), "0s");
        assert_eq!(fmt_duration(86400), "1d");
        assert_eq!(fmt_duration(5430), "1h30m30s");
        assert_eq!(fmt_duration(300), "5m");
        assert_eq!(fmt_duration(90), "1m30s");
        assert_eq!(fmt_duration(45), "45s");
    }

    #[test]
    fn round_trips() {
        for v in [0, 1, 45, 60, 90, 300, 3600, 5430, 86400, 90061] {
            assert_eq!(parse_duration(&fmt_duration(v)), Some(v));
        }
    }
}
