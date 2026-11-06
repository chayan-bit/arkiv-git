//! Canonical CBOR (RFC 8949 section 4.2.1) decoding by re-encode comparison.
//!
//! `minicbor` derive emits definite lengths, shortest integer forms and map keys in
//! declaration order (declared ascending), so re-encoding a decoded value reproduces the
//! canonical bytes. Any accepted-but-non-canonical input (non-shortest integers, indefinite
//! lengths, unsorted, duplicate or unknown keys, tags) re-encodes differently and is rejected.
//! `minicbor` 2.3 offers no strict decoding mode, so the check is ours (design section 2.1).

use crate::FormatError;
use crate::types::Hash32;
use minicbor::decode::{self, Decoder};
use minicbor::encode::{self, Encoder, Write};
use minicbor::{Decode, Encode};

pub(crate) fn to_vec<T: Encode<()>>(v: &T) -> Vec<u8> {
    minicbor::to_vec(v).unwrap_or_else(|_| unreachable!("encoding into a Vec cannot fail"))
}

pub(crate) fn decode_canonical<T>(bytes: &[u8]) -> Result<T, FormatError>
where
    T: Encode<()> + for<'b> Decode<'b, ()>,
{
    let mut d = Decoder::new(bytes);
    let v: T = d.decode().map_err(|e| FormatError::Cbor(e.to_string()))?;
    if d.position() != bytes.len() {
        return Err(FormatError::TrailingBytes);
    }
    if to_vec(&v) != bytes {
        return Err(FormatError::NonCanonical);
    }
    Ok(v)
}

/// `[* bstr .size 32]` as a CBOR array of 32-byte strings.
pub(crate) mod hashes {
    use super::*;

    pub fn encode<C, W: Write>(
        v: &[Hash32],
        e: &mut Encoder<W>,
        _: &mut C,
    ) -> Result<(), encode::Error<W::Error>> {
        e.array(v.len() as u64)?;
        v.iter().try_for_each(|h| e.bytes(h).map(|_| ()))
    }

    pub fn decode<C>(d: &mut Decoder<'_>, _: &mut C) -> Result<Vec<Hash32>, decode::Error> {
        let n = d
            .array()?
            .ok_or_else(|| decode::Error::message("indefinite array"))?;
        (0..n)
            .map(|_| {
                d.bytes()?
                    .try_into()
                    .map_err(|_| decode::Error::message("expected 32-byte string"))
            })
            .collect()
    }
}
