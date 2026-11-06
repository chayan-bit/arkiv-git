//! Entity state and op execution for `MemArkiv`, mirroring the node's executor rules.

use std::collections::{BTreeMap, HashMap};

use alloy_primitives::{Address, B256, U256};
use arkiv_bindings::types::validate_ident32_bytes;
use arkiv_bindings::{
    Attribute as WireAttr, Create, Delete, ENTITY_MAX_ATTRIBUTES, ExtendExpiry,
    IEntityRegistry as R, OP_CREATE, OP_DELETE, OP_EXTEND_EXPIRY, OP_PATCH, OP_TRANSFER_OWNERSHIP,
    Operation, Patch, TransferOwnership,
};
use arkiv_interfaces::entity::{Attribute, CreationFlags, Entity};

use crate::derive_key;

/// Longest remaining lifetime a create or extend may buy (Rabdi genesis).
pub const MAX_LIFETIME: u64 = 7_884_000;
/// Measured: each op after the first in a tx costs about this much compute.
const OP_GAS: u64 = 21_400;
/// Storage units of an entity beyond its attribute cells (calibrated on the 64 KiB create).
const SYSTEM_UNITS: u64 = 1_872;

/// A live entity and its position in creation order (block, tx index).
#[derive(Debug, Clone)]
pub struct Stored {
    pub entity: Entity,
    pub tx_pos: (u64, u32),
}

/// Everything a tx can change; cloned to roll back a revert or dry-run an estimate.
#[derive(Debug, Clone, Default)]
pub struct World {
    pub entities: BTreeMap<[u8; 32], Stored>,
    pub entity_nonce: HashMap<Address, u64>,
    pub balances: HashMap<Address, U256>,
    pub nonces: HashMap<Address, u64>,
}

/// Where and by whom a tx executes.
pub struct Env {
    pub chain_id: u64,
    pub caller: Address,
    pub block: u64,
    pub tx_index: u32,
}

type Revert = String;

/// The same text `decode_revert` gives for the node's revert data.
fn revert<E: alloy_sol_types::SolError>(e: E) -> Revert {
    crate::decode_revert(&e.abi_encode()).unwrap_or_else(|| E::SIGNATURE.to_string())
}

impl World {
    /// Apply every op or none; returns the storage-and-op gas on success.
    pub fn execute(&mut self, env: &Env, ops: &[Operation]) -> Result<u64, Revert> {
        if ops.is_empty() {
            return Err(revert(R::EmptyBatch {}));
        }
        let snapshot = self.clone();
        let mut gas = OP_GAS * (ops.len() as u64 - 1);
        for op in ops {
            match self.apply(env, op) {
                Ok(g) => gas += g,
                Err(e) => {
                    *self = snapshot;
                    return Err(e);
                }
            }
        }
        Ok(gas)
    }

    fn apply(&mut self, env: &Env, op: &Operation) -> Result<u64, Revert> {
        let bad = || {
            revert(R::NonCanonicalOperationData {
                operation: op.operation,
            })
        };
        match op.operation {
            OP_CREATE => self.create(env, op.payload_of::<Create>().map_err(|_| bad())?),
            OP_EXTEND_EXPIRY => {
                self.extend(env, op.payload_of::<ExtendExpiry>().map_err(|_| bad())?)
            }
            OP_DELETE => {
                let d = op.payload_of::<Delete>().map_err(|_| bad())?;
                self.owned(env, d.entityKey)?;
                self.entities.remove(&d.entityKey.0);
                Ok(0)
            }
            OP_PATCH => self.patch(env, op.payload_of::<Patch>().map_err(|_| bad())?),
            OP_TRANSFER_OWNERSHIP => self.transfer(
                env,
                op.payload_of::<TransferOwnership>().map_err(|_| bad())?,
            ),
            other => Err(revert(R::InvalidOpType { operation: other })),
        }
    }

