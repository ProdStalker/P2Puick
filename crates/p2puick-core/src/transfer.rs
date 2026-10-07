use crate::error::{Error, Result};
use crate::exclude::{default_exclude_dir_names, is_excluded_dir_name, normalize_excludes};
use crate::protocol::{
    read_message, write_message, AckPayload, ErrorPayload, FileEndPayload, FileEntry,
    FileStartPayload, HelloPayload, Manifest, Message, CHUNK_SIZE,
};
use crate::retry_queue::{self, FailedEntry};
use blake3::Hasher;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::fs::{self, File};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex};
use walkdir::WalkDir;

/// Min interval between high-frequency progress events (avoids flooding Tauri IPC).
const PROGRESS_MIN_INTERVAL: Duration = Duration::from_millis(200);

pub const DEFAULT_CONCURRENCY: usize = 4;
pub const DEFAULT_PORT: u16 = 47821;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressEvent {
    pub kind: ProgressKind,
    pub relative_path: String,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub files_done: u64,
    pub files_total: u64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProgressKind {
    Connected,
    Preparing,
    Manifest,
    FileStart,
    FileProgress,
    FileDone,
    /// Single file failed but transfer may continue; queued for retry when configured.
    FileFailed,
    Complete,
    Error,
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub pairing_code: String,
    pub hostname: String,
    pub concurrency: usize,
    /// Directory names to skip while walking (e.g. `node_modules`).
    pub exclude_dir_names: Vec<String>,
    /// Optional JSON file where failed/pending files are recorded for later retry.
    pub retry_queue_path: Option<PathBuf>,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            pairing_code: String::new(),
            hostname: hostname(),
            concurrency: DEFAULT_CONCURRENCY,
            exclude_dir_names: default_exclude_dir_names(),
            retry_queue_path: None,
        }
    }
}

#[derive(Clone)]
pub struct TransferSession {
    cancel: Arc<AtomicBool>,
}

