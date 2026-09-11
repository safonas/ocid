//! Shared building blocks for the `ocid` daemon and the `ocictl` CLI.
//!
//! Everything here is plain data + on-disk JSON/TOML handling; no networking,
//! no blob store. That keeps `ocictl` small and lets it read the index while
//! the daemon is stopped.

pub mod api;
pub mod config;
pub mod hash;
pub mod identity;
pub mod index;
pub mod oci;
pub mod paths;
pub mod release;
