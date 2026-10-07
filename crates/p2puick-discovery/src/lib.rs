//! LAN discovery for P2Puick via mDNS (`_p2puick._tcp`).

use anyhow::{anyhow, Context, Result};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::spawn_blocking;

pub const SERVICE_TYPE: &str = "_p2puick._tcp.local.";
pub const DEFAULT_PORT: u16 = 47821;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredPeer {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub pairing_hint: String,
    pub addresses: Vec<String>,
}

/// Advertise a P2Puick host session on the LAN.
/// Returns a guard that keeps the announcement alive until dropped.
pub struct Advertisement {
    daemon: ServiceDaemon,
    fullname: String,
}

impl Advertisement {
    pub fn start(instance_name: &str, port: u16, pairing_code: &str) -> Result<Self> {
        let daemon = ServiceDaemon::new().context("create mDNS daemon")?;
        let host_name = format!("{instance_name}.local.");
        let properties = [("code", pairing_code.to_string())];
        let service = ServiceInfo::new(
            SERVICE_TYPE,
            instance_name,
            &host_name,
            "",
            port,
            &properties[..],
        )
        .context("build service info")?
        .enable_addr_auto();

        let fullname = service.get_fullname().to_string();
        daemon
            .register(service)
            .context("register mDNS service")?;
        Ok(Self { daemon, fullname })
    }
}

impl Drop for Advertisement {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}

/// Browse for peers for a limited duration. Optionally filter by pairing code property.
pub async fn browse(
    timeout: Duration,
    expected_code: Option<String>,
) -> Result<Vec<DiscoveredPeer>> {
    let expected = expected_code;
    spawn_blocking(move || browse_blocking(timeout, expected))
        .await
        .map_err(|e| anyhow!("browse task join: {e}"))?
}

fn browse_blocking(
    timeout: Duration,
    expected_code: Option<String>,
) -> Result<Vec<DiscoveredPeer>> {
    let daemon = ServiceDaemon::new().context("create mDNS daemon")?;
    let receiver = daemon
        .browse(SERVICE_TYPE)
        .context("start mDNS browse")?;

    let deadline = std::time::Instant::now() + timeout;
    let mut found: HashMap<String, DiscoveredPeer> = HashMap::new();

    while std::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        let wait = remaining.min(Duration::from_millis(250));
        match receiver.recv_timeout(wait) {
            Ok(ServiceEvent::ServiceResolved(info)) => {
                let props = info.get_properties();
                let code = props
                    .get("code")
                    .map(|v| v.val_str().to_string())
                    .unwrap_or_default();
                if let Some(ref expected) = expected_code {
                    if code.trim() != expected.trim() {
                        continue;
                    }
                }
                let mut addresses: Vec<String> = info
                    .get_addresses()
                    .iter()
                    .filter_map(|ip| match ip {
                        IpAddr::V4(v4) if is_usable_lan_v4(*v4) => Some(v4.to_string()),
                        _ => None,
                    })
                    .collect();
                // Prefer private LAN over leftover link-local if both somehow appear.
                addresses.sort_by_key(|a| lan_addr_preference(a));
                if addresses.is_empty() {
                    continue;
                }
                let peer = DiscoveredPeer {
                    name: info.get_fullname().to_string(),
                    host: addresses[0].clone(),
                    port: info.get_port(),
                    pairing_hint: code,
                    addresses,
                };
                found.insert(peer.name.clone(), peer);
            }
            Ok(_) => {}
            Err(_) => {
                // Timeout or disconnected — keep looping until deadline.
            }
        }
    }

    let _ = daemon.stop_browse(SERVICE_TYPE);
    let _ = daemon.shutdown();
    Ok(found.into_values().collect())
}

/// Stream discoveries until the channel is closed or timeout elapses.
pub async fn browse_stream(
    timeout: Duration,
    expected_code: Option<String>,
    tx: mpsc::UnboundedSender<DiscoveredPeer>,
) -> Result<()> {
    let peers = browse(timeout, expected_code).await?;
    for peer in peers {
        if tx.send(peer).is_err() {
            break;
        }
    }
    Ok(())
}

pub fn format_addr(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

fn is_usable_lan_v4(ip: std::net::Ipv4Addr) -> bool {
    !ip.is_loopback() && !ip.is_unspecified() && !ip.is_multicast() && !ip.is_link_local()
}

/// Lower is better: private LAN first, then other global-ish, link-local last.
fn lan_addr_preference(addr: &str) -> u8 {
    let Ok(ip) = addr.parse::<std::net::Ipv4Addr>() else {
        return 90;
    };
    if ip.is_private() {
        0
    } else if ip.is_link_local() {
        80
    } else {
        40
    }
}

/// Non-loopback, non-link-local IPv4 addresses on local interfaces (for manual pairing).
pub fn list_lan_ipv4() -> Vec<String> {
    let Ok(ifaces) = if_addrs::get_if_addrs() else {
        return Vec::new();
    };
    let mut addrs: Vec<String> = ifaces
        .into_iter()
        .filter_map(|iface| match iface.addr {
            if_addrs::IfAddr::V4(v4) if is_usable_lan_v4(v4.ip) => Some(v4.ip.to_string()),
            _ => None,
        })
        .collect();
    addrs.sort_by_key(|a| (lan_addr_preference(a), a.clone()));
    addrs.dedup();
    addrs
}