impl TransferSession {
    pub fn new() -> Self {
        Self {
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    /// Host: listen, accept one peer, handshake as sender, wait for destination ready, then send.
    pub async fn host_and_send(
        &self,
        port: u16,
        config: SessionConfig,
        source_paths: Vec<PathBuf>,
        progress: mpsc::UnboundedSender<ProgressEvent>,
    ) -> Result<()> {
        // Fast size inventory (no hashing) while waiting for the peer.
        // Files are blake3-hashed on the fly during send — one I/O pass.
        let prep_progress = progress.clone();
        let prep_cancel = self.cancel.clone();
        let prep_paths = source_paths.clone();
        let excludes = normalize_excludes(&config.exclude_dir_names);
        let prep_task = tokio::spawn(async move {
            scan_inventory(&prep_paths, Some(&prep_progress), &prep_cancel, &excludes).await
        });
        self.host_listen_and_send(port, config, prep_task, progress)
            .await
    }

    /// Host: resend specific failed entries, preserving original relative paths.
    pub async fn host_and_resend(
        &self,
        port: u16,
        config: SessionConfig,
        entries: Vec<FailedEntry>,
        progress: mpsc::UnboundedSender<ProgressEvent>,
    ) -> Result<()> {
        let prep_task = tokio::spawn(async move { Ok(manifest_from_retry_entries(entries)) });
        self.host_listen_and_send(port, config, prep_task, progress)
            .await
    }

    async fn host_listen_and_send(
        &self,
        port: u16,
        config: SessionConfig,
        prep_task: tokio::task::JoinHandle<Result<(Manifest, RootMap)>>,
        progress: mpsc::UnboundedSender<ProgressEvent>,
    ) -> Result<()> {
        let listener = bind_listener(port).await.map_err(|e| map_bind_error(e, port))?;
        let _ = progress.send(ProgressEvent {
            kind: ProgressKind::Connected,
            relative_path: String::new(),
            bytes_done: 0,
            bytes_total: 0,
            files_done: 0,
            files_total: 0,
            message: format!("En attente d’une connexion sur le port {port}…"),
        });

        let (stream, peer) = accept_one(&listener, &self.cancel).await?;
        drop(listener);
        tune_tcp(&stream);

        tracing::info!("peer connected from {peer}");
        let _ = progress.send(ProgressEvent {
            kind: ProgressKind::Connected,
            relative_path: String::new(),
            bytes_done: 0,
            bytes_total: 0,
            files_done: 0,
            files_total: 0,
            message: format!("Pair connecté ({})", peer.ip()),
        });

        let (reader, writer) = stream.into_split();
        let writer = Arc::new(Mutex::new(writer));
        let mut reader = reader;

        match read_message(&mut reader).await? {
            Message::Hello(hello) => {
                if hello.pairing_code.trim() != config.pairing_code.trim() {
                    let mut w = writer.lock().await;
                    write_message(
                        &mut *w,
                        &Message::Error(ErrorPayload {
                            message: "pairing rejected".into(),
                        }),
                    )
                    .await?;
                    prep_task.abort();
                    return Err(Error::PairingRejected);
                }
            }
            other => {
                prep_task.abort();
                return Err(Error::UnexpectedMessage(format!("{other:?}")));
            }
        }

        {
            let mut w = writer.lock().await;
            write_message(
                &mut *w,
                &Message::HelloAck(HelloPayload {
                    pairing_code: config.pairing_code.clone(),
                    role: "host".into(),
                    hostname: config.hostname.clone(),
                }),
            )
            .await?;
        }

        let _ = progress.send(ProgressEvent {
            kind: ProgressKind::Preparing,
            relative_path: String::new(),
            bytes_done: 0,
            bytes_total: 0,
            files_done: 0,
            files_total: 0,
            message: "Handshake OK — inventaire des fichiers…".into(),
        });

        match read_message(&mut reader).await.map_err(|e| match e {
            Error::Io(io) => Error::from_io(io),
            other => other,
        })? {
            Message::Ready => {}
            Message::Cancel => {
                prep_task.abort();
                return Err(Error::Cancelled);
            }
            other => {
                prep_task.abort();
                return Err(Error::UnexpectedMessage(format!("{other:?}")));
            }
        }

        let (manifest, roots) = prep_task
            .await
            .map_err(|e| Error::Other(format!("préparation interrompue: {e}")))??;

        if manifest.files.is_empty() {
            return Err(Error::Other(
                "Aucun fichier à envoyer (sélection vide ou dossier sans fichiers).".into(),
            ));
        }

        let files_total = manifest.files.len() as u64;
        let bytes_total = manifest.total_bytes;
        let _ = progress.send(ProgressEvent {
            kind: ProgressKind::Manifest,
            relative_path: String::new(),
            bytes_done: 0,
            bytes_total,
            files_done: 0,
            files_total,
            message: format!("Envoi de {files_total} fichier(s) (hash à la volée)…"),
        });

        {
            let mut w = writer.lock().await;
            write_message(&mut *w, &Message::Manifest(manifest.clone())).await?;
        }

        let report = send_files(
            writer,
            &mut reader,
            &roots,
            &manifest.files,
            self.cancel.clone(),
            progress.clone(),
            bytes_total,
        )
        .await;

        persist_send_report(config.retry_queue_path.as_deref(), &report);

        match report.fatal {
            None if report.failed.is_empty() => Ok(()),
            None => {
                let _ = progress.send(ProgressEvent {
                    kind: ProgressKind::Complete,
                    relative_path: String::new(),
                    bytes_done: bytes_total,
                    bytes_total,
                    files_done: report.sent.len() as u64,
                    files_total,
                    message: format!(
                        "Terminé avec {} échec(s) enregistré(s) pour renvoi.",
                        report.failed.len()
                    ),
                });
                Ok(())
            }
            Some(err) => {
                let _ = progress.send(ProgressEvent {
                    kind: ProgressKind::Error,
                    relative_path: String::new(),
                    bytes_done: 0,
                    bytes_total,
                    files_done: report.sent.len() as u64,
                    files_total,
                    message: if report.failed.is_empty() {
                        err.to_string()
                    } else {
                        format!(
                            "{err} — {} fichier(s) en file de renvoi.",
                            report.failed.len()
                        )
                    },
                });
                Err(err)
            }
        }
    }

    /// Join host, handshake as receiver, signal ready with dest dir, then receive.
    pub async fn join_and_receive(
        &self,
        addr: &str,
        config: SessionConfig,
        dest_dir: PathBuf,
        progress: mpsc::UnboundedSender<ProgressEvent>,
    ) -> Result<()> {
        self.join_and_receive_addrs(&[addr.to_string()], config, dest_dir, progress)
            .await
    }

    /// Try several `host:port` candidates (mDNS often exposes multiple IPv4s).
    pub async fn join_and_receive_addrs(
        &self,
        addrs: &[String],
        config: SessionConfig,
        dest_dir: PathBuf,
        progress: mpsc::UnboundedSender<ProgressEvent>,
    ) -> Result<()> {
        if addrs.is_empty() {
            return Err(Error::Other("Aucune adresse hôte à joindre.".into()));
        }

        let mut last_err: Option<Error> = None;
        let mut stream = None;
        let mut used_addr = addrs[0].clone();
        for addr in addrs {
            let _ = progress.send(ProgressEvent {
                kind: ProgressKind::Connected,
                relative_path: String::new(),
                bytes_done: 0,
                bytes_total: 0,
                files_done: 0,
                files_total: 0,
                message: format!("Connexion vers {addr}…"),
            });
            match TcpStream::connect(addr).await {
                Ok(s) => {
                    stream = Some(s);
                    used_addr = addr.clone();
                    break;
                }
                Err(e) => {
                    last_err = Some(Error::from_io(e));
                }
            }
        }
        let stream = match stream {
            Some(s) => s,
            None => {
                let err = last_err.unwrap_or_else(|| {
                    Error::Other("Impossible de joindre l’hôte (aucune adresse).".into())
                });
                let _ = progress.send(ProgressEvent {
                    kind: ProgressKind::Error,
                    relative_path: String::new(),
                    bytes_done: 0,
                    bytes_total: 0,
                    files_done: 0,
                    files_total: 0,
                    message: err.to_string(),
                });
                return Err(err);
            }
        };

        tune_tcp(&stream);
        let peer_label = stream
            .peer_addr()
            .map(|a| a.ip().to_string())
            .unwrap_or(used_addr);
        let _ = progress.send(ProgressEvent {
            kind: ProgressKind::Connected,
            relative_path: String::new(),
            bytes_done: 0,
            bytes_total: 0,
            files_done: 0,
            files_total: 0,
            message: format!("Connecté à l’hôte {peer_label}"),
        });

        let (mut reader, mut writer) = stream.into_split();

        write_message(
            &mut writer,
            &Message::Hello(HelloPayload {
                pairing_code: config.pairing_code.clone(),
                role: "joiner".into(),
                hostname: config.hostname.clone(),
            }),
        )
        .await?;

        match read_message(&mut reader).await? {
            Message::HelloAck(_) => {}
            Message::Error(e) => {
                return Err(Error::Other(e.message));
            }
            other => return Err(Error::UnexpectedMessage(format!("{other:?}"))),
        }

        write_message(&mut writer, &Message::Ready).await?;

        let _ = progress.send(ProgressEvent {
            kind: ProgressKind::Preparing,
            relative_path: String::new(),
            bytes_done: 0,
            bytes_total: 0,
            files_done: 0,
            files_total: 0,
            message: "En attente du manifeste (inventaire côté hôte)…".into(),
        });

        let manifest = match read_message(&mut reader).await? {
            Message::Manifest(m) => m,
            Message::Cancel => return Err(Error::Cancelled),
            other => return Err(Error::UnexpectedMessage(format!("{other:?}"))),
        };

        let files_total = manifest.files.len() as u64;
        let bytes_total = manifest.total_bytes;
        let _ = progress.send(ProgressEvent {
            kind: ProgressKind::Manifest,
            relative_path: String::new(),
            bytes_done: 0,
            bytes_total,
            files_done: 0,
            files_total,
            message: format!("Réception de {files_total} fichier(s)"),
        });

        receive_files(
            &mut reader,
            &mut writer,
            &dest_dir,
            files_total,
            bytes_total,
            self.cancel.clone(),
            progress,
        )
        .await
    }
}

impl Default for TransferSession {
    fn default() -> Self {
        Self::new()
    }
}

fn hostname() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "P2Puick".into())
}

