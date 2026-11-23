use super::proto::{Msg, PROTOCOL_VERSION, Welcome, read_frame, write_frame};
use std::{
    net::{TcpStream, ToSocketAddrs},
    sync::mpsc,
    thread,
    time::Duration,
};
pub enum ClientEvent {
    Msg(Msg),
    Disconnected(String),
}
pub struct Client {
    tx: mpsc::Sender<Msg>,
    rx: mpsc::Receiver<ClientEvent>,
    close: TcpStream,
}
impl Client {
    pub fn connect(addr: impl ToSocketAddrs, name: &str) -> Result<(Self, Welcome), String> {
        let addr = addr
            .to_socket_addrs()
            .map_err(|e| e.to_string())?
            .next()
            .ok_or("no address")?;
        let mut stream =
            TcpStream::connect_timeout(&addr, Duration::from_secs(3)).map_err(|e| e.to_string())?;
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .map_err(|e| e.to_string())?;
        write_frame(
            &mut stream,
            &Msg::Hello {
                name: name.into(),
                version: PROTOCOL_VERSION,
            },
        )
        .map_err(|e| e.to_string())?;
        let welcome = match read_frame(&mut stream).map_err(|e| e.to_string())? {
            Msg::Welcome {
                player_id,
                seed,
                world_name,
                players,
            } => Welcome {
                player_id,
                seed,
                world_name,
                players,
            },
            Msg::Reject { reason } => return Err(reason),
            _ => return Err("invalid handshake response".into()),
        };
        stream.set_read_timeout(None).map_err(|e| e.to_string())?;
        let reader = stream.try_clone().map_err(|e| e.to_string())?;
        let (writer_tx, writer_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let close = reader.try_clone().map_err(|e| e.to_string())?;
        thread::spawn(move || {
            let mut s = stream;
            for m in writer_rx {
                if write_frame(&mut s, &m).is_err() {
                    break;
                }
            }
        });
        thread::spawn(move || {
            let mut s = reader;
            loop {
                match read_frame(&mut s) {
                    Ok(m) => {
                        if event_tx.send(ClientEvent::Msg(m)).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = event_tx.send(ClientEvent::Disconnected(e.to_string()));
                        break;
                    }
                }
            }
        });
        Ok((
            Self {
                tx: writer_tx,
                rx: event_rx,
                close,
            },
            welcome,
        ))
    }
    pub fn poll(&self) -> Vec<ClientEvent> {
        self.rx.try_iter().collect()
    }
    pub fn send(&self, msg: &Msg) {
        let _ = self.tx.send(msg.clone());
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.close.shutdown(std::net::Shutdown::Both);
    }
}
impl std::fmt::Debug for ClientEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ClientEvent")
    }
}
