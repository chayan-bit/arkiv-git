//! Conversions between protocol values and Rhai values: CBOR payloads, op/push/repo maps,
//! JSON input for the CLI dry run.

use minicbor::Decoder;
use minicbor::data::Type;
use rhai::{Array, Dynamic, Map};
use serde::Deserialize;

use crate::{ProgramOp, RefUpdate, RepoInfo};

const MAX_DEPTH: u32 = 32;

fn ids(v: &[[u8; 32]]) -> Array {
    v.iter().map(|x| hex::encode(x).into()).collect()
}

fn int(n: u64) -> Result<Dynamic, String> {
    i64::try_from(n)
        .map(Dynamic::from)
        .map_err(|_| format!("{n} exceeds i64"))
}

pub(crate) fn repo_map(r: &RepoInfo) -> Result<Dynamic, String> {
    let mut m = Map::new();
    m.insert("rid".into(), hex::encode(r.rid).into());
    m.insert("delegates".into(), ids(&r.delegates).into());
    m.insert("quorum".into(), int(r.quorum)?);
    m.insert("name".into(), r.name.clone().into());
    Ok(m.into())
}

pub(crate) fn op_map(op: &ProgramOp) -> Result<Dynamic, String> {
    let mut m = Map::new();
    m.insert("id".into(), hex::encode(op.id).into());
    m.insert("author".into(), hex::encode(op.author).into());
    m.insert("type".into(), op.type_name.clone().into());
    m.insert("action".into(), op.action.clone().into());
    m.insert("object".into(), hex::encode(op.object).into());
    m.insert("parents".into(), ids(&op.parents).into());
    m.insert("payload".into(), cbor_to_dynamic(&op.payload)?);
    m.insert("order".into(), int(op.arkiv_order)?);
    m.insert("ts".into(), int(op.ts_ms)?);
    let proofs = op
        .proofs
        .iter()
        .map(proof_map)
        .collect::<Result<Array, _>>()?;
    m.insert("proofs".into(), proofs.into());
    Ok(m.into())
}

fn proof_map(p: &crate::ProofCheck) -> Result<Dynamic, String> {
    let mut m = Map::new();
    m.insert("kind".into(), p.kind.clone().into());
    m.insert("image_id".into(), p.image_id.clone().into());
    m.insert("ok".into(), p.ok.into());
    m.insert(
        "public".into(),
        cbor_to_dynamic(&cbor_from_json(&p.public)?)?,
    );
    Ok(m.into())
}

pub(crate) fn push_map(p: &RefUpdate, ff: bool) -> Result<Dynamic, String> {
    let mut m = Map::new();
    m.insert("author".into(), hex::encode(p.author).into());
    m.insert("ref".into(), p.name.clone().into());
    m.insert("old".into(), hex::encode(&p.old).into());
    // `new` is a reserved word in Rhai, so scripts see the new oid as `head`.
    m.insert("head".into(), hex::encode(&p.new).into());
    m.insert("order".into(), int(p.arkiv_order)?);
    m.insert("ff".into(), ff.into());
    Ok(m.into())
}

/// CBOR to Rhai: ints, strings, bools, null as `()`, byte strings as blobs, arrays, and
/// maps with string keys. Floats, tags and indefinite lengths are rejected.
pub(crate) fn cbor_to_dynamic(bytes: &[u8]) -> Result<Dynamic, String> {
    let mut d = Decoder::new(bytes);
    let v = item(&mut d, 0).map_err(|e| format!("payload: {e}"))?;
    match d.position() == bytes.len() {
        true => Ok(v),
        false => Err("payload: trailing bytes".into()),
    }
}

fn item(d: &mut Decoder<'_>, depth: u32) -> Result<Dynamic, String> {
    if depth > MAX_DEPTH {
        return Err("nested too deep".into());
    }
    let e = |e: minicbor::decode::Error| e.to_string();
    let len = |n: Option<u64>| n.ok_or_else(|| "indefinite length".to_string());
    Ok(match d.datatype().map_err(e)? {
        Type::U8
        | Type::U16
        | Type::U32
        | Type::U64
        | Type::I8
        | Type::I16
        | Type::I32
        | Type::I64
        | Type::Int => {
            let n = d.int().map_err(e)?;
            Dynamic::from(i64::try_from(n).map_err(|_| "integer exceeds i64".to_string())?)
        }
        Type::Bool => d.bool().map_err(e)?.into(),
        Type::Null => {
            d.null().map_err(e)?;
            Dynamic::UNIT
        }
        Type::String => d.str().map_err(e)?.into(),
        Type::Bytes => Dynamic::from_blob(d.bytes().map_err(e)?.to_vec()),
        Type::Array => {
            let n = len(d.array().map_err(e)?)?;
            let v: Result<Array, _> = (0..n).map(|_| item(d, depth + 1)).collect();
            v?.into()
        }
        Type::Map => {
            let n = len(d.map().map_err(e)?)?;
            let mut m = Map::new();
            for _ in 0..n {
                let k = d
                    .str()
                    .map_err(|_| "map keys must be strings".to_string())?;
                m.insert(k.into(), item(d, depth + 1)?);
            }
            m.into()
        }
        t => return Err(format!("unsupported CBOR item {t}")),
    })
}

/// JSON to canonical CBOR (map keys sorted by length, then bytes). Floats are rejected.
pub fn cbor_from_json(v: &serde_json::Value) -> Result<Vec<u8>, String> {
    let mut e = minicbor::Encoder::new(Vec::new());
    json_item(&mut e, v).map_err(|x| x.to_string())?;
    Ok(e.into_writer())
}

type Enc = minicbor::Encoder<Vec<u8>>;
type EncErr = minicbor::encode::Error<std::convert::Infallible>;

fn json_item(e: &mut Enc, v: &serde_json::Value) -> Result<(), EncErr> {
    use serde_json::Value as J;
    match v {
        J::Null => e.null().map(drop),
        J::Bool(b) => e.bool(*b).map(drop),
        J::Number(n) => match (n.as_u64(), n.as_i64()) {
            (Some(u), _) => e.u64(u).map(drop),
            (None, Some(i)) => e.i64(i).map(drop),
            _ => Err(EncErr::message("floats are not allowed")),
        },
        J::String(s) => e.str(s).map(drop),
        J::Array(a) => {
            e.array(a.len() as u64)?;
            a.iter().try_for_each(|x| json_item(e, x))
        }
        J::Object(o) => {
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort_by_key(|k| (k.len(), k.as_bytes()));
            e.map(keys.len() as u64)?;
            keys.into_iter().try_for_each(|k| {
                e.str(k)?;
                json_item(e, &o[k])
            })
        }
    }
}

pub(crate) fn json_payload<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    cbor_from_json(&v).map_err(serde::de::Error::custom)
}

/// Serde for `Vec<[u8; 32]>` as a list of hex strings.
pub(crate) mod hexes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &[[u8; 32]], s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(v.iter().map(hex::encode))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<[u8; 32]>, D::Error> {
        Vec::<String>::deserialize(d)?
            .iter()
            .map(|s| {
                let mut out = [0u8; 32];
                hex::decode_to_slice(s, &mut out).map_err(serde::de::Error::custom)?;
                Ok(out)
            })
            .collect()
    }
}
