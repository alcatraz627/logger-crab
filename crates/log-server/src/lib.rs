//! logger-crab log service library surface. Re-exports the modules the
//! binary wires together. Downstream crates (tests, tooling) can depend on
//! `log_server::store` without pulling in `main.rs`.

pub mod config;
pub mod error;
pub mod models;
pub mod routes;
pub mod seed;
pub mod store;
