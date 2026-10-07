use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("pairing rejected")]
    PairingRejected,

    #[error("unexpected message: {0}")]
    UnexpectedMessage(String),

    #[error("hash mismatch for {path}: expected {expected}, got {actual}")]
    HashMismatch {
        path: String,
        expected: String,
        actual: String,
    },

    #[error("transfer cancelled")]
    Cancelled,

    #[error("{0}")]
    Other(String),
}

impl Error {
    pub fn protocol(msg: impl Into<String>) -> Self {
        Self::Protocol(msg.into())
    }

    pub fn from_io(err: std::io::Error) -> Self {
        match err.kind() {
            std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::BrokenPipe => Self::Other(
                "Connexion interrompue (reset). Souvent dû à un plantage réseau, une app fermée, ou une surcharge — réessaie le transfert."
                    .into(),
            ),
            std::io::ErrorKind::TimedOut => {
                Self::Other("Connexion expirée (timeout). Vérifie le Wi‑Fi et réessaie.".into())
            }
            std::io::ErrorKind::ConnectionRefused => Self::Other(
                "Connexion refusée. Sur l’hôte, clique d’abord « Attendre le pair & envoyer » (il doit écouter), puis rejoins depuis cet ordinateur."
                    .into(),
            ),
            std::io::ErrorKind::HostUnreachable
            | std::io::ErrorKind::NetworkUnreachable => Self::Other(
                "Hôte injoignable (pas de route). Même Wi‑Fi ? Saisis l’IP LAN affichée côté hôte (évite 169.254.x) et vérifie le pare-feu."
                    .into(),
            ),
            _ => {
                // macOS EHOSTUNREACH=65, Linux=113 — older Rust may not map the kind.
                match err.raw_os_error() {
                    Some(65) | Some(113) => Self::Other(
                        "Hôte injoignable (pas de route). Même Wi‑Fi ? Saisis l’IP LAN affichée côté hôte (évite 169.254.x) et vérifie le pare-feu."
                            .into(),
                    ),
                    Some(61) | Some(111) => Self::Other(
                        "Connexion refusée. Sur l’hôte, clique d’abord « Attendre le pair & envoyer », puis rejoins."
                            .into(),
                    ),
                    _ => Self::Io(err),
                }
            }
        }
    }
}
