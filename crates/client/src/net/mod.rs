#![allow(dead_code)]
//! LAN networking: the host is authoritative and all game traffic uses TCP.
//! The game loop can send state messages at about 20 Hz; UDP is reserved for discovery.

pub mod client;
pub mod discovery;
pub mod host;
pub mod proto;

#[allow(unused_imports)]
pub use client::{Client, ClientEvent};
#[allow(unused_imports)]
pub use discovery::{AnnounceInfo, Announcer, Browser, DEFAULT_PORT, Found};
#[allow(unused_imports)]
pub use host::{Host, HostEvent};
#[allow(unused_imports)]
pub use proto::{Msg, PROTOCOL_VERSION, Welcome, decode, encode, read_frame, write_frame};

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        thread,
        time::{Duration, Instant},
    };

    fn wait_until<T>(mut f: impl FnMut() -> Option<T>) -> T {
        let end = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(v) = f() {
                return v;
            }
            assert!(Instant::now() < end, "timed out waiting for network event");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn every_message_round_trips_and_garbage_is_safe() {
        let messages = vec![
            Msg::Hello {
                name: "a".into(),
                version: 1,
            },
            Msg::Welcome {
                player_id: 2,
                seed: 3,
                world_name: "w".into(),
                players: vec![(0, "h".into())],
            },
            Msg::Reject {
                reason: "no".into(),
            },
            Msg::Ping,
            Msg::Pong,
            Msg::PlayerState {
                id: 4,
                pos: [1.0, -2.0, 3.5],
                rot: [0.0, 1.0, 0.0, 1.0],
                planet_frame: Some(8),
            },
            Msg::PlayerState {
                id: 4,
                pos: [0.0; 3],
                rot: [0.0; 4],
                planet_frame: None,
            },
            Msg::PlayerJoined {
                id: 4,
                name: "n".into(),
            },
            Msg::PlayerLeft { id: 4 },
            Msg::BlockEdit {
                planet: 1,
                pos: [-1, 2, 3],
                block: 0,
            },
            Msg::Chat {
                id: 4,
                text: "hi".into(),
            },
            Msg::Announce {
                world_name: "w".into(),
                host_name: "h".into(),
                port: 12,
                players: 2,
                version: 1,
            },
        ];
        for msg in messages {
            assert_eq!(decode(&encode(&msg)).unwrap(), msg);
        }
        let mut seed = 7u64;
        for len in 0..300 {
            let mut bytes = vec![0; len];
            for b in &mut bytes {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                *b = (seed >> 32) as u8;
            }
            let _ = decode(&bytes);
        }
    }

    #[test]
    fn host_client_join_broadcast_and_leave() {
        let port = free_port();
        let host = Host::start(port, "world", 42, "host").unwrap();
        let (c1, _) = Client::connect(("127.0.0.1", port), "one").unwrap();
        let (c2, _) = Client::connect(("127.0.0.1", port), "two").unwrap();
        let mut joined = Vec::new();
        wait_until(|| {
            joined.extend(host.poll().into_iter().filter_map(|x| match x {
                HostEvent::Joined { id, .. } => Some(id),
                _ => None,
            }));
            (joined.len() >= 2).then_some(())
        });
        assert_eq!(joined.len(), 2);
        assert_eq!(host.player_count(), 2);
        host.broadcast(
            &Msg::Chat {
                id: 0,
                text: "hello".into(),
            },
            None,
        );
        assert!(
            wait_until(|| c1.poll().into_iter().find_map(|e| {
                if let ClientEvent::Msg(Msg::Chat { text, .. }) = e {
                    Some(text)
                } else {
                    None
                }
            }))
            .as_str()
                == "hello"
        );
        assert!(
            wait_until(|| c2.poll().into_iter().find_map(|e| {
                if let ClientEvent::Msg(Msg::Chat { text, .. }) = e {
                    Some(text)
                } else {
                    None
                }
            }))
            .as_str()
                == "hello"
        );
        let mut raw = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        write_frame(
            &mut raw,
            &Msg::Hello {
                name: "leaving".into(),
                version: PROTOCOL_VERSION,
            },
        )
        .unwrap();
        assert!(matches!(read_frame(&mut raw).unwrap(), Msg::Welcome { .. }));
        raw.shutdown(std::net::Shutdown::Both).unwrap();
        wait_until(|| {
            host.poll().into_iter().find_map(|e| {
                if let HostEvent::Left { id } = e {
                    Some(id)
                } else {
                    None
                }
            })
        });
    }

    #[test]
    fn reject_wrong_version() {
        let port = free_port();
        let _host = Host::start(port, "w", 1, "h").unwrap();
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        write_frame(
            &mut stream,
            &Msg::Hello {
                name: "x".into(),
                version: 999,
            },
        )
        .unwrap();
        assert!(matches!(
            read_frame(&mut stream).unwrap(),
            Msg::Reject { .. }
        ));
    }

    #[test]
    fn announcer_browser_loopback() {
        let port = free_port();
        let mut browser = Browser::bind(port).unwrap();
        let _announcer = Announcer::new(
            port,
            AnnounceInfo {
                world_name: "w".into(),
                host_name: "h".into(),
                port: 1234,
                players: 3,
                version: PROTOCOL_VERSION,
            },
        )
        .unwrap();
        let found = wait_until(|| {
            browser
                .poll()
                .into_iter()
                .find(|f| f.announce.world_name == "w")
        });
        assert_eq!(found.announce.host_name, "h");
    }

    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }
}