async fn bind_listener(port: u16) -> std::io::Result<TcpListener> {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    TcpListener::bind(addr).await
}

fn map_bind_error(err: std::io::Error, port: u16) -> Error {
    if err.kind() == std::io::ErrorKind::AddrInUse {
        Error::Other(format!(
            "Le port {port} est déjà utilisé. Ferme l’autre session P2Puick (ou quitte l’app) puis réessaie."
        ))
    } else {
        Error::from_io(err)
    }
}

fn tune_tcp(stream: &TcpStream) {
    let _ = stream.set_nodelay(true);
    let sock = socket2::SockRef::from(stream);
    let _ = sock.set_keepalive(true);
}

struct ProgressThrottle {
    last: Instant,
    pending: Option<ProgressEvent>,
}

impl ProgressThrottle {
    fn new() -> Self {
        Self {
            last: Instant::now()
                .checked_sub(PROGRESS_MIN_INTERVAL)
                .unwrap_or_else(Instant::now),
            pending: None,
        }
    }

    fn send_now(&mut self, tx: &mpsc::UnboundedSender<ProgressEvent>, ev: ProgressEvent) {
        self.pending = None;
        let _ = tx.send(ev);
        self.last = Instant::now();
    }

    fn send_throttled(&mut self, tx: &mpsc::UnboundedSender<ProgressEvent>, ev: ProgressEvent) {
        let now = Instant::now();
        if now.duration_since(self.last) >= PROGRESS_MIN_INTERVAL {
            self.pending = None;
            let _ = tx.send(ev);
            self.last = now;
        } else {
            self.pending = Some(ev);
        }
    }

    fn flush(&mut self, tx: &mpsc::UnboundedSender<ProgressEvent>) {
        if let Some(ev) = self.pending.take() {
            let _ = tx.send(ev);
            self.last = Instant::now();
        }
    }
}

