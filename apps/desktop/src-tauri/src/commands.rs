use crate::state::{AppState, Role};
use p2puick_core::{generate_pairing_code, ProgressEvent, SessionConfig, TransferSession};
use p2puick_discovery::{self, Advertisement, DiscoveredPeer};
use serde::Serialize;
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostStarted {
    pub pairing_code: String,
    pub port: u16,
    pub addresses: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LanInfo {
    pub addresses: Vec<String>,
    pub port: u16,
}

fn local_hostname() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "P2Puick".into())
}

#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo {
        name: "P2Puick".into(),
        version: env!("CARGO_PKG_VERSION").into(),
    }
}

#[tauri::command]
pub fn generate_code() -> String {
    generate_pairing_code()
}

#[tauri::command]
pub async fn start_host(
    app: AppHandle,
    state: State<'_, AppState>,
    pairing_code: Option<String>,
    port: Option<u16>,
) -> Result<HostStarted, String> {
    let code = pairing_code
        .filter(|c| !c.trim().is_empty())
        .unwrap_or_else(generate_pairing_code);
    let port = port.unwrap_or(p2puick_discovery::DEFAULT_PORT);

    {
        let mut inner = state.inner.lock().map_err(|e| e.to_string())?;
        inner.advertisement = None;
        inner.transfer = None;
        inner.pairing_code = Some(code.clone());
        inner.role = Some(Role::Host);
        inner.listen_port = port;
        inner.host_task_running = false;
        inner.source_paths.clear();
        inner.dest_dir = None;
        inner.peer_addr = None;

        let instance = format!("p2puick-{}", &code);
        let ad = Advertisement::start(&instance, port, &code).map_err(|e| e.to_string())?;
        inner.advertisement = Some(ad);
    }

    let _ = app.emit(
        "session-status",
        serde_json::json!({ "status": "hosting", "pairingCode": code, "port": port }),
    );

    Ok(HostStarted {
        pairing_code: code,
        port,
        addresses: p2puick_discovery::list_lan_ipv4(),
    })
}

#[tauri::command]
pub fn lan_info(port: Option<u16>) -> LanInfo {
    LanInfo {
        addresses: p2puick_discovery::list_lan_ipv4(),
        port: port.unwrap_or(p2puick_discovery::DEFAULT_PORT),
    }
}

#[tauri::command]
pub fn stop_host(state: State<'_, AppState>) -> Result<(), String> {
    let mut inner = state.inner.lock().map_err(|e| e.to_string())?;
    if let Some(session) = inner.transfer.take() {
        session.cancel();
    }
    inner.advertisement = None;
    inner.pairing_code = None;
    inner.role = None;
    inner.host_task_running = false;
    Ok(())
}

#[tauri::command]
pub async fn discover_peers(
    pairing_code: Option<String>,
    timeout_ms: Option<u64>,
) -> Result<Vec<DiscoveredPeer>, String> {
    let timeout = Duration::from_millis(timeout_ms.unwrap_or(2500));
    p2puick_discovery::browse(timeout, pairing_code)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn join_session(
    app: AppHandle,
    state: State<'_, AppState>,
    pairing_code: String,
    host: Option<String>,
    port: Option<u16>,
) -> Result<String, String> {
    let port = port.unwrap_or(p2puick_discovery::DEFAULT_PORT);
    let addr = if let Some(host) = host.filter(|h| !h.trim().is_empty()) {
        p2puick_discovery::format_addr(host.trim(), port)
    } else {
        let peers = p2puick_discovery::browse(Duration::from_secs(3), Some(pairing_code.clone()))
            .await
            .map_err(|e| e.to_string())?;
        let peer = peers.into_iter().next().ok_or_else(|| {
            "Aucun hôte trouvé sur le réseau local. Saisis l’IP manuellement.".to_string()
        })?;
        p2puick_discovery::format_addr(&peer.host, peer.port)
    };

    {
        let mut inner = state.inner.lock().map_err(|e| e.to_string())?;
        inner.pairing_code = Some(pairing_code.clone());
        inner.role = Some(Role::Joiner);
        inner.peer_addr = Some(addr.clone());
        inner.listen_port = port;
        inner.advertisement = None;
    }

    let _ = app.emit(
        "session-status",
        serde_json::json!({ "status": "joined", "pairingCode": pairing_code, "addr": addr }),
    );
    Ok(addr)
}

#[tauri::command]
pub async fn pick_files(app: AppHandle) -> Result<Vec<String>, String> {
    let files = app
        .dialog()
        .file()
        .set_title("Choisir des fichiers à envoyer")
        .blocking_pick_files();
    Ok(files
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| p.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned())
        .collect())
}

