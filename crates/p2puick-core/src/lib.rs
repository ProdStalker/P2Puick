//! P2Puick core: framing and pairing.

pub mod error;
pub mod pairing;
pub mod protocol;

pub use error::{Error, Result};
pub use pairing::{generate_pairing_code, verify_pairing_code};
pub use protocol::{FileEntry, Manifest, Message, PROTOCOL_VERSION};