/// Accept one connection while honouring cancel (poll every 250ms so the listener can drop).
async fn accept_one(
    listener: &TcpListener,
    cancel: &AtomicBool,
) -> Result<(TcpStream, SocketAddr)> {
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(Error::Cancelled);
        }
        match tokio::time::timeout(Duration::from_millis(250), listener.accept()).await {
            Ok(Ok(pair)) => return Ok(pair),
            Ok(Err(e)) => return Err(Error::from_io(e)),
            Err(_elapsed) => continue,
        }
    }
}

/// Map relative path → absolute source root file path.
type RootMap = Vec<(String, PathBuf)>;

fn manifest_from_retry_entries(entries: Vec<FailedEntry>) -> (Manifest, RootMap) {
    let mut files = Vec::new();
    let mut roots = Vec::new();
    let mut total_bytes = 0u64;
    for entry in entries {
        let abs = PathBuf::from(&entry.absolute_path);
        let size = if entry.size > 0 {
            entry.size
        } else {
            std::fs::metadata(&abs).map(|m| m.len()).unwrap_or(0)
        };
        total_bytes += size;
        let rel = normalize_relative_path(&entry.relative_path);
        files.push(FileEntry {
            relative_path: rel.clone(),
            size,
            hash: String::new(),
        });
        roots.push((rel, abs));
    }
    (
        Manifest {
            files,
            total_bytes,
        },
        roots,
    )
}

/// Fast walk: collect paths + sizes only (no blake3). Hash happens during send.
async fn scan_inventory(
    paths: &[PathBuf],
    progress: Option<&mpsc::UnboundedSender<ProgressEvent>>,
    cancel: &AtomicBool,
    excludes: &[String],
) -> Result<(Manifest, RootMap)> {
    let mut files = Vec::new();
    let mut roots = Vec::new();
    let mut total_bytes = 0u64;
    let mut indexed = 0u64;
    let mut throttle = ProgressThrottle::new();

    let emit_now = |throttle: &mut ProgressThrottle, message: String, files_done: u64| {
        if let Some(tx) = progress {
            throttle.send_now(
                tx,
                ProgressEvent {
                    kind: ProgressKind::Preparing,
                    relative_path: String::new(),
                    bytes_done: 0,
                    bytes_total: 0,
                    files_done,
                    files_total: 0,
                    message,
                },
            );
        }
    };
    let emit = |throttle: &mut ProgressThrottle, message: String, files_done: u64| {
        if let Some(tx) = progress {
            throttle.send_throttled(
                tx,
                ProgressEvent {
                    kind: ProgressKind::Preparing,
                    relative_path: String::new(),
                    bytes_done: 0,
                    bytes_total: 0,
                    files_done,
                    files_total: 0,
                    message,
                },
            );
        }
    };

    emit_now(
        &mut throttle,
        format!(
            "Inventaire rapide ({} exclusion(s))…",
            excludes.len()
        ),
        0,
    );

    for path in paths {
        if cancel.load(Ordering::SeqCst) {
            return Err(Error::Cancelled);
        }
        let path = fs::canonicalize(path).await.unwrap_or_else(|_| path.clone());
        let meta = fs::metadata(&path).await.map_err(Error::from_io)?;
        if meta.is_file() {
            let name = path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "file".into());
            let size = meta.len();
            total_bytes += size;
            indexed += 1;
            emit(
                &mut throttle,
                format!("Inventaire : {name}"),
                indexed,
            );
            files.push(FileEntry {
                relative_path: name.clone(),
                size,
                hash: String::new(),
            });
            roots.push((name, path));
        } else if meta.is_dir() {
            if let Some(root_name) = path.file_name().and_then(|n| n.to_str()) {
                if is_excluded_dir_name(root_name, excludes) {
                    emit_now(
                        &mut throttle,
                        format!("Dossier exclu (racine) : {root_name}"),
                        indexed,
                    );
                    continue;
                }
            }

            let base = path.clone();
            let walker = WalkDir::new(&path).into_iter().filter_entry(|e| {
                if e.depth() == 0 {
                    return true;
                }
                if e.file_type().is_dir() {
                    let name = e.file_name().to_string_lossy();
                    !is_excluded_dir_name(&name, excludes)
                } else {
                    true
                }
            });

            for entry in walker.filter_map(|e| e.ok()) {
                if cancel.load(Ordering::SeqCst) {
                    return Err(Error::Cancelled);
                }
                if !entry.file_type().is_file() {
                    continue;
                }
                let abs = entry.path().to_path_buf();
                // Never fall back to an absolute path — receivers reject those as unsafe.
                let Some(rel_os) = abs.strip_prefix(&base).ok() else {
                    continue;
                };
                let rel = normalize_relative_path(&rel_os.to_string_lossy());
                if rel.is_empty() {
                    continue;
                }
                if rel
                    .split('/')
                    .any(|part| is_excluded_dir_name(part, excludes))
                {
                    continue;
                }
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                total_bytes += size;
                indexed += 1;
                emit(
                    &mut throttle,
                    format!("Inventaire : {rel}"),
                    indexed,
                );
                files.push(FileEntry {
                    relative_path: rel.clone(),
                    size,
                    hash: String::new(),
                });
                roots.push((rel, abs));
            }
        }
    }

    if let Some(tx) = progress {
        throttle.flush(tx);
    }
    emit_now(
        &mut throttle,
        format!("Inventaire prêt ({indexed} fichier(s))."),
        indexed,
    );

    Ok((
        Manifest {
            files,
            total_bytes,
        },
        roots,
    ))
}

