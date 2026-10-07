//! P2Puick core: framing, pairing, and LAN file transfer.

pub mod error;
pub mod pairing;
pub mod protocol;
pub mod transfer;

pub use error::{Error, Result};
pub use pairing::{generate_pairing_code, verify_pairing_code};
pub use protocol::{FileEntry, Manifest, Message, PROTOCOL_VERSION};
pub use transfer::{
    ProgressEvent, ProgressKind, SessionConfig, TransferSession, DEFAULT_CONCURRENCY,
};
