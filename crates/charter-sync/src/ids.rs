//! Text forms of rid / nid (design 2.1: `z` + base58btc, or 64 lowercase hex) and remote
//! URLs (`arkiv://<rid>`, `arkiv://<rid>/<nid>`, or the same without the scheme as git
//! passes it for `arkiv::<rid>`).

use charter_core::{ErrorCode, Hash32, Nid, Rid};

use crate::{Result, SyncError};

/// Radicle-style text form: `z` + base58btc.
pub fn to_text(h: &Hash32) -> String {
    format!("z{}", bs58::encode(h).into_string())
}

pub fn hex32(h: &Hash32) -> String {
    hex::encode(h)
}

/// Parses 64 hex digits or `z` + base58btc of 32 bytes.
pub fn parse_id(s: &str) -> Result<Hash32> {
    let bad = || {
        SyncError::coded(
            ErrorCode::BadFormat,
            format!("not a repo or peer id: {s:?}"),
        )
    };
    let bytes = if s.len() == 64 {
        hex::decode(s).map_err(|_| bad())?
    } else {
        let b58 = s.strip_prefix('z').ok_or_else(bad)?;
        bs58::decode(b58).into_vec().map_err(|_| bad())?
    };
    bytes.try_into().map_err(|_| bad())
}

pub fn unhex32(s: &str) -> Result<Hash32> {
    let v = hex::decode(s)
        .map_err(|_| SyncError::coded(ErrorCode::BadFormat, format!("bad hex {s:?}")))?;
    v.try_into()
        .map_err(|_| SyncError::coded(ErrorCode::BadFormat, format!("not 32 bytes: {s:?}")))
}

/// A parsed remote URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteUrl {
    pub rid: Rid,
    /// One peer's refs instead of canonical refs.
    pub nid: Option<Nid>,
}

pub fn parse_url(url: &str) -> Result<RemoteUrl> {
    let rest = url.strip_prefix("arkiv://").unwrap_or(url);
    let rest = rest.trim_end_matches('/');
    let (rid, nid) = match rest.split_once('/') {
        Some((r, n)) => (r, Some(parse_id(n)?)),
        None => (rest, None),
    };
    Ok(RemoteUrl {
        rid: parse_id(rid)?,
        nid,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_forms_round_trip() {
        let h = [7u8; 32];
        let z = to_text(&h);
        assert!(z.starts_with('z'));
        assert_eq!(parse_id(&z).expect("z"), h);
        assert_eq!(parse_id(&hex32(&h)).expect("hex"), h);
        assert_eq!(unhex32(&hex32(&h)).expect("hex"), h);
        for bad in ["", "z", "zzzz0OIl", "abc", &"g".repeat(64), "z1111"] {
            assert_eq!(
                parse_id(bad).map_err(|e| e.code()),
                Err(ErrorCode::BadFormat)
            );
        }
        assert!(unhex32("00").is_err() && unhex32("zz").is_err());
    }

    #[test]
    fn urls_with_and_without_scheme_and_peer() {
        let (r, n) = ([1u8; 32], [2u8; 32]);
        let plain = parse_url(&to_text(&r)).expect("plain");
        assert_eq!(plain, RemoteUrl { rid: r, nid: None });
        let full = format!("arkiv://{}/{}/", to_text(&r), hex32(&n));
        assert_eq!(parse_url(&full).expect("full").nid, Some(n));
        assert!(parse_url("arkiv://nope").is_err());
    }
}