#[derive(Debug, Default)]
struct SendReport {
    sent: Vec<String>,
    failed: Vec<FailedEntry>,
    fatal: Option<Error>,
}

fn persist_send_report(queue_path: Option<&Path>, report: &SendReport) {
    let Some(path) = queue_path else {
        return;
    };
    if !report.sent.is_empty() {
        let _ = retry_queue::mark_sent(path, &report.sent);
    }
    if !report.failed.is_empty() {
        let _ = retry_queue::record_failures(path, report.failed.clone());
    }
}

fn remaining_failures(
    roots: &[(String, PathBuf)],
    files: &[FileEntry],
    from_index: usize,
    reason: &str,
) -> Vec<FailedEntry> {
    roots
        .iter()
        .skip(from_index)
        .map(|(rel, abs)| {
            let size = files
                .iter()
                .find(|f| &f.relative_path == rel)
                .map(|f| f.size)
                .unwrap_or(0);
            retry_queue::failed_entry(rel.clone(), abs.clone(), size, reason)
        })
        .collect()
}

async fn send_files<R, W>(
    writer: Arc<Mutex<W>>,
    reader: &mut R,
    roots: &RootMap,
    files: &[FileEntry],
    cancel: Arc<AtomicBool>,
    progress: mpsc::UnboundedSender<ProgressEvent>,
    bytes_total: u64,
) -> SendReport
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let files_total = files.len() as u64;
    let mut bytes_done = 0u64;
    let mut files_done = 0u64;
    let mut throttle = ProgressThrottle::new();
    let mut report = SendReport::default();

    for (idx, (rel, abs)) in roots.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            let mut w = writer.lock().await;
            let _ = write_message(&mut *w, &Message::Cancel).await;
            report.failed.extend(remaining_failures(
                roots,
                files,
                idx,
                "transfert annulé — fichier non envoyé",
            ));
            report.fatal = Some(Error::Cancelled);
            return report;
        }

        let entry = match files.iter().find(|f| &f.relative_path == rel).cloned() {
            Some(e) => e,
            None => {
                report.failed.push(retry_queue::failed_entry(
                    rel.clone(),
                    abs.clone(),
                    0,
                    "entrée manifeste manquante",
                ));
                continue;
            }
        };

        throttle.send_now(
            &progress,
            ProgressEvent {
                kind: ProgressKind::FileStart,
                relative_path: rel.clone(),
                bytes_done,
                bytes_total,
                files_done,
                files_total,
                message: format!("Envoi de {rel}"),
            },
        );

        let send_one = async {
            {
                let mut w = writer.lock().await;
                write_message(
                    &mut *w,
                    &Message::FileStart(FileStartPayload {
                        relative_path: entry.relative_path.clone(),
                        size: entry.size,
                        hash: String::new(),
                    }),
                )
                .await
                .map_err(|e| match e {
                    Error::Io(io) => Error::from_io(io),
                    other => other,
                })?;
            }

            let mut file = File::open(abs).await.map_err(Error::from_io)?;
            let mut hasher = Hasher::new();
            let mut buf = vec![0u8; CHUNK_SIZE];
            let mut sent_for_file = 0u64;
            loop {
                if cancel.load(Ordering::SeqCst) {
                    let mut w = writer.lock().await;
                    let _ = write_message(&mut *w, &Message::Cancel).await;
                    return Err(Error::Cancelled);
                }
                let n = file.read(&mut buf).await.map_err(Error::from_io)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
                {
                    let mut w = writer.lock().await;
                    write_message(&mut *w, &Message::FileChunk(buf[..n].to_vec()))
                        .await
                        .map_err(|e| match e {
                            Error::Io(io) => Error::from_io(io),
                            other => other,
                        })?;
                }
                sent_for_file += n as u64;
                bytes_done += n as u64;
                throttle.send_throttled(
                    &progress,
                    ProgressEvent {
                        kind: ProgressKind::FileProgress,
                        relative_path: rel.clone(),
                        bytes_done,
                        bytes_total,
                        files_done,
                        files_total,
                        message: format!("{sent_for_file}/{}", entry.size),
                    },
                );
            }

            let hash = hasher.finalize().to_hex().to_string();
            {
                let mut w = writer.lock().await;
                write_message(
                    &mut *w,
                    &Message::FileEnd(FileEndPayload {
                        relative_path: entry.relative_path.clone(),
                        hash,
                    }),
                )
                .await
                .map_err(|e| match e {
                    Error::Io(io) => Error::from_io(io),
                    other => other,
                })?;
            }

            match read_message(reader).await.map_err(|e| match e {
                Error::Io(io) => Error::from_io(io),
                other => other,
            })? {
                Message::Ack(ack) if ack.ok => Ok(()),
                Message::Ack(ack) => Err(Error::Other(format!(
                    "receiver rejected {}: {}",
                    ack.relative_path, ack.message
                ))),
                Message::Cancel => Err(Error::Cancelled),
                other => Err(Error::UnexpectedMessage(format!("{other:?}"))),
            }
        };

        match send_one.await {
            Ok(()) => {
                report.sent.push(abs.to_string_lossy().into_owned());
                files_done += 1;
                throttle.send_now(
                    &progress,
                    ProgressEvent {
                        kind: ProgressKind::FileDone,
                        relative_path: rel.clone(),
                        bytes_done,
                        bytes_total,
                        files_done,
                        files_total,
                        message: format!("OK {rel}"),
                    },
                );
            }
            Err(Error::Cancelled) => {
                report.failed.extend(remaining_failures(
                    roots,
                    files,
                    idx,
                    "transfert annulé — fichier non envoyé",
                ));
                report.fatal = Some(Error::Cancelled);
                return report;
            }
            Err(e) => {
                let reason = e.to_string();
                let is_reject = matches!(&e, Error::Other(msg) if msg.contains("receiver rejected"));
                report.failed.push(retry_queue::failed_entry(
                    rel.clone(),
                    abs.clone(),
                    entry.size,
                    reason.clone(),
                ));
                if is_reject {
                    // Per-file reject: keep going so the peer can receive the rest.
                    throttle.send_now(
                        &progress,
                        ProgressEvent {
                            kind: ProgressKind::FileFailed,
                            relative_path: rel.clone(),
                            bytes_done,
                            bytes_total,
                            files_done,
                            files_total,
                            message: format!("Échec {rel} (en file de renvoi) : {reason}"),
                        },
                    );
                    continue;
                }
                // Connection / IO fatal: queue current + remaining.
                report.failed.extend(remaining_failures(
                    roots,
                    files,
                    idx + 1,
                    "non envoyé (transfert interrompu)",
                ));
                report.fatal = Some(e);
                return report;
            }
        }
    }

    throttle.flush(&progress);
    if report.failed.is_empty() {
        let _ = progress.send(ProgressEvent {
            kind: ProgressKind::Complete,
            relative_path: String::new(),
            bytes_done: bytes_total,
            bytes_total,
            files_done: files_total,
            files_total,
            message: "Transfert terminé".into(),
        });
    }
    report
}

