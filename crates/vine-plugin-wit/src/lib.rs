//! The WIT interface Vine plugins are built against, in `wit/` next to this crate.
//!
//! Nothing links against this crate. It exists so the interface lives with the other crates;
//! the plugin host and `vine-plugin-sdk` point their bindings at the folder directly.

/// Where the `.wit` files are on disk, for tools that want to generate their own bindings
pub const WIT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/wit");
