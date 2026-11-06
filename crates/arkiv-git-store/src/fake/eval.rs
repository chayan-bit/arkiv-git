//! Query evaluation for `MemArkiv`: the node's parser, filter and sort, with GolemDB's quirks.

use arkiv_interfaces::entity_records::to_cells;
use arkiv_interfaces::query::Query as Ast;
use arkiv_interfaces::store::{Filter, Record, RecordKey, RecordVersion, Sort, Test, TypeId};
use arkiv_rpc_types::{Projection, QueryOptions, QueryResponse, entity_data_from, resolve_limit};
use serde_json::Value as Json;

use super::exec::Stored;
use crate::StoreError;

fn invalid(msg: impl std::fmt::Display) -> StoreError {
    StoreError::InvalidQuery(msg.to_string())
}

/// Answer `arkiv_query(q, opts)` over `live` (already in creation order) at block `head`.
pub fn run(live: &[&Stored], q: &str, opts: &Json, head: u64) -> Result<QueryResponse, StoreError> {
    let opts: QueryOptions = serde_json::from_value(opts.clone()).map_err(invalid)?;
    if opts.at_block.is_some() {
        return Err(invalid("state for this block is not retained"));
    }
    let limit = resolve_limit(opts.limit).map_err(invalid)?;
    let projection = Projection::resolve(opts.select.as_ref()).map_err(invalid)?;
    let stmt = arkiv_query::parse_statement(q).map_err(invalid)?;
    if has_not(&stmt.filter) {
        // GolemDB has no NOT (conformance Q1).
        return Err(invalid("count: store: NotImplemented"));
    }
    let filter = conflate(Filter::from(&stmt.filter));
    let mut hits: Vec<(&Stored, Record)> = Vec::new();
    for s in live {
        let r = record(s)?;
        if filter.matches(&r).map_err(|e| invalid(format!("{e:?}")))? {
            hits.push((s, r));
        }
    }
    if let Some(order) = &stmt.order {
        let mut sort = Sort::from(order);
        sort.type_id = conflate_type(sort.type_id);
        // Stable sort: ties keep creation order in either direction.
        hits.sort_by(|(_, a), (_, b)| {
            let (ka, kb) = (a.sort_key(&sort), b.sort_key(&sort));
            if order.direction == arkiv_interfaces::query::Direction::Descending {
                kb.cmp(&ka)
            } else {
                ka.cmp(&kb)
            }
        });
    }
    let count = hits.len() as u64;
    let data = if projection.is_count_only() {
        Vec::new()
    } else {
        let offset = usize::try_from(opts.offset.unwrap_or(0)).unwrap_or(usize::MAX);
        hits.iter()
            .skip(offset)
            .take(limit as usize)
            .map(|(s, _)| entity_data_from(s.entity.clone(), &projection))
            .collect()
    };
    Ok(QueryResponse {
        data,
        block_number: head,
        cursor: None,
        count: projection.count.then_some(count),
    })
}

fn has_not(q: &Ast) -> bool {
    match q {
        Ast::Not(_) => true,
        Ast::And(xs) | Ast::Or(xs) => xs.iter().any(has_not),
        _ => false,
    }
}

/// GolemDB stores `key` and `bytes32` as one type (conformance Q2).
fn conflate_type(t: TypeId) -> TypeId {
    if t == TypeId::KEY { TypeId::BYTES32 } else { t }
}

fn conflate(f: Filter) -> Filter {
    match f {
        Filter::Predicate(mut p) => {
            match &mut p.test {
                Test::Compare { type_id, .. } | Test::Between { type_id, .. } => {
                    *type_id = conflate_type(*type_id)
                }
                Test::Exists { type_id } => *type_id = type_id.map(conflate_type),
            }
            Filter::Predicate(p)
        }
        Filter::And(xs) => Filter::And(xs.into_iter().map(conflate).collect()),
        Filter::Or(xs) => Filter::Or(xs.into_iter().map(conflate).collect()),
        Filter::Not(x) => Filter::Not(Box::new(conflate(*x))),
        Filter::All => Filter::All,
    }
}

fn record(s: &Stored) -> Result<Record, StoreError> {
    let mut cells = to_cells(&s.entity).map_err(|e| invalid(format!("{e:?}")))?;
    for (_, c) in &mut cells {
        c.type_id = conflate_type(c.type_id);
    }
    Ok(Record {
        key: RecordKey::from_entity(s.entity.key),
        version: RecordVersion(0),
        cells,
        meta: None,
    })
}
