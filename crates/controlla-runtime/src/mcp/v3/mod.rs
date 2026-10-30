//! Extracted V3 execution-core subsystems.
//!
//! `v3_full.rs` remains the MCP facade while these modules own shared policy
//! types, pointer geometry, typing transactions, and receipt compaction. The
//! modules are compiled into the binary that hosts the V3 server.

#[allow(dead_code)]
pub mod act;
#[allow(dead_code)]
pub mod browser;
#[allow(dead_code)]
pub mod canvas;
#[allow(dead_code)]
pub mod extract;
#[allow(dead_code)]
pub mod pointer;
#[allow(dead_code)]
pub mod shared;
#[allow(dead_code)]
pub mod snapshot;
#[allow(dead_code)]
pub mod task;
#[allow(dead_code)]
pub mod typing;
#[allow(dead_code)]
pub mod verify;
#[allow(dead_code)]
pub mod workflow;
