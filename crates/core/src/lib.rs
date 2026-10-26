//! writepad-core: the engine behind WRITEpad. The desktop app, the `writepad`
//! CLI and its MCP server all go through here, so they agree on the on-disk
//! format and can safely work on one vault at the same time.

pub mod diff;
pub mod epub;
pub mod export;
pub mod history;
pub mod manuscript;
pub mod narrate;
pub mod pdf;
pub mod profile;
pub mod script;
pub mod search;
pub mod suggest;
pub mod vault;
pub mod world;

pub use manuscript::write_atomic;
