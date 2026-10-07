use crate::error::{Error, Result};
use crate::protocol::{
    read_message, write_message, AckPayload, ErrorPayload, FileEndPayload, FileEntry,
    FileStartPayload, HelloPayload, Manifest, Message, CHUNK_SIZE,
};
use blake3::Hasher;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::fs::{self, File};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex, Semaphore};
use walkdir::WalkDir;

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
    Complete,
    Error,
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub pairing_code: String,
    pub hostname: String,
    pub concurrency: usize,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            pairing_code: String::new(),
            hostname: hostname(),
            concurrency: DEFAULT_CONCURRENCY,
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
        // Hash while waiting for the peer — avoids a long silent stall after connect.
        let prep_progress = progress.clone();
        let prep_cancel = self.cancel.clone();
        let prep_paths = source_paths.clone();
        let prep_task = tokio::spawn(async move {
            build_manifest(&prep_paths, Some(&prep_progress), &prep_cancel).await
        });

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
            message: "Handshake OK — finalisation du manifeste…".into(),
        });

        match read_message(&mut reader).await? {
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
            message: format!("Envoi de {files_total} fichier(s)…"),
        });

        {
            let mut w = writer.lock().await;
            write_message(&mut *w, &Message::Manifest(manifest.clone())).await?;
        }

        send_files(
            writer,
            &mut reader,
            &roots,
            &manifest.files,
            config.concurrency,
            self.cancel.clone(),
            progress,
            bytes_total,
        )
        .await
    }

    /// Join host, handshake as receiver, signal ready with dest dir, then receive.
    pub async fn join_and_receive(
        &self,
        addr: &str,
        config: SessionConfig,
        dest_dir: PathBuf,
        progress: mpsc::UnboundedSender<ProgressEvent>,
    ) -> Result<()> {
        let stream = TcpStream::connect(addr).await?;
        let peer_label = stream
            .peer_addr()
            .map(|a| a.ip().to_string())
            .unwrap_or_else(|_| addr.to_string());
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
            message: "En attente du manifeste (l’hôte indexe/hash les fichiers)…".into(),
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
        Error::Io(err)
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
            Ok(Err(e)) => return Err(Error::Io(e)),
            Err(_elapsed) => continue,
        }
    }
}

/// Map relative path → absolute source root file path.
type RootMap = Vec<(String, PathBuf)>;

async fn build_manifest(
    paths: &[PathBuf],
    progress: Option<&mpsc::UnboundedSender<ProgressEvent>>,
    cancel: &AtomicBool,
) -> Result<(Manifest, RootMap)> {
    let mut files = Vec::new();
    let mut roots = Vec::new();
    let mut total_bytes = 0u64;
    let mut indexed = 0u64;

    let emit = |message: String, files_done: u64| {
        if let Some(tx) = progress {
            let _ = tx.send(ProgressEvent {
                kind: ProgressKind::Preparing,
                relative_path: String::new(),
                bytes_done: 0,
                bytes_total: 0,
                files_done,
                files_total: 0,
                message,
            });
        }
    };

    emit("Indexation des fichiers…".into(), 0);

    for path in paths {
        if cancel.load(Ordering::SeqCst) {
            return Err(Error::Cancelled);
        }
        let path = fs::canonicalize(path).await.unwrap_or_else(|_| path.clone());
        let meta = fs::metadata(&path).await?;
        if meta.is_file() {
            let name = path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "file".into());
            emit(format!("Hash blake3 : {name}"), indexed);
            let hash = hash_file(&path).await?;
            let size = meta.len();
            total_bytes += size;
            indexed += 1;
            files.push(FileEntry {
                relative_path: name.clone(),
                size,
                hash,
            });
            roots.push((name, path));
        } else if meta.is_dir() {
            let base = path.clone();
            for entry in WalkDir::new(&path).into_iter().filter_map(|e| e.ok()) {
                if cancel.load(Ordering::SeqCst) {
                    return Err(Error::Cancelled);
                }
                if !entry.file_type().is_file() {
                    continue;
                }
                let abs = entry.path().to_path_buf();
                let rel = abs
                    .strip_prefix(&base)
                    .unwrap_or(entry.path())
                    .to_string_lossy()
                    .replace('\\', "/");
                emit(format!("Hash blake3 : {rel}"), indexed);
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                let hash = hash_file(&abs).await?;
                total_bytes += size;
                indexed += 1;
                files.push(FileEntry {
                    relative_path: rel.clone(),
                    size,
                    hash,
                });
                roots.push((rel, abs));
            }
        }
    }

    emit(
        format!("Manifeste prêt ({indexed} fichier(s))."),
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

async fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).await?;
    let mut hasher = Hasher::new();
    let mut buf = vec![0u8; CHUNK_SIZE];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

