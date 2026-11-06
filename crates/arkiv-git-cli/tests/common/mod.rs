//! Shared test helpers: `e2e` drives the binaries against the file-persisted MemArkiv,
//! `live` drives them against Rabdi (ignored tests that spend testnet funds),
//! `programs` adds repo-program helpers on top of `e2e`; `collab` runs multi-peer
//! flows on Rabdi on top of `live`.

pub mod collab;
pub mod e2e;
pub mod live;
pub mod programs;
