//! Per-user metadata: nickname overrides, free-form notes, block list.
//!
//! Persisted to `~/.config/livestreamlist/users.json`. The `UserStore` is
//! the only thing that touches the file; the rest of the app talks to it
//! through `Arc<UserStore>` (sync, parking_lot Mutex inside).

pub mod models;
pub mod store;

pub use models::{FieldUpdate, UserMetadata, UserMetadataPatch};
pub use store::UserStore;
