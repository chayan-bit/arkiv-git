//! `--until` values: an RFC 3339 instant or `+<n>s|m|h|d` relative to now.

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::{Result, TimelockError};

/// Unix seconds for `s`; `now` anchors relative values.
pub fn parse_until(s: &str, now: u64) -> Result<u64> {
    let bad = || {
        TimelockError::Usage(format!(
            "not a time: {s:?} (use RFC 3339 or +90s, +30m, +12h, +7d)"
        ))
    };
    if let Some(rel) = s.strip_prefix('+') {
        let (num, mul) = match rel.as_bytes().last() {
            Some(b's') => (&rel[..rel.len() - 1], 1),
            Some(b'm') => (&rel[..rel.len() - 1], 60),
            Some(b'h') => (&rel[..rel.len() - 1], 3600),
            Some(b'd') => (&rel[..rel.len() - 1], 86_400),
            _ => return Err(bad()),
        };
        let n: u64 = num.parse().map_err(|_| bad())?;
        return n
            .checked_mul(mul)
            .and_then(|d| now.checked_add(d))
            .ok_or_else(bad);
    }
    let t = OffsetDateTime::parse(s, &Rfc3339).map_err(|_| bad())?;
    u64::try_from(t.unix_timestamp()).map_err(|_| bad())
}

/// RFC 3339 (UTC) rendering of unix seconds, for messages.
pub fn fmt_unix(t: u64) -> String {
    i64::try_from(t)
        .ok()
        .and_then(|t| OffsetDateTime::from_unix_timestamp(t).ok())
        .and_then(|d| d.format(&Rfc3339).ok())
        .unwrap_or_else(|| t.to_string())
}