    fn create(&mut self, env: &Env, c: Create) -> Result<u64, Revert> {
        let flags = CreationFlags::from_bits(c.creationFlags).ok_or_else(|| {
            revert(R::ReservedCreationFlags {
                creationFlags: c.creationFlags,
            })
        })?;
        let attributes = decode_attributes(&c.attributes, false)?;
        let expires_at = admit(
            env.block,
            c.expiresAt.max(env.block.saturating_add(c.minLifetime)),
        )?;
        let nonce = self.entity_nonce.entry(env.caller).or_default();
        let key = derive_key(env.chain_id, env.caller, *nonce, c.salt);
        *nonce += 1;
        let entity = Entity {
            key: key.0,
            creator: env.caller.into_array(),
            owner: env.caller.into_array(),
            created_at: env.block,
            updated_at: env.block,
            expires_at,
            creation_flags: flags,
            attributes,
        };
        let gas = storage_gas(units(&entity), expires_at - env.block);
        self.entities.insert(
            key.0,
            Stored {
                entity,
                tx_pos: (env.block, env.tx_index),
            },
        );
        Ok(gas)
    }

    fn extend(&mut self, env: &Env, x: ExtendExpiry) -> Result<u64, Revert> {
        let e = self.live(env, x.entityKey)?;
        let may = e.owner == env.caller.into_array()
            || e.creation_flags.allows_permissionless_extension();
        if !may {
            return Err(not_owner(x.entityKey, env.caller, e.owner));
        }
        let target = x.expiresAt.max(env.block.saturating_add(x.minLifetime));
        let current = e.expires_at;
        if target < current {
            let err = R::ExpiryNotExtended {
                entityKey: x.entityKey,
                newExpiresAt: target,
                currentExpiresAt: current,
            };
            return Err(revert(err));
        }
        if target == current {
            return Ok(0);
        }
        admit(env.block, target)?;
        let gas = storage_gas(units(e), target - current);
        let stored = self
            .entities
            .get_mut(&x.entityKey.0)
            .ok_or_else(|| not_found(x.entityKey))?;
        stored.entity.expires_at = target;
        stored.entity.updated_at = env.block;
        Ok(gas)
    }

    fn patch(&mut self, env: &Env, p: Patch) -> Result<u64, Revert> {
        let e = self.owned(env, p.entityKey)?;
        if e.creation_flags.is_readonly() {
            return Err(revert(R::ReadOnlyEntity {
                entityKey: p.entityKey,
            }));
        }
        if p.mutations.is_empty() {
            return Err(revert(R::EmptyMutations {
                entityKey: p.entityKey,
            }));
        }
        let mut entity = e.clone();
        for (m, decoded) in p
            .mutations
            .iter()
            .zip(decode_attributes(&p.mutations, true)?)
        {
            entity.attributes.retain(|a| a.key != decoded.key);
            if !m.is_tombstone() {
                entity.attributes.push(decoded);
            }
        }
        entity.attributes.sort_by(|a, b| a.key.cmp(&b.key));
        if entity.attributes.len() > ENTITY_MAX_ATTRIBUTES {
            return Err(too_many(entity.attributes.len()));
        }
        entity.updated_at = env.block;
        let gas = storage_gas(units(&entity), entity.expires_at - env.block);
        if let Some(s) = self.entities.get_mut(&p.entityKey.0) {
            s.entity = entity;
        }
        Ok(gas)
    }

    fn transfer(&mut self, env: &Env, t: TransferOwnership) -> Result<u64, Revert> {
        let e = self.owned(env, t.entityKey)?;
        if t.newOwner == Address::ZERO {
            return Err(revert(R::TransferToZeroAddress {
                entityKey: t.entityKey,
            }));
        }
        if t.newOwner.into_array() == e.owner {
            return Err(revert(R::TransferToSelf {
                entityKey: t.entityKey,
            }));
        }
        if let Some(s) = self.entities.get_mut(&t.entityKey.0) {
            s.entity.owner = t.newOwner.into_array();
            s.entity.updated_at = env.block;
        }
        Ok(0)
    }

