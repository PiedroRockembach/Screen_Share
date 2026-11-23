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

/// Keyframes arrive in bursts; the system default (64 KB on Windows) drops part
/// of them before we read. Linux caps this at net.core.rmem_max.
const BUFFER: usize = 4 << 20;

/// A dual-stack UDP socket (IPv6 and IPv4 on the same port).
fn dual_socket(port: u16) -> Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_only_v6(false)?;
    let _ = socket.set_recv_buffer_size(BUFFER);
    let _ = socket.set_send_buffer_size(BUFFER);
    socket.bind(&SocketAddr::from((Ipv6Addr::UNSPECIFIED, port)).into())?;
    Ok(socket.into())
}

/// A socket on 127.0.0.1 between the tunnel and GStreamer / the video server.
fn local_socket() -> Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    let _ = socket.set_recv_buffer_size(BUFFER);
    let _ = socket.set_send_buffer_size(BUFFER);
    socket.bind(&SocketAddr::from(([127, 0, 0, 1], 0)).into())?;
    socket.set_read_timeout(Some(Duration::from_millis(500)))?;
    Ok(socket.into())
}

fn as_v6(addr: SocketAddr) -> SocketAddr {
    match addr {
        SocketAddr::V4(v4) => SocketAddr::V6(SocketAddrV6::new(v4.ip().to_ipv6_mapped(), v4.port(), 0, 0)),
        v6 => v6,
    }
}

/// Where a packet arrived. Replies leave through the same socket and from the
/// same address. On Linux each physical interface also gets its own socket
/// bound to it (SO_BINDTODEVICE): a VPN or link aggregator such as net-merge
/// routes the whole internet (`::/1`) into itself, and replies to a guest
/// coming from outside would vanish there; a bound socket bypasses those routes.
#[derive(Clone)]
struct Arrival {
    via: Arc<UdpSocket>,
    #[cfg(target_os = "linux")]
    info: Option<nix::libc::in6_pktinfo>,
}

/// One socket of the host on the room port, optionally bound to an interface.
fn host_socket(port: u16, device: Option<&str>) -> Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_only_v6(false)?;
    let _ = socket.set_recv_buffer_size(BUFFER);
    let _ = socket.set_send_buffer_size(BUFFER);
    #[cfg(target_os = "linux")]
    {
        use nix::sys::socket::{setsockopt, sockopt::Ipv6RecvPacketInfo};
        // Several sockets share the port; the kernel hands each packet to the
        // one bound to the interface it came in on, else to the unbound one.
        socket.set_reuse_address(true)?;
        if let Some(device) = device {
            socket.bind_device(Some(device.as_bytes()))?;
        }
        setsockopt(&socket, Ipv6RecvPacketInfo, &true)?;
    }
    #[cfg(not(target_os = "linux"))]
    let _ = device;
    socket.bind(&SocketAddr::from((Ipv6Addr::UNSPECIFIED, port)).into())?;
    socket.set_read_timeout(Some(Duration::from_millis(500)))?;
    Ok(socket.into())
}

#[cfg(target_os = "linux")]
fn recv(socket: &UdpSocket, buf: &mut [u8]) -> std::io::Result<(usize, SocketAddr, Option<nix::libc::in6_pktinfo>)> {
    use nix::sys::socket::{ControlMessageOwned, MsgFlags, SockaddrIn6, recvmsg};
    use std::os::fd::AsRawFd;
    let mut iov = [std::io::IoSliceMut::new(buf)];
    let mut cmsg = nix::cmsg_space!(nix::libc::in6_pktinfo);
    let msg = recvmsg::<SockaddrIn6>(socket.as_raw_fd(), &mut iov, Some(&mut cmsg), MsgFlags::empty())
        .map_err(std::io::Error::from)?;
    let mut info = None;
    for c in msg.cmsgs().map_err(std::io::Error::from)? {
        if let ControlMessageOwned::Ipv6PacketInfo(i) = c {
            info = Some(i);
        }
    }
    let from = msg.address.ok_or_else(|| std::io::Error::other("no source"))?;
    Ok((msg.bytes, SocketAddr::V6(from.into()), info))
}

