//! Our own UDP tunnel. Only the host opens a port. Guests prove they hold the
//! room secret (HMAC) before anything is forwarded; each SRT connection of a
//! guest becomes a "flow" relayed to the host's local video server. Media stays
//! encrypted end to end by SRT itself.

use anyhow::{Result, bail};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use socket2::{Domain, Protocol, Socket, Type};
use std::collections::HashMap;
use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MAGIC: [u8; 2] = *b"St";
const HELLO: u8 = 1;
const WELCOME: u8 = 2;
const DATA: u8 = 3;
const PING: u8 = 4;
const PONG: u8 = 5;
const LIST_REQUEST: u8 = 6;
const LIST: u8 = 7;
const HEADER: usize = 11;
const IDLE: Duration = Duration::from_secs(30);

fn header(kind: u8, session: u64) -> Vec<u8> {
    let mut packet = Vec::with_capacity(1500);
    packet.extend_from_slice(&MAGIC);
    packet.push(kind);
    packet.extend_from_slice(&session.to_be_bytes());
    packet
}

fn parse(packet: &[u8]) -> Option<(u8, u64, &[u8])> {
    if packet.len() < HEADER || packet[..2] != MAGIC {
        return None;
    }
    Some((packet[2], u64::from_be_bytes(packet[3..11].try_into().ok()?), &packet[HEADER..]))
}

fn sign(secret: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(secret).expect("any key length");
    for part in parts {
        mac.update(part);
    }
    mac.finalize().into_bytes().into()
}

fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).expect("system randomness");
    bytes
}

/// A dual-stack UDP socket (IPv6 and IPv4 on the same port).
fn dual_socket(port: u16) -> Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_only_v6(false)?;
    let _ = socket.set_recv_buffer_size(4 << 20);
    let _ = socket.set_send_buffer_size(4 << 20);
    socket.bind(&SocketAddr::from((Ipv6Addr::UNSPECIFIED, port)).into())?;
    Ok(socket.into())
}

fn as_v6(addr: SocketAddr) -> SocketAddr {
    match addr {
        SocketAddr::V4(v4) => SocketAddr::V6(SocketAddrV6::new(v4.ip().to_ipv6_mapped(), v4.port(), 0, 0)),
        v6 => v6,
    }
}

/// Where a packet arrived. Replies leave through the same interface and from
/// the same address; otherwise a VPN or link aggregator (net-merge) that
/// captures the default routes would send them out with another source.
#[derive(Clone, Copy)]
struct Arrival {
    #[cfg(target_os = "linux")]
    info: Option<nix::libc::in6_pktinfo>,
}

struct HostSocket(UdpSocket);

impl HostSocket {
    fn new(port: u16) -> Result<Self> {
        let socket = dual_socket(port)?;
        #[cfg(target_os = "linux")]
        {
            use nix::sys::socket::{setsockopt, sockopt::Ipv6RecvPacketInfo};
            setsockopt(&socket, Ipv6RecvPacketInfo, &true)?;
        }
        socket.set_read_timeout(Some(Duration::from_millis(500)))?;
        Ok(Self(socket))
    }

    #[cfg(target_os = "linux")]
    fn recv(&self, buf: &mut [u8]) -> std::io::Result<(usize, SocketAddr, Arrival)> {
        use nix::sys::socket::{ControlMessageOwned, MsgFlags, SockaddrIn6, recvmsg};
        use std::os::fd::AsRawFd;
        let mut iov = [std::io::IoSliceMut::new(buf)];
        let mut cmsg = nix::cmsg_space!(nix::libc::in6_pktinfo);
        let msg = recvmsg::<SockaddrIn6>(self.0.as_raw_fd(), &mut iov, Some(&mut cmsg), MsgFlags::empty())
            .map_err(std::io::Error::from)?;
        let mut info = None;
        for c in msg.cmsgs().map_err(std::io::Error::from)? {
            if let ControlMessageOwned::Ipv6PacketInfo(i) = c {
                info = Some(i);
            }
        }
        let from = msg.address.ok_or_else(|| std::io::Error::other("no source"))?;
        Ok((msg.bytes, SocketAddr::V6(from.into()), Arrival { info }))
    }

    #[cfg(not(target_os = "linux"))]
    fn recv(&self, buf: &mut [u8]) -> std::io::Result<(usize, SocketAddr, Arrival)> {
        let (n, from) = self.0.recv_from(buf)?;
        Ok((n, from, Arrival {}))
    }

    #[cfg(target_os = "linux")]
    fn send(&self, packet: &[u8], to: SocketAddr, arrival: Arrival) {
        use nix::sys::socket::{ControlMessage, MsgFlags, SockaddrIn6, sendmsg};
        use std::os::fd::AsRawFd;
        let SocketAddr::V6(to) = as_v6(to) else { return };
        let to = SockaddrIn6::from(to);
        let iov = [std::io::IoSlice::new(packet)];
        let _ = match &arrival.info {
            Some(info) => sendmsg(self.0.as_raw_fd(), &iov, &[ControlMessage::Ipv6PacketInfo(info)], MsgFlags::empty(), Some(&to)),
            None => sendmsg(self.0.as_raw_fd(), &iov, &[], MsgFlags::empty(), Some(&to)),
        };
    }

