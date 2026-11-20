//! Server identity (TLS certificate), paired devices and pairing secrets.

use crate::config::config_dir;
use anyhow::{Context, Result};
use base64::Engine as _;
use dread_proto::sha256;
use rand::RngCore;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use subtle::ConstantTimeEq;
use tracing::info;

pub struct Identity {
    pub cert: CertificateDer<'static>,
    key: Vec<u8>,
    pub fingerprint: [u8; 32],
}

impl Identity {
    /// Loads the persistent certificate, creating it on first run.
    pub fn load_or_create() -> Result<Self> {
        let dir = config_dir();
        std::fs::create_dir_all(&dir)?;
        let cp = dir.join("server-cert.der");
        let kp = dir.join("server-key.der");
        let (cert, key) = match (std::fs::read(&cp), std::fs::read(&kp)) {
            (Ok(c), Ok(k)) => (c, k),
            _ => {
                let kp_ = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)?;
                let mut params = rcgen::CertificateParams::new(vec!["dread.local".to_string()])?;
                params.distinguished_name.push(rcgen::DnType::CommonName, "DREAD Server");
                params.not_before = rcgen::date_time_ymd(2024, 1, 1);
                params.not_after = rcgen::date_time_ymd(2124, 1, 1);
                let cert = params.self_signed(&kp_)?;
                let c = cert.der().to_vec();
                let k = kp_.serialize_der();
                std::fs::write(&cp, &c)?;
                write_private(&kp, &k)?;
                info!("generated new server certificate");
                (c, k)
            }
        };
        let fingerprint = sha256(&cert);
        Ok(Self { cert: CertificateDer::from(cert), key, fingerprint })
    }

    pub fn fingerprint_hex(&self) -> String {
        hex::encode(self.fingerprint)
    }

    /// Human-readable fingerprint (first 16 bytes, grouped).
    pub fn fingerprint_short(&self) -> String {
        self.fingerprint[..16].chunks(2).map(hex::encode).collect::<Vec<_>>().join(":").to_uppercase()
    }

    pub fn tls_config(&self) -> Result<Arc<rustls::ServerConfig>> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let cfg = rustls::ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])?
            .with_no_client_auth()
            .with_single_cert(vec![self.cert.clone()], PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.key.clone())))
            .context("building TLS config")?;
        Ok(Arc::new(cfg))
    }
}

fn write_private(p: &std::path::Path, data: &[u8]) -> Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(p)?;
        f.write_all(data)?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    {
        std::fs::write(p, data)?;
        Ok(())
    }
}

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PairedDevice {
    pub device_id: String,
    pub name: String,
    /// SHA-256 of the device token; the token itself is never stored.
    pub token_hash: String,
    pub paired_at: u64,
    pub last_seen: u64,
}

#[derive(Default, Serialize, Deserialize)]
pub struct DeviceStore {
    pub devices: Vec<PairedDevice>,
}

impl DeviceStore {
    fn path() -> std::path::PathBuf {
        config_dir().join("devices.json")
    }

    pub fn load() -> Self {
        std::fs::read(Self::path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let p = Self::path();
        std::fs::create_dir_all(p.parent().unwrap())?;
        write_private(&p, &serde_json::to_vec_pretty(self)?)
    }

    /// Checks a presented token in constant time.
    pub fn verify(&mut self, device_id: &str, token_b64: &str) -> bool {
        let Ok(tok) = base64::engine::general_purpose::STANDARD.decode(token_b64) else { return false };
        let h = hex::encode(sha256(&tok));
        if let Some(d) = self.devices.iter_mut().find(|d| d.device_id == device_id) {
            if bool::from(d.token_hash.as_bytes().ct_eq(h.as_bytes())) {
                d.last_seen = now_secs();
                let _ = self.save();
                return true;
            }
        }
        false
    }

    /// Registers a device and returns its new token (base64).
    pub fn pair(&mut self, device_id: &str, name: &str) -> Result<String> {
        let mut tok = [0u8; 32];
        rand::rng().fill_bytes(&mut tok);
        let token_hash = hex::encode(sha256(&tok));
        self.devices.retain(|d| d.device_id != device_id);
        self.devices.push(PairedDevice { device_id: device_id.into(), name: name.into(), token_hash, paired_at: now_secs(), last_seen: now_secs() });
        self.save()?;
        Ok(base64::engine::general_purpose::STANDARD.encode(tok))
    }

    pub fn revoke(&mut self, device_id: &str) -> Result<()> {
        self.devices.retain(|d| d.device_id != device_id);
        self.save()
    }
}

/// One-time secret embedded in the pairing QR code.
pub struct QrSecret {
    pub secret: [u8; 16],
    pub created: Instant,
}

impl QrSecret {
    pub const LIFETIME: Duration = Duration::from_secs(300);

    pub fn new() -> Self {
        let mut secret = [0u8; 16];
        rand::rng().fill_bytes(&mut secret);
        Self { secret, created: Instant::now() }
    }

    pub fn b64(&self) -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(self.secret)
    }

    pub fn expired(&self) -> bool {
        self.created.elapsed() > Self::LIFETIME
    }

    pub fn matches(&self, presented: &str) -> bool {
        let Ok(p) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(presented) else { return false };
        !self.expired() && bool::from(p.as_slice().ct_eq(&self.secret))
    }
}

pub fn random_hex(n: usize) -> String {
    let mut b = vec![0u8; n];
    rand::rng().fill_bytes(&mut b);
    hex::encode(b)
}

/// True for addresses that are not routable on the public internet.
pub fn is_private(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_private() || v4.is_loopback() || v4.is_link_local() || (o[0] == 100 && (o[1] & 0xc0) == 64)
        }
        std::net::IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_private(std::net::IpAddr::V4(v4));
            }
            let s = v6.segments();
            v6.is_loopback() || (s[0] & 0xfe00) == 0xfc00 || (s[0] & 0xffc0) == 0xfe80
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_ranges() {
        for a in ["192.168.1.5", "10.0.0.1", "172.20.1.1", "127.0.0.1", "169.254.3.3", "100.100.1.1", "fe80::1", "fd00::5", "::ffff:192.168.0.2"] {
            assert!(is_private(a.parse().unwrap()), "{a}");
        }
        for a in ["8.8.8.8", "1.1.1.1", "2001:4860::8888", "172.32.0.1"] {
            assert!(!is_private(a.parse().unwrap()), "{a}");
        }
    }
}
