//! Network transport: TLS listener, LAN discovery, socket statistics.

use crate::engine::{Engine, TransportKind};
use crate::security::is_private;
use crate::session::{self, ConnInfo};
use anyhow::{Context, Result};
use serde_json::json;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;
use tokio::net::{TcpListener, UdpSocket};
use tracing::{debug, info, warn};

/// IPv4 addresses of this machine on private networks (for the QR code and UI).
pub fn lan_addresses() -> Vec<IpAddr> {
    let mut v: Vec<IpAddr> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !i.is_loopback())
        .map(|i| i.ip())
        .filter(|ip| ip.is_ipv4() && is_private(*ip))
        .collect();
    v.sort();
    v.dedup();
    v
}

/// (interface, address) pairs for the Connection page.
pub fn interfaces() -> Vec<(String, IpAddr)> {
    if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !i.is_loopback() && i.ip().is_ipv4())
        .map(|i| (i.name.clone(), i.ip()))
        .collect()
}

/// (total retransmitted segments, segments sent) of a TCP socket.
#[cfg(target_os = "linux")]
pub fn tcp_retrans(fd: i32) -> Option<(u64, u64)> {
    // struct tcp_info prefix up to tcpi_segs_out (offset per linux/tcp.h)
    let mut buf = [0u8; 256];
    let mut len = buf.len() as libc::socklen_t;
    // SAFETY: buffer valid for len bytes
    let r = unsafe { libc::getsockopt(fd, libc::IPPROTO_TCP, libc::TCP_INFO, buf.as_mut_ptr() as *mut _, &mut len) };
    if r != 0 || len < 140 {
        return None;
    }
    let u32_at = |o: usize| u32::from_ne_bytes(buf[o..o + 4].try_into().unwrap()) as u64;
    // tcpi_total_retrans at 100, tcpi_segs_out at 136
    Some((u32_at(100), u32_at(136)))
}

#[cfg(not(target_os = "linux"))]
pub fn tcp_retrans(_fd: i32) -> Option<(u64, u64)> {
    None
}

fn bind(port: u16) -> Result<TcpListener> {
    let sock = socket2::Socket::new(socket2::Domain::IPV6, socket2::Type::STREAM, None);
    let std_listener = match sock {
        Ok(s) => {
            s.set_only_v6(false)?;
            s.set_reuse_address(true)?;
            s.bind(&SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port).into())?;
            s.listen(64)?;
            s.set_nonblocking(true)?;
            std::net::TcpListener::from(s)
        }
        Err(_) => {
            let l = std::net::TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port))?;
            l.set_nonblocking(true)?;
            l
        }
    };
    Ok(TcpListener::from_std(std_listener)?)
}

/// Accepts TLS connections; rebinds when the port or Wi-Fi switch changes.
pub async fn serve(e: Engine) {
    let acceptor = tokio_rustls::TlsAcceptor::from(e.tls.clone());
    loop {
        let (port, enabled) = {
            let c = e.config.read();
            (c.connection.port, c.connection.wifi_enabled)
        };
        if !enabled {
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        }
        let listener = match bind(port) {
            Ok(l) => {
                info!(port, "listening for clients");
                l
            }
            Err(err) => {
                e.set_error(Some(format!("Cannot listen on port {port}: {err:#}")));
                tokio::time::sleep(Duration::from_secs(3)).await;
                continue;
            }
        };
        loop {
            let still = {
                let c = e.config.read();
                c.connection.port == port && c.connection.wifi_enabled
            };
            if !still {
                info!("listener settings changed; rebinding");
                break;
            }
            let Ok(res) = tokio::time::timeout(Duration::from_secs(1), listener.accept()).await else { continue };
            let (tcp, addr) = match res {
                Ok(x) => x,
                Err(err) => {
                    warn!("accept: {err}");
                    continue;
                }
            };
            let allow_public = e.config.read().connection.allow_public_addresses;
            if !allow_public && !is_private(addr.ip()) {
                warn!(%addr, "rejected connection from a public address");
                continue;
            }
            let _ = tcp.set_nodelay(true);
            #[cfg(unix)]
            let fd = Some(std::os::fd::AsRawFd::as_raw_fd(&tcp));
            #[cfg(not(unix))]
            let fd = None;
            let acc = acceptor.clone();
            let e2 = e.clone();
            tokio::spawn(async move {
                match tokio::time::timeout(Duration::from_secs(10), acc.accept(tcp)).await {
                    Ok(Ok(tls)) => {
                        let peer = match addr.ip() {
                            IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(v6)).to_string(),
                            ip => ip.to_string(),
                        };
                        session::handle(e2, tls, ConnInfo { peer, transport: TransportKind::Wifi, socket_fd: fd }).await
                    }
                    Ok(Err(err)) => debug!(%addr, "TLS handshake failed: {err}"),
                    Err(_) => debug!(%addr, "TLS handshake timed out"),
                }
            });
        }
    }
}