    fn live(&self, env: &Env, key: B256) -> Result<&Entity, Revert> {
        let e = &self
            .entities
            .get(&key.0)
            .ok_or_else(|| not_found(key))?
            .entity;
        if e.expires_at <= env.block {
            return Err(revert(R::EntityExpired {
                entityKey: key,
                expiresAt: e.expires_at,
            }));
        }
        Ok(e)
    }

    fn owned(&self, env: &Env, key: B256) -> Result<&Entity, Revert> {
        let e = self.live(env, key)?;
        if e.owner != env.caller.into_array() {
            return Err(not_owner(key, env.caller, e.owner));
        }
        Ok(e)
    }
}

fn not_found(key: B256) -> Revert {
    revert(R::EntityNotFound { entityKey: key })
}

fn not_owner(key: B256, caller: Address, owner: [u8; 20]) -> Revert {
    revert(R::NotOwner {
        entityKey: key,
        caller,
        owner: Address::from(owner),
    })
}

fn too_many(n: usize) -> Revert {
    revert(R::TooManyAttributes {
        count: U256::from(n),
        maxCount: U256::from(ENTITY_MAX_ATTRIBUTES),
    })
}

/// The resolved expiry, or the node's revert for a dead-on-arrival or too-long lifetime.
fn admit(block: u64, target: u64) -> Result<u64, Revert> {
    if target <= block {
        return Err(revert(R::ExpiryDeadOnArrival {
            target,
            currentBlock: block,
        }));
    }
    if target - block > MAX_LIFETIME {
        let e = R::ExpiryBeyondMaxLifetime {
            target,
            currentBlock: block,
            maxLifetime: MAX_LIFETIME,
        };
        return Err(revert(e));
    }
    Ok(target)
}

/// Attribute rules: count, name grammar, strictly ascending names, typed values.
fn decode_attributes(wire: &[WireAttr], tombstones_ok: bool) -> Result<Vec<Attribute>, Revert> {
    if wire.len() > ENTITY_MAX_ATTRIBUTES {
        return Err(too_many(wire.len()));
    }
    let mut out = Vec::with_capacity(wire.len());
    for (i, a) in wire.iter().enumerate() {
        validate_ident32_bytes(&a.name.0).map_err(|e| format!("Ident32InvalidByte({e:?})"))?;
        if i > 0 && wire[i - 1].name.0 >= a.name.0 {
            return Err(revert(R::AttributesNotSorted {}));
        }
        let name = a
            .name
            .0
            .iter()
            .take_while(|b| **b != 0)
            .copied()
            .collect::<Vec<u8>>();
        let bad_type = || {
            revert(R::InvalidValueType {
                name: a.name,
                typeId: a.typeId,
            })
        };
        match a.to_value().map_err(|_| bad_type())? {
            Some(value) => out.push(Attribute {
                key: name,
                value,
                indexed: a.is_indexed(),
            }),
            None if tombstones_ok => out.push(Attribute::unindexed(
                name,
                arkiv_bindings::AttributeValue::Bool(false),
            )),
            None => return Err(revert(R::TombstoneInCreate { name: a.name })),
        }
    }
    Ok(out)
}

/// Storage units: cell bytes once, index bytes twice, per-cell overheads (tokenomics weights).
pub fn units(e: &Entity) -> u64 {
    SYSTEM_UNITS
        + e.attributes
            .iter()
            .map(|a| {
                let len = a.value.encode().len() as u64;
                len + 64 + if a.indexed { 2 * len + 65 } else { 0 }
            })
            .sum::<u64>()
}

/// `ceil(units * blocks * 14 / 1e6)`.
pub fn storage_gas(units: u64, blocks: u64) -> u64 {
    (u128::from(units) * u128::from(blocks) * 14).div_ceil(1_000_000) as u64
}

/// Tx compute floor: `21000 + 10 * tokens`, tokens = zero bytes + 4 x non-zero bytes.
pub fn intrinsic_gas(calldata: &[u8]) -> u64 {
    let tokens: u64 = calldata.iter().map(|b| if *b == 0 { 1 } else { 4 }).sum();
    21_000 + 10 * tokens
}