async fn receive_files<R, W>(
    reader: &mut R,
    writer: &mut W,
    dest_dir: &Path,
    files_total: u64,
    bytes_total: u64,
    cancel: Arc<AtomicBool>,
    progress: mpsc::UnboundedSender<ProgressEvent>,
) -> Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    fs::create_dir_all(dest_dir).await.map_err(Error::from_io)?;
    let mut files_done = 0u64;
    let mut bytes_done = 0u64;
    let mut throttle = ProgressThrottle::new();

    loop {
        if cancel.load(Ordering::SeqCst) {
            let _ = write_message(writer, &Message::Cancel).await;
            return Err(Error::Cancelled);
        }

        let msg = match read_message(reader).await {
            Ok(m) => m,
            Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(Error::Io(e)) => return Err(Error::from_io(e)),
            Err(e) => return Err(e),
        };

        match msg {
            Message::FileStart(start) => {
                throttle.send_now(
                    &progress,
                    ProgressEvent {
                        kind: ProgressKind::FileStart,
                        relative_path: start.relative_path.clone(),
                        bytes_done,
                        bytes_total,
                        files_done,
                        files_total,
                        message: format!("Réception de {}", start.relative_path),
                    },
                );

                let dest_path = match safe_join(dest_dir, &start.relative_path) {
                    Ok(p) => p,
                    Err(e) => {
                        let reason = e.to_string();
                        let _ = drain_until_file_end(reader).await;
                        let _ = write_message(
                            writer,
                            &Message::Ack(AckPayload {
                                relative_path: start.relative_path.clone(),
                                ok: false,
                                message: reason.clone(),
                            }),
                        )
                        .await;
                        throttle.send_now(
                            &progress,
                            ProgressEvent {
                                kind: ProgressKind::FileFailed,
                                relative_path: start.relative_path.clone(),
                                bytes_done,
                                bytes_total,
                                files_done,
                                files_total,
                                message: format!(
                                    "Échec {}: {reason} (hôte peut renvoyer plus tard)",
                                    start.relative_path
                                ),
                            },
                        );
                        files_done += 1;
                        if files_done >= files_total {
                            break;
                        }
                        continue;
                    }
                };
                if let Some(parent) = dest_path.parent() {
                    if let Err(e) = fs::create_dir_all(parent).await {
                        let reason = Error::from_io(e).to_string();
                        let _ = drain_until_file_end(reader).await;
                        let _ = write_message(
                            writer,
                            &Message::Ack(AckPayload {
                                relative_path: start.relative_path.clone(),
                                ok: false,
                                message: reason.clone(),
                            }),
                        )
                        .await;
                        throttle.send_now(
                            &progress,
                            ProgressEvent {
                                kind: ProgressKind::FileFailed,
                                relative_path: start.relative_path.clone(),
                                bytes_done,
                                bytes_total,
                                files_done,
                                files_total,
                                message: format!("Échec {}: {reason}", start.relative_path),
                            },
                        );
                        files_done += 1;
                        if files_done >= files_total {
                            break;
                        }
                        continue;
                    }
                }
                let mut out = match File::create(&dest_path).await {
                    Ok(f) => f,
                    Err(e) => {
                        let reason = Error::from_io(e).to_string();
                        let _ = drain_until_file_end(reader).await;
                        let _ = write_message(
                            writer,
                            &Message::Ack(AckPayload {
                                relative_path: start.relative_path.clone(),
                                ok: false,
                                message: reason.clone(),
                            }),
                        )
                        .await;
                        throttle.send_now(
                            &progress,
                            ProgressEvent {
                                kind: ProgressKind::FileFailed,
                                relative_path: start.relative_path.clone(),
                                bytes_done,
                                bytes_total,
                                files_done,
                                files_total,
                                message: format!("Échec {}: {reason}", start.relative_path),
                            },
                        );
                        files_done += 1;
                        if files_done >= files_total {
                            break;
                        }
                        continue;
                    }
                };
                let mut hasher = Hasher::new();
                let mut received = 0u64;
                let mut file_failed = false;

                loop {
                    match read_message(reader).await.map_err(|e| match e {
                        Error::Io(io) => Error::from_io(io),
                        other => other,
                    })? {
                        Message::FileChunk(chunk) => {
                            if let Err(e) = out.write_all(&chunk).await {
                                file_failed = true;
                                let reason = Error::from_io(e).to_string();
                                drop(out);
                                let _ = fs::remove_file(&dest_path).await;
                                let _ = drain_until_file_end(reader).await;
                                let _ = write_message(
                                    writer,
                                    &Message::Ack(AckPayload {
                                        relative_path: start.relative_path.clone(),
                                        ok: false,
                                        message: reason.clone(),
                                    }),
                                )
                                .await;
                                throttle.send_now(
                                    &progress,
                                    ProgressEvent {
                                        kind: ProgressKind::FileFailed,
                                        relative_path: start.relative_path.clone(),
                                        bytes_done,
                                        bytes_total,
                                        files_done,
                                        files_total,
                                        message: format!(
                                            "Échec {}: {reason}",
                                            start.relative_path
                                        ),
                                    },
                                );
                                break;
                            }
                            hasher.update(&chunk);
                            received += chunk.len() as u64;
                            bytes_done += chunk.len() as u64;
                            throttle.send_throttled(
                                &progress,
                                ProgressEvent {
                                    kind: ProgressKind::FileProgress,
                                    relative_path: start.relative_path.clone(),
                                    bytes_done,
                                    bytes_total,
                                    files_done,
                                    files_total,
                                    message: format!("{received}/{}", start.size),
                                },
                            );
                        }
                        Message::FileEnd(end) => {
                            if file_failed {
                                break;
                            }
                            out.flush().await.map_err(Error::from_io)?;
                            drop(out);
                            let actual = hasher.finalize().to_hex().to_string();
                            let expected = if !end.hash.is_empty() {
                                end.hash.clone()
                            } else {
                                start.hash.clone()
                            };
                            if actual != expected {
                                let _ = fs::remove_file(&dest_path).await;
                                write_message(
                                    writer,
                                    &Message::Ack(AckPayload {
                                        relative_path: start.relative_path.clone(),
                                        ok: false,
                                        message: format!(
                                            "hash mismatch expected {expected} got {actual}"
                                        ),
                                    }),
                                )
                                .await?;
                                files_done += 1;
                                throttle.send_now(
                                    &progress,
                                    ProgressEvent {
                                        kind: ProgressKind::FileFailed,
                                        relative_path: start.relative_path.clone(),
                                        bytes_done,
                                        bytes_total,
                                        files_done,
                                        files_total,
                                        message: format!(
                                            "Hash incorrect pour {} (hôte peut renvoyer)",
                                            start.relative_path
                                        ),
                                    },
                                );
                                break;
                            }
                            write_message(
                                writer,
                                &Message::Ack(AckPayload {
                                    relative_path: start.relative_path.clone(),
                                    ok: true,
                                    message: "ok".into(),
                                }),
                            )
                            .await?;
                            files_done += 1;
                            throttle.send_now(
                                &progress,
                                ProgressEvent {
                                    kind: ProgressKind::FileDone,
                                    relative_path: start.relative_path.clone(),
                                    bytes_done,
                                    bytes_total,
                                    files_done,
                                    files_total,
                                    message: format!("OK {}", start.relative_path),
                                },
                            );
                            break;
                        }
                        Message::Cancel => return Err(Error::Cancelled),
                        other => {
                            return Err(Error::UnexpectedMessage(format!("{other:?}")));
                        }
                    }
                }

                if file_failed {
                    files_done += 1;
                }

                if files_done >= files_total {
                    break;
                }
            }
            Message::Cancel => return Err(Error::Cancelled),
            Message::Error(e) => return Err(Error::Other(e.message)),
            other => {
                tracing::warn!("unexpected while receiving: {other:?}");
                if files_done >= files_total {
                    break;
                }
            }
        }
    }

    throttle.flush(&progress);
    let _ = progress.send(ProgressEvent {
        kind: ProgressKind::Complete,
        relative_path: String::new(),
        bytes_done,
        bytes_total,
        files_done,
        files_total,
        message: "Transfert terminé".into(),
    });
    Ok(())
}

