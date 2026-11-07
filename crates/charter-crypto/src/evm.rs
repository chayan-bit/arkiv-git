//! Per-repo secp256k1 (EVM) keys derived from the master seed (design section 3.1).
//!
//! Pending user decision 3 in docs/p0/README.md (derive EVM owner keys from the same seed
//! as the signing identity). This module is the only place that couples the two; swapping
//! it for independently generated EVM keys touches nothing else in this crate.
//!
//! Derivation: HKDF-SHA256 with `salt = "arkiv-git/v1/evm"`,
//! `info = role || rid || u32be(index)` (`rid` omitted for `wallet`), 32 bytes reduced into
//! the secp256k1 order. On a zero scalar (probability about 2^-256) the derivation retries
//! with `u32be(counter)` appended to `info`, counter 1, 2, ...

use crate::{Hash32, MasterSeed};
use k256::elliptic_curve::ops::Reduce;
use k256::elliptic_curve::sec1::ToEncodedPoint;
use k256::{NonZeroScalar, ProjectivePoint, Scalar, U256};
use sha3::{Digest, Keccak256};
use zeroize::{ZeroizeOnDrop, Zeroizing};

const EVM_SALT: &[u8] = b"arkiv-git/v1/evm"; // protocol constant: unchanged by the Charter rename

/// A 20-byte EVM address.
pub type Address = [u8; 20];

/// A derived EVM key: its 32-byte secret scalar (zeroized on drop) and address.
#[derive(Clone)]
pub struct EvmKey {
    secret: Zeroizing<[u8; 32]>,
    address: Address,
}

impl ZeroizeOnDrop for EvmKey {}

impl EvmKey {
    /// Big-endian secret scalar, for building a transaction signer.
    pub fn secret(&self) -> &[u8; 32] {
        &self.secret
    }

    pub fn address(&self) -> Address {
        self.address
    }
}

impl core::fmt::Debug for EvmKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "EvmKey(0x")?;
        self.address.iter().try_for_each(|b| write!(f, "{b:02x}"))?;
        f.write_str(")")
    }
}

/// Funds holder; tops up the per-repo keys.
pub fn wallet(seed: &MasterSeed, index: u32) -> EvmKey {
    derive(seed, &[b"wallet", &index.to_be_bytes()])
}

/// Creates the control-plane entities of one repo.
pub fn control(seed: &MasterSeed, rid: &Hash32, index: u32) -> EvmKey {
    derive(seed, &[b"control", rid, &index.to_be_bytes()])
}

/// Creates refs, manifests and chunks of one repo.
pub fn bulk(seed: &MasterSeed, rid: &Hash32, index: u32) -> EvmKey {
    derive(seed, &[b"bulk", rid, &index.to_be_bytes()])
}

/// The address of a secret scalar, or `None` if it is zero or not below the curve order.
pub fn address_of(secret: &[u8; 32]) -> Option<Address> {
    let s = Option::<NonZeroScalar>::from(NonZeroScalar::from_repr((*secret).into()))?;
    Some(address_of_scalar(&s))
}

fn address_of_scalar(s: &NonZeroScalar) -> Address {
    let point = (ProjectivePoint::GENERATOR * **s).to_affine();
    let enc = point.to_encoded_point(false);
    let hash = Keccak256::digest(&enc.as_bytes()[1..]);
    let mut a = [0u8; 20];
    a.copy_from_slice(&hash[12..]);
    a
}

fn derive(seed: &MasterSeed, parts: &[&[u8]]) -> EvmKey {
    let base: Vec<u8> = parts.concat();
    (0u32..)
        .find_map(|counter| {
            let info = if counter == 0 {
                base.clone()
            } else {
                [base.as_slice(), &counter.to_be_bytes()].concat()
            };
            let okm = seed.expand(EVM_SALT, &info);
            let scalar = <Scalar as Reduce<U256>>::reduce_bytes(&(*okm).into());
            Option::<NonZeroScalar>::from(NonZeroScalar::new(scalar))
        })
        .map(|s| EvmKey {
            secret: Zeroizing::new(s.to_bytes().into()),
            address: address_of_scalar(&s),
        })
        .unwrap_or_else(|| unreachable!("a nonzero scalar is found with overwhelming probability"))
}
