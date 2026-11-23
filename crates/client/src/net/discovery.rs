use super::proto::{Msg, decode, encode};
use std::{
    collections::HashMap,
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub const DEFAULT_PORT: u16 = 47777;
#[derive(Clone, Debug)]
pub struct AnnounceInfo {
    pub world_name: String,
    pub host_name: String,
    pub port: u16,
    pub players: u8,
    pub version: u32,
}
impl From<AnnounceInfo> for Msg {
    fn from(x: AnnounceInfo) -> Self {
        Msg::Announce {
            world_name: x.world_name,
            host_name: x.host_name,
            port: x.port,
            players: x.players,
            version: x.version,
        }
    }
}
pub struct Announcer {
    info: Arc<Mutex<AnnounceInfo>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Announcer {
    pub fn new(port: u16, info: AnnounceInfo) -> io::Result<Self> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        socket.set_broadcast(true)?;
        socket.set_read_timeout(Some(Duration::from_millis(100)))?;
        let info = Arc::new(Mutex::new(info));
        let share = info.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let quit = stop.clone();
        let t = thread::spawn(move || {
            while !quit.load(Ordering::Relaxed) {
                let msg = share.lock().unwrap().clone();
                let bytes = [b"FLKS1".as_slice(), encode(&Msg::from(msg)).as_slice()].concat();
                let local = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok();
                if let Some(s) = local {
                    let _ = s.set_broadcast(true);
                    let _ = s.send_to(&bytes, (Ipv4Addr::BROADCAST, DEFAULT_PORT));
                    let _ = s.send_to(&bytes, (Ipv4Addr::LOCALHOST, DEFAULT_PORT));
                    let _ = s.send_to(&bytes, (Ipv4Addr::BROADCAST, port));
                    let _ = s.send_to(&bytes, (Ipv4Addr::LOCALHOST, port));
                    if let Ok(addr) = local_broadcast() {
                        let _ = s.send_to(&bytes, (addr, port));
                    }
                }
                for _ in 0..10 {
                    if quit.load(Ordering::Relaxed) {
                        break;
                    }
                    thread::sleep(Duration::from_millis(100));
                }
            }
        });
        Ok(Self {
            info,
            stop,
            thread: Some(t),
        })
    }
    pub fn info(&self) -> Arc<Mutex<AnnounceInfo>> {
        self.info.clone()
    }
}
impl Drop for Announcer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
fn local_broadcast() -> io::Result<Ipv4Addr> {
    let s = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    s.connect((Ipv4Addr::new(8, 8, 8, 8), 9))?;
    match s.local_addr()?.ip() {
        IpAddr::V4(ip) => {
            let o = ip.octets();
            Ok(Ipv4Addr::new(o[0], o[1], o[2], 255))
        }
        _ => Err(io::Error::other("not ipv4")),
    }
}

#[derive(Clone, Debug)]
pub struct Found {
    pub addr: SocketAddr,
    pub announce: AnnounceInfo,
    pub last_seen: Instant,
}
pub struct Browser {
    socket: UdpSocket,
    found: HashMap<SocketAddr, Found>,
}
/// Binds the wildcard address first. If the platform refuses a second wildcard
/// listener (std has no portable `SO_REUSEADDR` API), it falls back to loopback.
impl Browser {
    pub fn bind(port: u16) -> io::Result<Self> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, port))
            .or_else(|_| UdpSocket::bind((Ipv4Addr::LOCALHOST, port)))?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            found: HashMap::new(),
        })
    }
    pub fn poll(&mut self) -> Vec<Found> {
        let mut b = [0; 65536];
        loop {
            match self.socket.recv_from(&mut b) {
                Ok((n, addr)) => {
                    if n >= 5 && &b[..5] == b"FLKS1" {
                        if let Ok(Msg::Announce {
                            world_name,
                            host_name,
                            port,
                            players,
                            version,
                        }) = decode(&b[5..n])
                        {
                            self.found.insert(
                                addr,
                                Found {
                                    addr,
                                    announce: AnnounceInfo {
                                        world_name,
                                        host_name,
                                        port,
                                        players,
                                        version,
                                    },
                                    last_seen: Instant::now(),
                                },
                            );
                        }
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
        let now = Instant::now();
        self.found
            .retain(|_, x| now.duration_since(x.last_seen) < Duration::from_secs(4));
        self.found.values().cloned().collect()
    }
}