/// Discard chunks until FileEnd after a per-file setup failure.
async fn drain_until_file_end<R>(reader: &mut R) -> Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
{
    loop {
        match read_message(reader).await.map_err(|e| match e {
            Error::Io(io) => Error::from_io(io),
            other => other,
        })? {
            Message::FileChunk(_) => continue,
            Message::FileEnd(_) => return Ok(()),
            Message::Cancel => return Err(Error::Cancelled),
            other => return Err(Error::UnexpectedMessage(format!("{other:?}"))),
        }
    }
}

/// Normalize wire paths to `/`-separated relative form (no drive/root prefix).
fn normalize_relative_path(relative: &str) -> String {
    let normalized = relative.replace('\\', "/");
    let mut parts = Vec::new();
    for part in normalized.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        // Skip Windows drive-like prefixes left over from absolute fallbacks ("C:", "D:").
        if part.len() == 2 && part.as_bytes()[1] == b':' {
            continue;
        }
        parts.push(part.to_string());
    }
    parts.join("/")
}

fn safe_join(base: &Path, relative: &str) -> Result<PathBuf> {
    let cleaned = normalize_relative_path(relative);
    if cleaned.is_empty() {
        return Err(Error::protocol(format!("unsafe path: {relative}")));
    }

    let mut out = PathBuf::new();
    for component in Path::new(&cleaned).components() {
        match component {
            Component::Normal(part) => {
                let s = part.to_string_lossy();
                if s.contains('\0') || s == ".." {
                    return Err(Error::protocol(format!("unsafe path: {relative}")));
                }
                out.push(part);
            }
            Component::CurDir => {}
            // ParentDir / RootDir / Prefix must never appear after normalize.
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(Error::protocol(format!("unsafe path: {relative}")));
            }
        }
    }

    if out.as_os_str().is_empty() {
        return Err(Error::protocol(format!("unsafe path: {relative}")));
    }

    // Reject any remaining `..` segment (substring check was too loose / too brittle).
    if out
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err(Error::protocol(format!("unsafe path: {relative}")));
    }

    Ok(base.join(out))
}

