//! Per-node sync (docs/roadmap/per-node-sync.md): the records devices exchange, how two
//! versions of a record merge, and the engine that moves them. No IO and no threads: each
//! platform drives the engine from its own transport.

pub mod engine;
#[cfg(feature = "http")]
pub mod http;
pub mod memory;
pub mod merge;
pub mod store;
pub mod wire;

pub use engine::{sync_once, Backend, ChangeBatch, Engine, RecordStore, SaveResult, SyncReport};
pub use memory::MemoryServer;
pub use merge::merge;
pub use store::MemoryStore;
pub use wire::{Envelope, Kind, NodeRecord, Position, Reg, VERSION};
