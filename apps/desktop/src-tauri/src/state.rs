use p2puick_core::TransferSession;
use p2puick_discovery::Advertisement;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Default)]
pub struct AppState {
    pub inner: Mutex<InnerState>,
}

pub struct InnerState {
    pub pairing_code: Option<String>,
    pub role: Option<Role>,
    pub listen_port: u16,
    pub peer_addr: Option<String>,
    pub source_paths: Vec<PathBuf>,
    pub dest_dir: Option<PathBuf>,
    pub advertisement: Option<Advertisement>,
    pub transfer: Option<TransferSession>,
    pub host_task_running: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Host,
    Joiner,
}

impl Default for InnerState {
    fn default() -> Self {
        Self {
            pairing_code: None,
            role: None,
            listen_port: p2puick_discovery::DEFAULT_PORT,
            peer_addr: None,
            source_paths: Vec::new(),
            dest_dir: None,
            advertisement: None,
            transfer: None,
            host_task_running: false,
        }
    }
}