#[cfg(test)]
mod path_tests {
    use super::{normalize_relative_path, safe_join};
    use std::path::Path;

    #[test]
    fn accepts_nested_project_asset_path() {
        let dest = Path::new("/tmp/p2puick-dest");
        let joined = safe_join(
            dest,
            "golden-flute-classic/assets/images/logos/region_nyon.png",
        )
        .unwrap();
        assert_eq!(
            joined,
            dest.join("golden-flute-classic/assets/images/logos/region_nyon.png")
        );
    }

    #[test]
    fn accepts_leading_slash_as_relative() {
        let dest = Path::new("/tmp/p2puick-dest");
        let joined = safe_join(dest, "/assets/images/logo.png").unwrap();
        assert_eq!(joined, dest.join("assets/images/logo.png"));
    }

    #[test]
    fn accepts_windows_separators() {
        let dest = Path::new("/tmp/p2puick-dest");
        let joined = safe_join(dest, r"assets\images\logo.png").unwrap();
        assert_eq!(joined, dest.join("assets/images/logo.png"));
    }

    #[test]
    fn rejects_parent_dir_segments() {
        let dest = Path::new("/tmp/p2puick-dest");
        assert!(safe_join(dest, "assets/../../../etc/passwd").is_err());
        assert!(safe_join(dest, "assets/foo/../../secret").is_err());
    }

    #[test]
    fn normalize_strips_drive_prefix() {
        assert_eq!(
            normalize_relative_path(r"C:\project\assets\logo.png"),
            "project/assets/logo.png"
        );
    }
}