#[cfg(target_os = "linux")]
fn send(arrival: &Arrival, packet: &[u8], to: SocketAddr) {
    use nix::sys::socket::{ControlMessage, MsgFlags, SockaddrIn6, sendmsg};
    use std::os::fd::AsRawFd;
    let SocketAddr::V6(to) = as_v6(to) else { return };
    let to = SockaddrIn6::from(to);
    let iov = [std::io::IoSlice::new(packet)];
    let fd = arrival.via.as_raw_fd();
    let _ = match &arrival.info {
        Some(info) => sendmsg(fd, &iov, &[ControlMessage::Ipv6PacketInfo(info)], MsgFlags::empty(), Some(&to)),
        None => sendmsg(fd, &iov, &[], MsgFlags::empty(), Some(&to)),
    };
}

#[cfg(not(target_os = "linux"))]
fn send(arrival: &Arrival, packet: &[u8], to: SocketAddr) {
    let _ = arrival.via.send_to(packet, as_v6(to));
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

type LiveFn = dyn Fn() -> Vec<String> + Send + Sync;

/// State shared by the receive thread of every host socket.
struct Shared {
    secret: [u8; 16],
    srt_port: u16,
    live: Box<LiveFn>,
    sessions: Mutex<HashMap<u64, Session>>,
    stop: Arc<AtomicBool>,
}

impl Shared {
    fn handle(&self, packet: &[u8], from: SocketAddr, arrival: Arrival) {
        let Some((kind, id, body)) = parse(packet) else { return };
        let mut sessions = self.sessions.lock().unwrap();
        if kind == HELLO {
            if body.len() != 48 || sign(&self.secret, &[&id.to_be_bytes(), &body[..16]]) != body[16..48] {
                return;
            }
            let mut reply = header(WELCOME, id);
            reply.extend_from_slice(&sign(&self.secret, &[b"W", &id.to_be_bytes(), &body[..16]]));
            send(&arrival, &reply, from);
            let peer = Peer { addr: from, arrival, last_seen: Instant::now() };
            sessions.entry(id).or_insert_with(|| Session { peer: Arc::new(Mutex::new(peer)), flows: HashMap::new() });
            return;
        }
        let Some(session) = sessions.get_mut(&id) else { return };
        {
            // Guests may change address (Wi-Fi to cable, NAT rebinding).
            let mut peer = session.peer.lock().unwrap();
            peer.addr = from;
            peer.arrival = arrival.clone();
            peer.last_seen = Instant::now();
        }
        match kind {
            PING => send(&arrival, &header(PONG, id), from),
            LIST_REQUEST => {
                // Asking the video server takes a moment: don't hold up the
                // other sockets' media meanwhile.
                drop(sessions);
                let mut reply = header(LIST, id);
                let mut names = (self.live)();
                let mut json = serde_json::to_vec(&names).unwrap_or_default();
                while json.len() > 1100 && names.pop().is_some() {
                    json = serde_json::to_vec(&names).unwrap_or_default();
                }
                reply.extend_from_slice(&json);
                send(&arrival, &reply, from);
            }
            DATA if body.len() >= 2 => {
                let flow_id = u16::from_be_bytes([body[0], body[1]]);
                let flow = match session.flows.get_mut(&flow_id) {
                    Some(f) => f,
                    None => {
                        let Ok(flow) = open_flow(self.srt_port, id, flow_id, session.peer.clone()) else { return };
                        session.flows.entry(flow_id).or_insert(flow)
                    }
                };
                flow.last_seen = Instant::now();
                let _ = flow.socket.send(&body[2..]);
            }
            _ => {}
        }
    }

    fn cleanup(&self) {
        let mut sessions = self.sessions.lock().unwrap();
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

    fn close(&self) {
        for session in self.sessions.lock().unwrap().values() {
            for flow in session.flows.values() {
                flow.stop.store(true, Ordering::Relaxed);
            }
        }
    }
}

/// Reads one host socket until the room closes (or `own_stop`, when its
/// interface goes away).
fn serve(shared: Arc<Shared>, socket: Arc<UdpSocket>, own_stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 65536];
        while !shared.stop.load(Ordering::Relaxed) && !own_stop.load(Ordering::Relaxed) {
            #[cfg(target_os = "linux")]
            let Ok((n, from, info)) = recv(&socket, &mut buf) else { continue };
            #[cfg(target_os = "linux")]
            let arrival = Arrival { via: socket.clone(), info };
            #[cfg(not(target_os = "linux"))]
            let Ok((n, from)) = socket.recv_from(&mut buf) else { continue };
            #[cfg(not(target_os = "linux"))]
            let arrival = Arrival { via: socket.clone() };
            shared.handle(&buf[..n], from, arrival);
        }
    });
}

