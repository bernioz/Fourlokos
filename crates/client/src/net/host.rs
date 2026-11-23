use super::{
    discovery::{AnnounceInfo, Announcer, DEFAULT_PORT},
    proto::{Msg, PROTOCOL_VERSION, read_frame, write_frame},
};
use std::{
    collections::HashMap,
    io,
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};
#[derive(Debug)]
pub enum HostEvent {
    Joined { id: u32, name: String },
    Left { id: u32 },
    FromClient { id: u32, msg: Msg },
}
struct Shared {
    clients: HashMap<u32, mpsc::Sender<Msg>>,
    names: HashMap<u32, String>,
}
pub struct Host {
    events: mpsc::Receiver<HostEvent>,
    shared: Arc<Mutex<Shared>>,
    _listener: thread::JoinHandle<()>,
    _announcer: Announcer,
}
impl Host {
    pub fn start(
        port: u16,
        world_name: impl Into<String>,
        seed: u64,
        host_name: impl Into<String>,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind(("0.0.0.0", port))?;
        listener.set_nonblocking(true)?;
        let actual = listener.local_addr()?.port();
        let shared = Arc::new(Mutex::new(Shared {
            clients: HashMap::new(),
            names: HashMap::new(),
        }));
        let (sh_tx, events) = mpsc::channel();
        let s = shared.clone();
        let world = world_name.into();
        let hn = host_name.into();
        let announcer = Announcer::new(
            DEFAULT_PORT,
            AnnounceInfo {
                world_name: world.clone(),
                host_name: hn,
                port: actual,
                players: 0,
                version: PROTOCOL_VERSION,
            },
        )?;
        let join = thread::spawn(move || {
            let mut next = 1u32;
            loop {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let id = next;
                        next += 1;
                        let s = s.clone();
                        let tx = sh_tx.clone();
                        let w = world.clone();
                        thread::spawn(move || client_thread(id, stream, s, tx, w, seed));
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            events,
            shared,
            _listener: join,
            _announcer: announcer,
        })
    }
    pub fn poll(&self) -> Vec<HostEvent> {
        self.events.try_iter().collect()
    }
    pub fn send(&self, id: u32, msg: &Msg) {
        if let Some(tx) = self.shared.lock().unwrap().clients.get(&id) {
            let _ = tx.send(msg.clone());
        }
    }
    pub fn broadcast(&self, msg: &Msg, except: Option<u32>) {
        for (&id, tx) in &self.shared.lock().unwrap().clients {
            if Some(id) != except {
                let _ = tx.send(msg.clone());
            }
        }
    }
    pub fn player_count(&self) -> usize {
        self.shared.lock().unwrap().clients.len()
    }
}
fn client_thread(
    id: u32,
    mut stream: TcpStream,
    shared: Arc<Mutex<Shared>>,
    events: mpsc::Sender<HostEvent>,
    world: String,
    seed: u64,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let hello = match read_frame(&mut stream) {
        Ok(Msg::Hello { name, version }) => {
            if version != PROTOCOL_VERSION {
                let _ = write_frame(
                    &mut stream,
                    &Msg::Reject {
                        reason: "version mismatch".into(),
                    },
                );
                return;
            }
            if name.trim().is_empty() {
                let _ = write_frame(
                    &mut stream,
                    &Msg::Reject {
                        reason: "empty name".into(),
                    },
                );
                return;
            }
            name
        }
        _ => {
            let _ = write_frame(
                &mut stream,
                &Msg::Reject {
                    reason: "expected hello".into(),
                },
            );
            return;
        }
    };
    let _ = stream.set_read_timeout(None);
    {
        let mut g = shared.lock().unwrap();
        if g.names.values().any(|n| n == &hello) {
            let _ = write_frame(
                &mut stream,
                &Msg::Reject {
                    reason: "duplicate name".into(),
                },
            );
            return;
        }
        let players = g
            .names
            .iter()
            .map(|(&i, n)| (i, n.clone()))
            .chain(std::iter::once((0, "host".into())))
            .collect();
        if write_frame(
            &mut stream,
            &Msg::Welcome {
                player_id: id,
                seed,
                world_name: world,
                players,
            },
        )
        .is_err()
        {
            return;
        }
        g.names.insert(id, hello.clone());
        let (tx, rx) = mpsc::channel();
        g.clients.insert(id, tx);
        drop(g);
        let _ = events.send(HostEvent::Joined {
            id,
            name: hello.clone(),
        });
        for (&other, tx) in &shared.lock().unwrap().clients {
            if other != id {
                let _ = tx.send(Msg::PlayerJoined {
                    id,
                    name: hello.clone(),
                });
            }
        }
        let reader = stream.try_clone().unwrap();
        let writer = thread::spawn(move || {
            let mut s = stream;
            for m in rx {
                if write_frame(&mut s, &m).is_err() {
                    break;
                }
            }
        });
        let mut r = reader;
        loop {
            match read_frame(&mut r) {
                Ok(m) => {
                    if events.send(HostEvent::FromClient { id, msg: m }).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let mut g = shared.lock().unwrap();
        g.clients.remove(&id);
        g.names.remove(&id);
        for tx in g.clients.values() {
            let _ = tx.send(Msg::PlayerLeft { id });
        }
        drop(g);
        let _ = writer.join();
    }
    let _ = events.send(HostEvent::Left { id });
}
