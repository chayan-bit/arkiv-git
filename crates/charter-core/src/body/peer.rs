use super::{Body, cbor_codec, require, seq_prev_ok};
use crate::attrs::Attribute;
use crate::types::{Address, Hash32, Nid, Rid};
use crate::{FormatError, Kind, field};
use charter_crypto::{Alg, PublicKey};

/// `peer`: a peer's public key, self-signed as proof of possession.
#[derive(Debug, Clone, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(map)]
pub struct PeerBody {
    #[n(1)]
    pub alg: u16,
    #[cbor(n(2), with = "minicbor::bytes")]
    pub pk: Vec<u8>,
    /// Display only, never trusted.
    #[n(3)]
    pub alias: String,
}

impl PeerBody {
    pub fn public_key(&self) -> Result<PublicKey, FormatError> {
        let alg = Alg::from_id(self.alg).ok_or(field("peer", "alg", "unknown algorithm"))?;
        Ok(PublicKey::from_bytes(alg, &self.pk)?)
    }
}

impl Body for PeerBody {
    const KIND: Kind = Kind::Peer;
    cbor_codec!();

    fn validate(&self) -> Result<(), FormatError> {
        let alg = Alg::from_id(self.alg).ok_or(field("peer", "alg", "unknown algorithm"))?;
        require(
            self.pk.len() == alg.pk_len() && self.pk.len() <= 4096,
            field("peer", "pk", "length does not match algorithm"),
        )?;
        require(self.alias.len() <= 64, field("peer", "alias", "over 64 B"))
    }

    fn extra_attributes(&self, signer: &Nid, h: &Hash32) -> Vec<Attribute> {
        vec![
            Attribute::b32("ag.nid", *signer),
            Attribute::b32("ag.h", *h),
        ]
    }
}

/// `bind`: binds a peer to its per-repo control and bulk EVM addresses.
#[derive(Debug, Clone, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(map)]
pub struct BindBody {
    #[cbor(n(1), with = "minicbor::bytes")]
    pub rid: Rid,
    #[n(2)]
    pub seq: u64,
    #[cbor(n(3), with = "minicbor::bytes")]
    pub prev: Hash32,
    #[cbor(n(4), with = "minicbor::bytes")]
    pub control: Address,
    #[cbor(n(5), with = "minicbor::bytes")]
    pub bulk: Address,
}

impl Body for BindBody {
    const KIND: Kind = Kind::Bind;
    cbor_codec!();

    fn validate(&self) -> Result<(), FormatError> {
        require(
            seq_prev_ok(self.seq, &self.prev),
            field("bind", "prev", "zero iff seq = 0"),
        )
    }

    fn extra_attributes(&self, signer: &Nid, h: &Hash32) -> Vec<Attribute> {
        vec![
            Attribute::b32("ag.rid", self.rid),
            Attribute::b32("ag.nid", *signer),
            Attribute::b32("ag.h", *h),
            Attribute::u64("ag.seq", self.seq),
        ]
    }
}

/// `idacc`: a delegate's acceptance of an identity revision.
#[derive(Debug, Clone, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(map)]
pub struct IdAccBody {
    #[cbor(n(1), with = "minicbor::bytes")]
    pub rid: Rid,
    /// `h` of the revision being accepted.
    #[cbor(n(2), with = "minicbor::bytes")]
    pub rev: Hash32,
}

impl Body for IdAccBody {
    const KIND: Kind = Kind::IdAcc;
    cbor_codec!();

    fn validate(&self) -> Result<(), FormatError> {
        Ok(())
    }

    fn extra_attributes(&self, signer: &Nid, _h: &Hash32) -> Vec<Attribute> {
        vec![
            Attribute::b32("ag.rid", self.rid),
            Attribute::b32("ag.nid", *signer),
            Attribute::b32("ag.rev", self.rev),
        ]
    }
}
