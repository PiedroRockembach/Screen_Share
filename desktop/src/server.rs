use anyhow::{Context, Result, bail};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

pub const SRT_PORT: u16 = 48890;
const API_PORT: u16 = 48891;

pub struct Server {
    child: Child,
}

/// The installer puts the server next to the app; during development it comes
/// from the data folder.
fn mediamtx_binary() -> PathBuf {
    let name = if cfg!(windows) { "mediamtx.exe" } else { "mediamtx" };
    let bundled = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.join(name)));
    match bundled {
        Some(path) if path.exists() => path,
        _ => crate::paths::data_dir().join("bin").join(name),
    }
}

/// Receive buffer for the video server's socket. A sender on the same PC
/// delivers a whole keyframe at once (no network to spread it), and the system
/// default (208 KB on Linux) overflowed at 20 Mbps. MediaMTX refuses to start
/// when it gets less than it asked for, so on Linux this never exceeds what
/// the kernel allows (net.core.rmem_max).
fn udp_read_buffer() -> u64 {
    const WANTED: u64 = 8 << 20;
    if cfg!(target_os = "linux") {
        std::fs::read_to_string("/proc/sys/net/core/rmem_max")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map_or(0, |max| WANTED.min(max))
    } else {
        WANTED
    }
}

impl Server {
    pub fn start(passphrase: &str) -> Result<Self> {
        let bin = mediamtx_binary();
        if !bin.exists() {
            bail!("servidor de vídeo não encontrado em {}", bin.display());
        }
        let dir = crate::paths::data_dir();
        let config = dir.join("desktop-mediamtx.yml");
        // 1128 bytes = 6 pacotes MPEG-TS; cabe em qualquer túnel com MTU 1280.
        let read_buffer = udp_read_buffer();
        std::fs::write(
            &config,
            format!(
                "logLevel: info\nrtsp: false\nrtmp: false\nhls: false\nwebrtc: false\nmoq: false\n\
                 api: true\napiAddress: 127.0.0.1:{API_PORT}\nudpMaxPayloadSize: 1128\nudpReadBufferSize: {read_buffer}\n\
                 srt: true\nsrtAddress: 127.0.0.1:{SRT_PORT}\n\
                 authInternalUsers:\n  - user: any\n    pass:\n    ips: [\"127.0.0.1\", \"::1\"]\n    permissions:\n      - action: publish\n      - action: read\n      - action: api\n\
                 paths:\n  all_others:\n    srtPublishPassphrase: {passphrase}\n    srtReadPassphrase: {passphrase}\n"
            ),
        )?;
        let log = std::fs::File::create(dir.join("desktop-mediamtx.log"))?;
        let mut command = Command::new(&bin);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let child = command
            .arg(&config)
            .current_dir(&dir)
            .stdout(Stdio::from(log.try_clone()?))
            .stderr(Stdio::from(log))
            .spawn()
            .context("iniciar o servidor de vídeo")?;
        let server = Server { child };
        for _ in 0..20 {
            if live_streams().is_ok() {
                return Ok(server);
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        bail!("o servidor de vídeo não respondeu (veja desktop-mediamtx.log)")
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn live_streams() -> Result<Vec<String>> {
    let mut stream = TcpStream::connect(("127.0.0.1", API_PORT))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.write_all(b"GET /v3/paths/list HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n")?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    let body = response.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");
    let json: serde_json::Value = serde_json::from_str(body)?;
    let mut names: Vec<String> = json["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| item["ready"].as_bool() == Some(true))
        .filter_map(|item| item["name"].as_str().map(str::to_owned))
        .collect();
    names.sort();
    Ok(names)
}

/// Where GStreamer connects: the host's own server, or a guest's local end of
/// the tunnel. SRT holds `latency_ms` to retransmit lost packets: almost nothing
/// inside one PC, more over the internet.
#[derive(Clone)]
pub struct SrtTarget {
    pub port: u16,
    pub latency_ms: u32,
    pub passphrase: String,
}

impl SrtTarget {
    fn uri(&self, streamid: &str) -> String {
        format!(
            "srt://127.0.0.1:{}?mode=caller&streamid={streamid}&latency={}&passphrase={}",
            self.port, self.latency_ms, self.passphrase
        )
    }

    pub fn publish(&self, name: &str) -> String {
        self.uri(&format!("publish:{name}"))
    }

    pub fn read(&self, name: &str) -> String {
        self.uri(&format!("read:{name}"))
    }
}
