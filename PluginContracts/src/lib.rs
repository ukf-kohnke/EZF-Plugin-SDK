//! Rust-side plugin SDK and logical plugin contracts.
//!
//! This crate is intentionally independent of the application `UseCases` crate.
//! It contains only the contract required by plugin implementations; plugin loading,
//! lifecycle management and dynamic-library handling live in `PluginHost`.
#![allow(non_snake_case)]
pub use DatabaseContracts;
pub use PluginAbi;

pub mod ApiContract;
pub mod IPlugin;
