//! Writing to Arkiv: signed bodies and chunks become create ops (flags 3, design 2.1), sent
//! through the paced pipeline (design 4.6); per-repo addresses are topped up from the wallet
//! when short (design 4.2 step 6).

use alloy_signer_local::PrivateKeySigner;
use arkiv_git_core::{
    AttrValue, Attribute, Body, Envelope, ErrorCode, Hash32, Rid, chunk_attributes, content_hash,
    entity_attributes,
};
use arkiv_git_crypto::SecretKey;
use arkiv_git_store::pipeline::Landed;
use arkiv_git_store::{
    Address, Arkiv, AttributeValue, Operation, PipelineConfig, StoreError, TxPipeline, U256,
    create_op, random_salt,
};

use crate::{Result, SyncError};

/// Gas of a plain transfer.
const TRANSFER_GAS: u64 = 21_000;

/// A signed entity ready to send: its envelope bytes and content hash.
#[derive(Debug, Clone)]
pub struct Signed {
    pub bytes: Vec<u8>,
    pub h: Hash32,
}

pub fn sign<B: Body>(sk: &SecretKey, body: &B) -> Result<Signed> {
    let env = Envelope::sign(sk, B::KIND, body.encode())?;
    let bytes = env.encode();
    Ok(Signed {
        h: content_hash(B::KIND, &bytes),
        bytes,
    })
}

fn wire(attrs: &[Attribute]) -> Vec<(&str, AttributeValue, bool)> {
    attrs
        .iter()
        .map(|a| {
            let v = match &a.value {
                AttrValue::Str(s) => AttributeValue::Str(s.clone()),
                AttrValue::U64(n) => AttributeValue::U64(*n),
                AttrValue::Bytes(b) => AttributeValue::Bytes(b.clone()),
                AttrValue::Bytes32(h) => AttributeValue::Bytes32(*h),
            };
            (a.name.as_str(), v, a.indexed())
        })
        .collect()
}

/// The create op for already-signed envelope bytes of kind `B`.
pub fn entity_op<B: Body>(bytes: &[u8], lifetime: u64) -> Result<Operation> {
    let env = Envelope::decode(bytes)?;
    let body = B::decode(&env.body)?;
    let attrs = entity_attributes(&env, bytes, &body);
    Ok(create_op(&wire(&attrs), lifetime, random_salt())?)
}

pub fn chunk_op(
    rid: &Rid,
    pack_id: &Hash32,
    index: u64,
    bytes: &[u8],
    lifetime: u64,
) -> Result<Operation> {
    let attrs = chunk_attributes(rid, pack_id, index, bytes);
    Ok(create_op(&wire(&attrs), lifetime, random_salt())?)
}

/// Sends every job (one tx each) from `signer` and waits for all receipts.
pub async fn send<A: Arkiv>(
    a: &A,
    signer: &PrivateKeySigner,
    cfg: &PipelineConfig,
    jobs: Vec<Vec<Operation>>,
) -> Result<Vec<Landed>> {
    if jobs.is_empty() {
        return Ok(vec![]);
    }
    let chain_id = a.chain_id().await?;
    let mut p = TxPipeline::new(a, signer.clone(), chain_id, cfg.clone());
    Ok(p.run(jobs).await?)
}

/// Gas `execute(ops)` would use from `from`.
pub async fn estimate<A: Arkiv>(a: &A, from: Address, ops: &[Operation]) -> Result<u64> {
    Ok(a.estimate(from, ops).await?)
}

/// What `gas` costs at the pipeline's fee cap: `gas x 1.1 x 2 x base fee`.
pub fn price(gas: u64, base_fee: u128) -> U256 {
    U256::from(gas) * U256::from(11) / U256::from(10) * U256::from(2 * base_fee)
}

/// Ensures `target` holds `need` wei plus 25 percent headroom (estimates of the real txs
/// drift a little from the probes), topping it up to twice `need` from `wallet`.
pub async fn ensure_funds<A: Arkiv>(
    a: &A,
    wallet: &PrivateKeySigner,
    target: Address,
    need: U256,
    cfg: &PipelineConfig,
) -> Result<()> {
    let have = a.balance(target).await?;
    if have >= need + need / U256::from(4) {
        return Ok(());
    }
    let missing = need * U256::from(2) - have.min(need);
    let base_fee = a.base_fee().await?;
    let wallet_need = missing + price(TRANSFER_GAS, base_fee);
    let wallet_have = a.balance(wallet.address()).await?;
    if wallet_have < wallet_need {
        return Err(SyncError::coded(
            ErrorCode::InsufficientFunds,
            format!(
                "address {target} needs {need} wei, has {have}; wallet {} has {wallet_have}, \
                 needs {wallet_need} to top it up (run `arkiv-git fund`)",
                wallet.address()
            ),
        ));
    }
    let chain_id = a.chain_id().await?;
    let mut p = TxPipeline::new(a, wallet.clone(), chain_id, cfg.clone());
    let r = p.transfer(target, missing).await?;
    if !r.success {
        return Err(StoreError::Reverted {
            tx: Some(r.tx_hash),
            reason: "top-up transfer failed".into(),
        }
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkiv_git_core::{PeerBody, open};
    use arkiv_git_crypto::MasterSeed;

    #[test]
    fn signed_entity_ops_carry_the_schema_attributes() {
        let sk = MasterSeed::from_bytes([4; 32]).signing_key();
        let pk = sk.public_key();
        let body = PeerBody {
            alg: pk.alg().id(),
            pk: pk.as_bytes().to_vec(),
            alias: "a".into(),
        };
        let s = sign(&sk, &body).expect("sign");
        let op = entity_op::<PeerBody>(&s.bytes, 10).expect("op");
        assert!(!op.operationData.is_empty());
        let env = Envelope::decode(&s.bytes).expect("env");
        let attrs = entity_attributes(&env, &s.bytes, &body);
        assert_eq!(wire(&attrs).len(), attrs.len());
        assert!(wire(&attrs).iter().any(|(n, _, idx)| *n == "ag.v" && !idx));
        let obs = arkiv_git_core::Observed {
            key: [0; 32],
            creator: [0; 20],
            block: 1,
            tx: 0,
            expires_at: 2,
            attributes: attrs,
        };
        assert_eq!(open::<PeerBody>(&obs).expect("opens").h, s.h);
        assert!(entity_op::<PeerBody>(b"junk", 1).is_err());
        assert!(chunk_op(&[1; 32], &[2; 32], 0, &[1, 2, 3], 5).is_ok());
        assert_eq!(price(10, 1), U256::from(22));
    }
}
