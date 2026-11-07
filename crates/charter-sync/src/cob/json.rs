//! JSON views of op payloads. JSON to CBOR rejects floats so payloads stay deterministic and
//! sorts map keys by encoded bytes (canonical CBOR); CBOR to JSON renders byte strings as
//! `{"$bytes": hex}`, non-text map keys as their JSON text, and integers outside the i64
//! and u64 ranges as decimal strings.

use charter_core::ErrorCode;
use minicbor::data::Type;
use minicbor::{Decoder, Encoder};
use serde_json::{Map, Number, Value};

use crate::{Result, SyncError};

fn bad(msg: impl Into<String>) -> SyncError {
    SyncError::coded(ErrorCode::BadFormat, msg)
}

/// Canonical CBOR of `v`; floats are `E_BAD_FORMAT`.
pub fn to_cbor(v: &Value) -> Result<Vec<u8>> {
    let mut e = Encoder::new(Vec::new());
    match v {
        Value::Null => e.null(),
        Value::Bool(b) => e.bool(*b),
        Value::Number(n) => match (n.as_u64(), n.as_i64()) {
            (Some(u), _) => e.u64(u),
            (None, Some(i)) => e.i64(i),
            _ => return Err(bad(format!("payload number {n} is a float"))),
        },
        Value::String(s) => e.str(s),
        Value::Array(items) => {
            e.array(items.len() as u64).map_err(|_| bad("encode"))?;
            for item in items {
                e.writer_mut().extend(to_cbor(item)?);
            }
            Ok(&mut e)
        }
        Value::Object(map) => {
            let mut entries = map
                .iter()
                .map(|(k, v)| Ok((to_cbor(&Value::String(k.clone()))?, to_cbor(v)?)))
                .collect::<Result<Vec<_>>>()?;
            entries.sort();
            e.map(entries.len() as u64).map_err(|_| bad("encode"))?;
            entries
                .into_iter()
                .for_each(|(k, v)| e.writer_mut().extend(k.into_iter().chain(v)));
            Ok(&mut e)
        }
    }
    .map_err(|_| bad("encode"))?;
    Ok(e.into_writer())
}

/// JSON view of one CBOR value (payloads of verified ops always convert).
pub fn to_json(cbor: &[u8]) -> Result<Value> {
    let mut d = Decoder::new(cbor);
    let v = value(&mut d).ok_or_else(|| bad("payload is not a supported CBOR value"))?;
    match d.position() == cbor.len() {
        true => Ok(v),
        false => Err(bad("trailing bytes after payload")),
    }
}

fn int(i: i128) -> Value {
    match (u64::try_from(i), i64::try_from(i)) {
        (Ok(u), _) => Value::Number(Number::from(u)),
        (_, Ok(s)) => Value::Number(Number::from(s)),
        _ => Value::String(i.to_string()),
    }
}

fn value(d: &mut Decoder<'_>) -> Option<Value> {
    Some(match d.datatype().ok()? {
        Type::U8 | Type::U16 | Type::U32 | Type::U64 => int(d.int().ok()?.into()),
        Type::I8 | Type::I16 | Type::I32 | Type::I64 | Type::Int => int(d.int().ok()?.into()),
        Type::Bytes => {
            let b = hex::encode(d.bytes().ok()?);
            Value::Object(Map::from_iter([("$bytes".to_owned(), Value::String(b))]))
        }
        Type::String => Value::String(d.str().ok()?.to_owned()),
        Type::Bool => Value::Bool(d.bool().ok()?),
        Type::Null => {
            d.null().ok()?;
            Value::Null
        }
        Type::Array => {
            let n = d.array().ok()??;
            Value::Array((0..n).map(|_| value(d)).collect::<Option<_>>()?)
        }
        Type::Map => {
            let n = d.map().ok()??;
            let mut m = Map::new();
            for _ in 0..n {
                let k = match value(d)? {
                    Value::String(s) => s,
                    other => other.to_string(),
                };
                m.insert(k, value(d)?);
            }
            Value::Object(m)
        }
        _ => return None,
    })
}
