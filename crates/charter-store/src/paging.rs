//! Paging rules (design 4.8): keyset paging on a `u64` attribute, and offset paging with
//! a count check. No cursors: the Rabdi SDK does not expose them.

use std::collections::HashSet;

use alloy_primitives::B256;
use arkiv_rpc_types::EntityData;

use crate::{Arkiv, Attrs, MAX_LIMIT, Pred, Query, QueryOpts, RowExt, Select, StoreError, attr};

/// Restarts allowed when the match count changes between offset pages.
pub const MAX_RESTARTS: usize = 3;

/// Every entity matching `filter` with `name >= from`, oldest value first, each once.
///
/// Requests `name >= last ORDER BY name:u64`, drops keys already seen, and halves the page
/// on HTTP 413 or 5xx. More than one page sharing one value is `E_PAGING`.
pub async fn keyset<A: Arkiv>(
    a: &A,
    filter: &Pred,
    name: &str,
    from: u64,
    select: Select,
) -> Result<Vec<EntityData>, StoreError> {
    let select = with_key_and(select, name);
    let (mut last, mut limit) = (from, MAX_LIMIT);
    let mut seen: HashSet<B256> = HashSet::new();
    let mut out = Vec::new();
    loop {
        let q = Query::from(Pred::And(vec![filter.clone(), Pred::ge(attr(name), last)]))
            .order_by_u64(name);
        let opts = QueryOpts {
            limit,
            offset: 0,
            select: select.clone(),
        };
        let page = match a.query(&q, &opts).await {
            Err(StoreError::Http { status, .. })
                if (status == 413 || status >= 500) && limit > 1 =>
            {
                limit /= 2;
                continue;
            }
            other => other?,
        };
        let full = page.rows.len() == limit as usize;
        let mut max = last;
        for row in page.rows {
            let key = row
                .key
                .ok_or_else(|| StoreError::Rpc("row without key".into()))?;
            let v = row.attr_u64(name).filter(|v| *v >= last).ok_or_else(|| {
                StoreError::InconsistentView(format!("row {key} lacks {name} >= {last}"))
            })?;
            max = max.max(v);
            if seen.insert(key) {
                out.push(row);
            }
        }
        if !full {
            return Ok(out);
        }
        if max == last {
            return Err(StoreError::Paging(format!(
                "more than {limit} entities share {name} = {last}"
            )));
        }
        last = max;
    }
}

/// Every entity matching `q` in node order, by offset pages, each page selecting `count`;
/// a count change restarts, at most [`MAX_RESTARTS`] times, then `E_INCONSISTENT_VIEW`.
pub async fn offset_all<A: Arkiv>(
    a: &A,
    q: &Query,
    select: Select,
) -> Result<Vec<EntityData>, StoreError> {
    let select = Select {
        count: true,
        ..select
    };
    'attempt: for _ in 0..=MAX_RESTARTS {
        let (mut out, mut expected) = (Vec::new(), None);
        loop {
            let opts = QueryOpts {
                limit: MAX_LIMIT,
                offset: out.len() as u64,
                select: select.clone(),
            };
            let page = a.query(q, &opts).await?;
            let count = page
                .count
                .ok_or_else(|| StoreError::Rpc("count missing".into()))?;
            if expected.is_some_and(|e| e != count) {
                continue 'attempt;
            }
            expected = Some(count);
            let n = page.rows.len();
            out.extend(page.rows);
            if n < MAX_LIMIT as usize || out.len() as u64 >= count {
                return Ok(out);
            }
        }
    }
    Err(StoreError::InconsistentView(format!(
        "match count kept changing over {MAX_RESTARTS} restarts"
    )))
}

fn with_key_and(select: Select, name: &str) -> Select {
    let attrs = match select.attrs {
        Attrs::All => Attrs::All,
        Attrs::None => Attrs::Named(vec![name.to_string()]),
        Attrs::Named(mut ns) => {
            if !ns.iter().any(|n| n == name) {
                ns.push(name.to_string());
            }
            Attrs::Named(ns)
        }
    };
    Select {
        key: true,
        count: false,
        attrs,
        ..select
    }
}