    #[cfg(not(target_os = "linux"))]
    fn send(&self, packet: &[u8], to: SocketAddr, _arrival: Arrival) {
        let _ = self.0.send_to(packet, as_v6(to));
    }
}

struct Peer {
    addr: SocketAddr,
    arrival: Arrival,
    last_seen: Instant,
}

struct Flow {
    socket: Arc<UdpSocket>,
    last_seen: Instant,
    stop: Arc<AtomicBool>,
}

struct Session {
    peer: Arc<Mutex<Peer>>,
    flows: HashMap<u16, Flow>,
}

pub struct Host {
    stop: Arc<AtomicBool>,
}

impl Drop for Host {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Host {
    pub fn start(
        port: u16,
        secret: [u8; 16],
        srt_port: u16,
        live: impl Fn() -> Vec<String> + Send + 'static,
    ) -> Result<Self> {
        let socket = Arc::new(HostSocket::new(port)?);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_loop = stop.clone();
        std::thread::spawn(move || {
            let mut sessions: HashMap<u64, Session> = HashMap::new();
            let mut buf = vec![0u8; 65536];
            let mut last_cleanup = Instant::now();
            while !stop_loop.load(Ordering::Relaxed) {
                if last_cleanup.elapsed() > Duration::from_secs(5) {
                    last_cleanup = Instant::now();
                    sessions.retain(|_, s| s.peer.lock().unwrap().last_seen.elapsed() < IDLE);
                    for session in sessions.values_mut() {
                        session.flows.retain(|_, f| {
                            let alive = f.last_seen.elapsed() < IDLE;
                            if !alive {
                                f.stop.store(true, Ordering::Relaxed);
                            }
                            alive
                        });
                    }
                }
                let Ok((n, from, arrival)) = socket.recv(&mut buf) else { continue };
                let Some((kind, id, body)) = parse(&buf[..n]) else { continue };
                if kind == HELLO {
                    if body.len() != 48 || sign(&secret, &[&id.to_be_bytes(), &body[..16]]) != body[16..48] {
                        continue;
                    }
                    let peer = Peer { addr: from, arrival, last_seen: Instant::now() };
                    sessions.entry(id).or_insert_with(|| Session { peer: Arc::new(Mutex::new(peer)), flows: HashMap::new() });
                    let mut reply = header(WELCOME, id);
                    reply.extend_from_slice(&sign(&secret, &[b"W", &id.to_be_bytes(), &body[..16]]));
                    socket.send(&reply, from, arrival);
                    continue;
                }
                let Some(session) = sessions.get_mut(&id) else { continue };
                {
                    // Guests may change address (Wi-Fi to cable, NAT rebinding).
                    let mut peer = session.peer.lock().unwrap();
                    peer.addr = from;
                    peer.arrival = arrival;
                    peer.last_seen = Instant::now();
                }
                match kind {
                    PING => socket.send(&header(PONG, id), from, arrival),
                    LIST_REQUEST => {
                        let mut reply = header(LIST, id);
                        let mut names = live();
                        let mut json = serde_json::to_vec(&names).unwrap_or_default();
                        while json.len() > 1100 && names.pop().is_some() {
                            json = serde_json::to_vec(&names).unwrap_or_default();
                        }
                        reply.extend_from_slice(&json);
                        socket.send(&reply, from, arrival);
                    }
                    DATA if body.len() >= 2 => {
                        let flow_id = u16::from_be_bytes([body[0], body[1]]);
                        let flow = match session.flows.get_mut(&flow_id) {
                            Some(f) => f,
                            None => {
                                let Ok(flow) = open_flow(srt_port, id, flow_id, session.peer.clone(), socket.clone()) else { continue };
                                session.flows.entry(flow_id).or_insert(flow)
                            }
                        };
                        flow.last_seen = Instant::now();
                        let _ = flow.socket.send(&body[2..]);
                    }
                    _ => {}
                }
            }
            for session in sessions.values() {
                for flow in session.flows.values() {
                    flow.stop.store(true, Ordering::Relaxed);
                }
            }
        });
        Ok(Self { stop })
    }
}

fn open_flow(srt_port: u16, session: u64, flow_id: u16, peer: Arc<Mutex<Peer>>, tunnel: Arc<HostSocket>) -> Result<Flow> {
    let local = UdpSocket::bind(("127.0.0.1", 0))?;
    local.connect(("127.0.0.1", srt_port))?;
    local.set_read_timeout(Some(Duration::from_millis(500)))?;
    let local = Arc::new(local);
    let stop = Arc::new(AtomicBool::new(false));
    let (reader, stop_reader) = (local.clone(), stop.clone());
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 65536];
        let mut packet = header(DATA, session);
        packet.extend_from_slice(&flow_id.to_be_bytes());
        let prefix = packet.len();
        while !stop_reader.load(Ordering::Relaxed) {
            let Ok(n) = reader.recv(&mut buf) else { continue };
            packet.truncate(prefix);
            packet.extend_from_slice(&buf[..n]);
            let (addr, arrival) = {
                let p = peer.lock().unwrap();
                (p.addr, p.arrival)
            };
            tunnel.send(&packet, addr, arrival);
        }
    });
    Ok(Flow { socket: local, last_seen: Instant::now(), stop })
}