#[tauri::command]
pub async fn pick_folder(app: AppHandle, title: Option<String>) -> Result<Option<String>, String> {
    let folder = app
        .dialog()
        .file()
        .set_title(title.unwrap_or_else(|| "Choisir un dossier".into()))
        .blocking_pick_folder();
    Ok(folder
        .and_then(|p| p.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned()))
}

#[tauri::command]
pub async fn begin_send(
    app: AppHandle,
    state: State<'_, AppState>,
    paths: Vec<String>,
) -> Result<(), String> {
    if paths.is_empty() {
        return Err("Aucun fichier sélectionné".into());
    }

    let (code, port, session) = {
        let mut inner = state.inner.lock().map_err(|e| e.to_string())?;
        if inner.role != Some(Role::Host) {
            return Err("Démarre d’abord une session hôte".into());
        }
        if inner.host_task_running {
            return Err("Transfert déjà en cours".into());
        }
        let code = inner
            .pairing_code
            .clone()
            .ok_or_else(|| "Code de pairing manquant".to_string())?;
        let port = inner.listen_port;
        inner.source_paths = paths.iter().map(PathBuf::from).collect();
        inner.host_task_running = true;
        let session = TransferSession::new();
        inner.transfer = Some(session.clone());
        (code, port, session)
    };

    let source_paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ProgressEvent>();
    let progress_app = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(ev) = rx.recv().await {
            let _ = progress_app.emit("transfer-progress", ev);
        }
    });

    let result = session
        .host_and_send(
            port,
            SessionConfig {
                pairing_code: code,
                hostname: local_hostname(),
                concurrency: p2puick_core::DEFAULT_CONCURRENCY,
            },
            source_paths,
            tx,
        )
        .await;

    if let Ok(mut inner) = app.state::<AppState>().inner.lock() {
        inner.host_task_running = false;
    }

    match result {
        Ok(()) => Ok(()),
        Err(e) => {
            let msg = e.to_string();
            let _ = app.emit(
                "transfer-progress",
                ProgressEvent {
                    kind: p2puick_core::ProgressKind::Error,
                    relative_path: String::new(),
                    bytes_done: 0,
                    bytes_total: 0,
                    files_done: 0,
                    files_total: 0,
                    message: msg.clone(),
                },
            );
            Err(msg)
        }
    }
}

#[tauri::command]
pub async fn begin_receive(
    app: AppHandle,
    state: State<'_, AppState>,
    dest_dir: String,
) -> Result<(), String> {
    let (code, addr, session) = {
        let mut inner = state.inner.lock().map_err(|e| e.to_string())?;
        if inner.role != Some(Role::Joiner) {
            return Err("Rejoins d’abord une session".into());
        }
        let code = inner
            .pairing_code
            .clone()
            .ok_or_else(|| "Code de pairing manquant".to_string())?;
        let addr = inner
            .peer_addr
            .clone()
            .ok_or_else(|| "Adresse peer manquante".to_string())?;
        inner.dest_dir = Some(PathBuf::from(&dest_dir));
        let session = TransferSession::new();
        inner.transfer = Some(session.clone());
        (code, addr, session)
    };

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ProgressEvent>();
    let progress_app = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(ev) = rx.recv().await {
            let _ = progress_app.emit("transfer-progress", ev);
        }
    });

    let result = session
        .join_and_receive(
            &addr,
            SessionConfig {
                pairing_code: code,
                hostname: local_hostname(),
                concurrency: p2puick_core::DEFAULT_CONCURRENCY,
            },
            PathBuf::from(dest_dir),
            tx,
        )
        .await;

    match result {
        Ok(()) => Ok(()),
        Err(e) => {
            let msg = e.to_string();
            let _ = app.emit(
                "transfer-progress",
                ProgressEvent {
                    kind: p2puick_core::ProgressKind::Error,
                    relative_path: String::new(),
                    bytes_done: 0,
                    bytes_total: 0,
                    files_done: 0,
                    files_total: 0,
                    message: msg.clone(),
                },
            );
            Err(msg)
        }
    }
}

#[tauri::command]
pub fn cancel_transfer(state: State<'_, AppState>) -> Result<(), String> {
    let inner = state.inner.lock().map_err(|e| e.to_string())?;
    if let Some(session) = inner.transfer.as_ref() {
        session.cancel();
    }
    Ok(())
}
