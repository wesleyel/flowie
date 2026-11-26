//! Per-node sync (docs/roadmap/per-node-sync.md): the records devices exchange, how two
//! versions of a record merge, and the engine that moves them. No IO and no threads: each
//! platform drives the engine from its own transport.

pub mod engine;
pub mod memory;
pub mod merge;
pub mod store;
pub mod wire;

pub use engine::{ChangeBatch, Engine, RecordStore, SaveResult};
pub use memory::MemoryServer;
pub use merge::merge;
pub use store::MemoryStore;
pub use wire::{Envelope, Kind, NodeRecord, Position, Reg, VERSION};