/// A guest's side: a local UDP port that GStreamer talks to as if the video
/// server were on this PC, relayed through the tunnel.
pub struct Guest {
    pub local_port: u16,
    pub host: SocketAddr,
    live: Arc<Mutex<Vec<String>>>,
    last_pong: Arc<Mutex<Instant>>,
    stop: Arc<AtomicBool>,
}

impl Drop for Guest {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Guest {
    /// Tries every address in the invite at once; the first that answers wins.
    pub fn connect(candidates: &[SocketAddr], secret: [u8; 16]) -> Result<Self> {
        let tunnel = Arc::new(dual_socket(0)?);
        tunnel.set_read_timeout(Some(Duration::from_millis(100)))?;
        let session = u64::from_be_bytes(random::<8>());
        let nonce = random::<16>();
        let mut hello = header(HELLO, session);
        hello.extend_from_slice(&nonce);
        hello.extend_from_slice(&sign(&secret, &[&session.to_be_bytes(), &nonce]));
        let expected = sign(&secret, &[b"W", &session.to_be_bytes(), &nonce]);

        let started = Instant::now();
        let mut buf = vec![0u8; 65536];
        let mut last_send = Instant::now() - Duration::from_secs(1);
        let host = loop {
            if started.elapsed() > Duration::from_secs(8) {
                bail!("nenhum endereço do convite respondeu");
            }
            if last_send.elapsed() > Duration::from_millis(300) {
                for addr in candidates {
                    let _ = tunnel.send_to(&hello, as_v6(*addr));
                }
                last_send = Instant::now();
            }
            if let Ok((n, from)) = tunnel.recv_from(&mut buf) {
                if let Some((WELCOME, id, body)) = parse(&buf[..n]) {
                    if id == session && body == expected {
                        break from;
                    }
                }
            }
        };

        let proxy = Arc::new(UdpSocket::bind(("127.0.0.1", 0))?);
        proxy.set_read_timeout(Some(Duration::from_millis(500)))?;
        let local_port = proxy.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let live = Arc::new(Mutex::new(Vec::new()));
        let last_pong = Arc::new(Mutex::new(Instant::now()));
        let flows: Arc<Mutex<(HashMap<SocketAddr, u16>, HashMap<u16, SocketAddr>)>> = Arc::default();

        {
            let (proxy, tunnel, stop, flows) = (proxy.clone(), tunnel.clone(), stop.clone(), flows.clone());
            std::thread::spawn(move || {
                let mut buf = vec![0u8; 65536];
                let mut next = 0u16;
                while !stop.load(Ordering::Relaxed) {
                    let Ok((n, from)) = proxy.recv_from(&mut buf) else { continue };
                    let flow = {
                        let mut f = flows.lock().unwrap();
                        match f.0.get(&from) {
                            Some(id) => *id,
                            None => {
                                next = next.wrapping_add(1);
                                f.0.insert(from, next);
                                f.1.insert(next, from);
                                next
                            }
                        }
                    };
                    let mut packet = header(DATA, session);
                    packet.extend_from_slice(&flow.to_be_bytes());
                    packet.extend_from_slice(&buf[..n]);
                    let _ = tunnel.send_to(&packet, host);
                }
            });
        }
        {
            let (proxy, tunnel, stop, live, last_pong) = (proxy.clone(), tunnel.clone(), stop.clone(), live.clone(), last_pong.clone());
            std::thread::spawn(move || {
                let mut buf = vec![0u8; 65536];
                while !stop.load(Ordering::Relaxed) {
                    let Ok((n, from)) = tunnel.recv_from(&mut buf) else { continue };
                    if from != host {
                        continue;
                    }
                    let Some((kind, id, body)) = parse(&buf[..n]) else { continue };
                    if id != session {
                        continue;
                    }
                    *last_pong.lock().unwrap() = Instant::now();
                    match kind {
                        DATA if body.len() >= 2 => {
                            let flow = u16::from_be_bytes([body[0], body[1]]);
                            if let Some(to) = flows.lock().unwrap().1.get(&flow) {
                                let _ = proxy.send_to(&body[2..], to);
                            }
                        }
                        LIST => {
                            if let Ok(names) = serde_json::from_slice(body) {
                                *live.lock().unwrap() = names;
                            }
                        }
                        _ => {}
                    }
                }
            });
        }
        {
            let (tunnel, stop) = (tunnel.clone(), stop.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let _ = tunnel.send_to(&header(PING, session), host);
                    let _ = tunnel.send_to(&header(LIST_REQUEST, session), host);
                    std::thread::sleep(Duration::from_secs(2));
                }
            });
        }
        Ok(Self { local_port, host, live, last_pong, stop })
    }

    pub fn live(&self) -> Vec<String> {
        self.live.lock().unwrap().clone()
    }

    pub fn connected(&self) -> bool {
        self.last_pong.lock().unwrap().elapsed() < Duration::from_secs(10)
    }
}