#[allow(clippy::too_many_arguments)]
async fn send_files<R, W>(
    writer: Arc<Mutex<W>>,
    reader: &mut R,
    roots: &RootMap,
    files: &[FileEntry],
    concurrency: usize,
    cancel: Arc<AtomicBool>,
    progress: mpsc::UnboundedSender<ProgressEvent>,
    bytes_total: u64,
) -> Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let sem = Arc::new(Semaphore::new(concurrency.max(1)));
    let files_total = files.len() as u64;
    let bytes_done = Arc::new(Mutex::new(0u64));
    let files_done = Arc::new(Mutex::new(0u64));
    let mut handles = Vec::new();

    // Sequential send over one TCP stream is safer for framing; concurrency applies to
    // hashing/prep. For v1 we stream files one-by-one but prep next file under semaphore.
    for (rel, abs) in roots {
        if cancel.load(Ordering::SeqCst) {
            let mut w = writer.lock().await;
            let _ = write_message(&mut *w, &Message::Cancel).await;
            return Err(Error::Cancelled);
        }

        let entry = files
            .iter()
            .find(|f| &f.relative_path == rel)
            .cloned()
            .ok_or_else(|| Error::Other(format!("missing manifest entry for {rel}")))?;

        let _permit = sem.acquire().await.expect("semaphore");
        let _ = progress.send(ProgressEvent {
            kind: ProgressKind::FileStart,
            relative_path: rel.clone(),
            bytes_done: *bytes_done.lock().await,
            bytes_total,
            files_done: *files_done.lock().await,
            files_total,
            message: format!("Envoi de {rel}"),
        });

        {
            let mut w = writer.lock().await;
            write_message(
                &mut *w,
                &Message::FileStart(FileStartPayload {
                    relative_path: entry.relative_path.clone(),
                    size: entry.size,
                    hash: entry.hash.clone(),
                }),
            )
            .await?;
        }

        let mut file = File::open(abs).await?;
        let mut buf = vec![0u8; CHUNK_SIZE];
        let mut sent_for_file = 0u64;
        loop {
            if cancel.load(Ordering::SeqCst) {
                let mut w = writer.lock().await;
                let _ = write_message(&mut *w, &Message::Cancel).await;
                return Err(Error::Cancelled);
            }
            let n = file.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            {
                let mut w = writer.lock().await;
                write_message(&mut *w, &Message::FileChunk(buf[..n].to_vec())).await?;
            }
            sent_for_file += n as u64;
            let mut bd = bytes_done.lock().await;
            *bd += n as u64;
            let _ = progress.send(ProgressEvent {
                kind: ProgressKind::FileProgress,
                relative_path: rel.clone(),
                bytes_done: *bd,
                bytes_total,
                files_done: *files_done.lock().await,
                files_total,
                message: format!("{sent_for_file}/{}", entry.size),
            });
        }

        {
            let mut w = writer.lock().await;
            write_message(
                &mut *w,
                &Message::FileEnd(FileEndPayload {
                    relative_path: entry.relative_path.clone(),
                    hash: entry.hash.clone(),
                }),
            )
            .await?;
        }

        // Wait ack
        match read_message(reader).await? {
            Message::Ack(ack) if ack.ok => {}
            Message::Ack(ack) => {
                return Err(Error::Other(format!(
                    "receiver rejected {}: {}",
                    ack.relative_path, ack.message
                )));
            }
            Message::Cancel => return Err(Error::Cancelled),
            other => return Err(Error::UnexpectedMessage(format!("{other:?}"))),
        }

        {
            let mut fd = files_done.lock().await;
            *fd += 1;
            let _ = progress.send(ProgressEvent {
                kind: ProgressKind::FileDone,
                relative_path: rel.clone(),
                bytes_done: *bytes_done.lock().await,
                bytes_total,
                files_done: *fd,
                files_total,
                message: format!("OK {rel}"),
            });
        }
        drop(_permit);
        handles.push(rel.clone());
    }

    let _ = progress.send(ProgressEvent {
        kind: ProgressKind::Complete,
        relative_path: String::new(),
        bytes_done: bytes_total,
        bytes_total,
        files_done: files_total,
        files_total,
        message: "Transfert terminé".into(),
    });
    Ok(())
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
    fs::create_dir_all(dest_dir).await?;
    let mut files_done = 0u64;
    let mut bytes_done = 0u64;

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
            Err(e) => return Err(e),
        };

        match msg {
            Message::FileStart(start) => {
                let _ = progress.send(ProgressEvent {
                    kind: ProgressKind::FileStart,
                    relative_path: start.relative_path.clone(),
                    bytes_done,
                    bytes_total,
                    files_done,
                    files_total,
                    message: format!("Réception de {}", start.relative_path),
                });

                let dest_path = safe_join(dest_dir, &start.relative_path)?;
                if let Some(parent) = dest_path.parent() {
                    fs::create_dir_all(parent).await?;
                }
                let mut out = File::create(&dest_path).await?;
                let mut hasher = Hasher::new();
                let mut received = 0u64;

                loop {
                    match read_message(reader).await? {
                        Message::FileChunk(chunk) => {
                            out.write_all(&chunk).await?;
                            hasher.update(&chunk);
                            received += chunk.len() as u64;
                            bytes_done += chunk.len() as u64;
                            let _ = progress.send(ProgressEvent {
                                kind: ProgressKind::FileProgress,
                                relative_path: start.relative_path.clone(),
                                bytes_done,
                                bytes_total,
                                files_done,
                                files_total,
                                message: format!("{received}/{}", start.size),
                            });
                        }
                        Message::FileEnd(end) => {
                            out.flush().await?;
                            drop(out);
                            let actual = hasher.finalize().to_hex().to_string();
                            if actual != end.hash && actual != start.hash {
                                let _ = fs::remove_file(&dest_path).await;
                                write_message(
                                    writer,
                                    &Message::Ack(AckPayload {
                                        relative_path: start.relative_path.clone(),
                                        ok: false,
                                        message: format!(
                                            "hash mismatch expected {} got {actual}",
                                            start.hash
                                        ),
                                    }),
                                )
                                .await?;
                                return Err(Error::HashMismatch {
                                    path: start.relative_path,
                                    expected: start.hash,
                                    actual,
                                });
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
                            let _ = progress.send(ProgressEvent {
                                kind: ProgressKind::FileDone,
                                relative_path: start.relative_path.clone(),
                                bytes_done,
                                bytes_total,
                                files_done,
                                files_total,
                                message: format!("OK {}", start.relative_path),
                            });
                            break;
                        }
                        Message::Cancel => return Err(Error::Cancelled),
                        other => {
                            return Err(Error::UnexpectedMessage(format!("{other:?}")));
                        }
                    }
                }

                if files_done >= files_total {
                    break;
                }
            }
            Message::Cancel => return Err(Error::Cancelled),
            Message::Error(e) => return Err(Error::Other(e.message)),
            other => {
                // Ignore trailing noise
                tracing::warn!("unexpected while receiving: {other:?}");
                if files_done >= files_total {
                    break;
                }
            }
        }
    }

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

fn safe_join(base: &Path, relative: &str) -> Result<PathBuf> {
    let rel = Path::new(relative);
    if rel.is_absolute() || relative.contains("..") {
        return Err(Error::protocol(format!("unsafe path: {relative}")));
    }
    Ok(base.join(rel))
}