/// Answers `DREAD?` broadcasts and registers the mDNS service.
pub async fn discovery(e: Engine) {
    let mut mdns: Option<(mdns_sd::ServiceDaemon, String)> = None;
    let sock = match UdpSocket::bind((Ipv4Addr::UNSPECIFIED, dread_proto::DISCOVERY_PORT)).await {
        Ok(s) => Some(s),
        Err(err) => {
            warn!("UDP discovery port {} unavailable: {err}", dread_proto::DISCOVERY_PORT);
            None
        }
    };
    let mut buf = [0u8; 64];
    let mut registered_port = 0u16;
    loop {
        let (enabled, port, name, id) = {
            let c = e.config.read();
            (c.connection.discovery_enabled && c.connection.wifi_enabled, c.connection.port, c.general.server_name.clone(), c.general.server_id.clone())
        };
        // keep mDNS registration in sync with settings
        if enabled && (mdns.is_none() || registered_port != port) {
            if let Some((d, full)) = mdns.take() {
                let _ = d.unregister(&full);
                let _ = d.shutdown();
            }
            match register_mdns(&e, &name, &id, port) {
                Ok(m) => {
                    registered_port = port;
                    mdns = Some(m);
                }
                Err(err) => warn!("mDNS registration failed: {err:#}"),
            }
        } else if !enabled {
            if let Some((d, full)) = mdns.take() {
                let _ = d.unregister(&full);
                let _ = d.shutdown();
            }
        }
        let Some(s) = sock.as_ref() else {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        };
        let Ok(Ok((n, from))) = tokio::time::timeout(Duration::from_secs(2), s.recv_from(&mut buf)).await else { continue };
        if !enabled || &buf[..n.min(6)] != b"DREAD?" || !is_private(from.ip()) {
            continue;
        }
        let reply = json!({
            "t": "announce",
            "id": id,
            "name": name,
            "port": port,
            "fp": e.identity.fingerprint_hex(),
            "proto": dread_proto::PROTO_VERSION,
        });
        let _ = s.send_to(serde_json::to_vec(&reply).unwrap().as_slice(), from).await;
    }
}

fn register_mdns(e: &Engine, name: &str, id: &str, port: u16) -> Result<(mdns_sd::ServiceDaemon, String)> {
    let d = mdns_sd::ServiceDaemon::new().context("starting mDNS")?;
    let host = format!("{}.local.", crate::config::hostname().replace(' ', "-"));
    let fp = e.identity.fingerprint_hex();
    let props = [("id", id), ("name", name), ("fp", fp.as_str()), ("proto", "1")];
    let ips: Vec<IpAddr> = lan_addresses();
    let info = mdns_sd::ServiceInfo::new(dread_proto::MDNS_SERVICE, name, &host, &ips[..], port, &props[..])?;
    let full = info.get_fullname().to_string();
    d.register(info)?;
    info!(%full, "mDNS service registered");
    Ok((d, full))
}
