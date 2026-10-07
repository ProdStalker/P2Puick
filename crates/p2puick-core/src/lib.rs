//! P2Puick core: framing, pairing, and LAN file transfer.

pub mod error;
pub mod exclude;
pub mod pairing;
pub mod protocol;
pub mod retry_queue;
pub mod transfer;

pub use error::{Error, Result};
pub use exclude::{default_exclude_dir_names, DEFAULT_EXCLUDE_DIR_NAMES};
pub use pairing::{generate_pairing_code, verify_pairing_code};
pub use protocol::{FileEntry, Manifest, Message, PROTOCOL_VERSION};
pub use retry_queue::{FailedEntry, RetryQueue};
pub use transfer::{
    ProgressEvent, ProgressKind, SessionConfig, TransferSession, DEFAULT_CONCURRENCY,
};
