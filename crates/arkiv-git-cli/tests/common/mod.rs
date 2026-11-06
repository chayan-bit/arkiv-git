//! Shared test helpers: `e2e` drives the binaries against the file-persisted MemArkiv,
//! `live` drives them against Rabdi (ignored tests that spend testnet funds), and
//! `programs` adds repo-program helpers on top of `e2e`.

pub mod e2e;
pub mod live;
pub mod programs;