/// Interface name -> (its index when bound, stop flag of its socket).
#[cfg(target_os = "linux")]
type Bound = HashMap<String, (u32, Arc<AtomicBool>)>;

/// Keeps one bound socket per physical interface, following Wi-Fi reconnects
/// and cables plugged in or out.
#[cfg(target_os = "linux")]
fn refresh_interfaces(shared: &Arc<Shared>, port: u16, bound: &mut Bound) {
    let index = |name: &str| nix::net::if_::if_nametoindex(name).unwrap_or(0);
    let wanted = crate::room::physical_interfaces();
    bound.retain(|name, (idx, stop)| {
        let keep = wanted.contains(name) && index(name) == *idx;
        if !keep {
            stop.store(true, Ordering::Relaxed);
        }
        keep
    });
    for name in wanted {
        if bound.contains_key(&name) {
            continue;
        }
        if let Ok(socket) = host_socket(port, Some(&name)) {
            let stop = Arc::new(AtomicBool::new(false));
            serve(shared.clone(), Arc::new(socket), stop.clone());
            bound.insert(name.clone(), (index(&name), stop));
        }
    }
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
        live: impl Fn() -> Vec<String> + Send + Sync + 'static,
    ) -> Result<Self> {
        // The shared port would let a second copy of the app bind it too;
        // a plain bind first fails if the room is already open elsewhere.
        drop(UdpSocket::bind(SocketAddr::from((Ipv6Addr::UNSPECIFIED, port)))?);
        let main = Arc::new(host_socket(port, None)?);
        let stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(Shared {
            secret,
            srt_port,
            live: Box::new(live),
            sessions: Mutex::new(HashMap::new()),
            stop: stop.clone(),
        });
        serve(shared.clone(), main, Arc::new(AtomicBool::new(false)));
        #[cfg(target_os = "linux")]
        let mut bound = Bound::new();
        #[cfg(target_os = "linux")]
        refresh_interfaces(&shared, port, &mut bound);
        std::thread::spawn(move || {
            while !shared.stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_secs(5));
                shared.cleanup();
                #[cfg(target_os = "linux")]
                refresh_interfaces(&shared, port, &mut bound);
            }
            shared.close();
            #[cfg(target_os = "linux")]
            for (_, stop) in bound.values() {
                stop.store(true, Ordering::Relaxed);
            }
        });
        Ok(Self { stop })
    }
}

fn open_flow(srt_port: u16, session: u64, flow_id: u16, peer: Arc<Mutex<Peer>>) -> Result<Flow> {
    let local = local_socket()?;
    local.connect(("127.0.0.1", srt_port))?;
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
                (p.addr, p.arrival.clone())
            };
            send(&arrival, &packet, addr);
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

        let proxy = Arc::new(local_socket()?);
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
