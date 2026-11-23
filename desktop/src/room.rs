use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use std::net::{IpAddr, SocketAddr};

pub const TUNNEL_PORT: u16 = 48900;
const PREFIX: &str = "sala1-";

pub struct Invite {
    pub secret: [u8; 16],
    pub addrs: Vec<SocketAddr>,
}

impl Invite {
    pub fn encode(&self) -> String {
        let addrs: Vec<String> = self.addrs.iter().map(|a| a.to_string()).collect();
        let text = format!("{}|{}", hex(&self.secret), addrs.join(","));
        format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(text))
    }

    pub fn decode(code: &str) -> Result<Self> {
        let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
        let body = code.strip_prefix(PREFIX).context("esse texto não é um convite da Sala de Tela")?;
        let text = String::from_utf8(URL_SAFE_NO_PAD.decode(body).context("o convite está incompleto")?)?;
        let (secret, addrs) = text.split_once('|').context("o convite está incompleto")?;
        let secret = unhex(secret).context("o convite está incompleto")?;
        let addrs: Vec<SocketAddr> = addrs.split(',').filter_map(|a| a.parse().ok()).collect();
        if addrs.is_empty() {
            bail!("o convite não tem nenhum endereço");
        }
        Ok(Self { secret, addrs })
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn unhex(text: &str) -> Option<[u8; 16]> {
    let mut out = [0u8; 16];
    if text.len() != 32 {
        return None;
    }
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

fn virtual_interface(name: &str) -> bool {
    ["lo", "docker", "br-", "veth", "virbr", "tailscale", "net-merge", "tun", "wg", "zt", "vEthernet"]
        .iter()
        .any(|p| name.starts_with(p))
}

/// Physical interfaces with an address friends can reach (home-network IPv4 or
/// global IPv6). The tunnel keeps one socket bound to each.
#[cfg(target_os = "linux")]
pub fn physical_interfaces() -> Vec<String> {
    let mut names: Vec<String> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|iface| !virtual_interface(&iface.name))
        .filter(|iface| match iface.ip() {
            IpAddr::V4(ip) => ip.is_private(),
            IpAddr::V6(ip) => (ip.segments()[0] & 0xe000) == 0x2000,
        })
        .map(|iface| iface.name)
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Temporary (privacy) IPv6 addresses rotate every few hours; invites use the
/// stable ones.
#[cfg(target_os = "linux")]
fn temporary_ipv6() -> Vec<IpAddr> {
    let text = std::fs::read_to_string("/proc/net/if_inet6").unwrap_or_default();
    text.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let flags = u8::from_str_radix(fields.get(4)?, 16).ok()?;
            if flags & 0x01 == 0 {
                return None;
            }
            let hex = fields.first()?;
            let mut bytes = [0u8; 16];
            for (i, b) in bytes.iter_mut().enumerate() {
                *b = u8::from_str_radix(hex.get(2 * i..2 * i + 2)?, 16).ok()?;
            }
            Some(IpAddr::from(bytes))
        })
        .collect()
}

#[cfg(not(target_os = "linux"))]
fn temporary_ipv6() -> Vec<IpAddr> {
    Vec::new()
}

/// Addresses friends can try: home-network IPv4 (same house) and global IPv6.
pub fn local_addresses() -> Vec<IpAddr> {
    let temporary = temporary_ipv6();
    let mut v4 = Vec::new();
    let mut v6 = Vec::new();
    for iface in if_addrs::get_if_addrs().unwrap_or_default() {
        if virtual_interface(&iface.name) {
            continue;
        }
        match iface.ip() {
            IpAddr::V4(ip) if ip.is_private() => v4.push(IpAddr::V4(ip)),
            IpAddr::V6(ip) if (ip.segments()[0] & 0xe000) == 0x2000 && !temporary.contains(&IpAddr::V6(ip)) => {
                v6.push(IpAddr::V6(ip))
            }
            _ => {}
        }
    }
    v4.sort();
    v4.dedup();
    v6.sort();
    v6.dedup();
    v4.into_iter().chain(v6).collect()
}

#[cfg(target_os = "linux")]
pub fn open_firewall() -> Result<()> {
    use std::process::Command;
    if Command::new("sh").args(["-c", "command -v ufw"]).output().map(|o| !o.status.success()).unwrap_or(true) {
        return Ok(());
    }
    // Added even while ufw is off, so the port is already open if it gets turned
    // on later. (Checking `ufw status` first silently skipped the rule.)
    let rule = TUNNEL_PORT.to_string() + "/udp";
    let status = Command::new("pkexec")
        .args(["ufw", "allow", &rule, "comment", "Sala de Tela"])
        .status()?;
    if !status.success() {
        bail!("a liberação do firewall foi cancelada");
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn open_firewall() -> Result<()> {
    Ok(())
}
