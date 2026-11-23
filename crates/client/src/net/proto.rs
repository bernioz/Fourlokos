use std::io::{self, Read, Write};

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_STRING: usize = 256;
pub const MAX_FRAME: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    Hello {
        name: String,
        version: u32,
    },
    Welcome {
        player_id: u32,
        seed: u64,
        world_name: String,
        players: Vec<(u32, String)>,
    },
    Reject {
        reason: String,
    },
    PlayerState {
        id: u32,
        pos: [f64; 3],
        rot: [f32; 4],
        planet_frame: Option<u32>,
    },
    PlayerJoined {
        id: u32,
        name: String,
    },
    PlayerLeft {
        id: u32,
    },
    BlockEdit {
        planet: u32,
        pos: [i32; 3],
        block: u16,
    },
    Chat {
        id: u32,
        text: String,
    },
    Ping,
    Pong,
    Announce {
        world_name: String,
        host_name: String,
        port: u16,
        players: u8,
        version: u32,
    },
}
#[derive(Debug, Clone, PartialEq)]
pub struct Welcome {
    pub player_id: u32,
    pub seed: u64,
    pub world_name: String,
    pub players: Vec<(u32, String)>,
}

struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if n > self.b.len().saturating_sub(self.p) {
            return Err("truncated message".into());
        }
        let r = &self.b[self.p..self.p + n];
        self.p += n;
        Ok(r)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32, String> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn string(&mut self) -> Result<String, String> {
        let n = self.u16()? as usize;
        if n > MAX_STRING {
            return Err("string too long".into());
        }
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| "invalid utf-8".into())
    }
    fn done(&self) -> Result<(), String> {
        if self.p == self.b.len() {
            Ok(())
        } else {
            Err("trailing bytes".into())
        }
    }
}
fn string(out: &mut Vec<u8>, s: &str) -> Result<(), String> {
    if s.len() > MAX_STRING {
        return Err("string too long".into());
    }
    out.extend_from_slice(&(s.len() as u16).to_be_bytes());
    out.extend_from_slice(s.as_bytes());
    Ok(())
}
fn u32(out: &mut Vec<u8>, n: u32) {
    out.extend_from_slice(&n.to_be_bytes());
}
fn u64(out: &mut Vec<u8>, n: u64) {
    out.extend_from_slice(&n.to_be_bytes());
}
fn i32(out: &mut Vec<u8>, n: i32) {
    out.extend_from_slice(&n.to_be_bytes());
}

pub fn encode(msg: &Msg) -> Vec<u8> {
    let mut o = Vec::new();
    let result: Result<(), String> = (|| {
        match msg {
            Msg::Hello { name, version } => {
                o.push(0);
                string(&mut o, name)?;
                u32(&mut o, *version);
            }
            Msg::Welcome {
                player_id,
                seed,
                world_name,
                players,
            } => {
                o.push(1);
                u32(&mut o, *player_id);
                u64(&mut o, *seed);
                string(&mut o, world_name)?;
                if players.len() > u8::MAX as usize {
                    return Err("too many players".into());
                }
                o.push(players.len() as u8);
                for (id, n) in players {
                    u32(&mut o, *id);
                    string(&mut o, n)?;
                }
            }
            Msg::Reject { reason } => {
                o.push(2);
                string(&mut o, reason)?;
            }
            Msg::PlayerState {
                id,
                pos,
                rot,
                planet_frame,
            } => {
                o.push(3);
                u32(&mut o, *id);
                for x in pos {
                    o.extend_from_slice(&x.to_be_bytes())
                }
                for x in rot {
                    o.extend_from_slice(&x.to_be_bytes())
                }
                o.push(planet_frame.is_some() as u8);
                if let Some(x) = planet_frame {
                    u32(&mut o, *x)
                }
            }
            Msg::PlayerJoined { id, name } => {
                o.push(4);
                u32(&mut o, *id);
                string(&mut o, name)?
            }
            Msg::PlayerLeft { id } => {
                o.push(5);
                u32(&mut o, *id)
            }
            Msg::BlockEdit { planet, pos, block } => {
                o.push(6);
                u32(&mut o, *planet);
                for x in pos {
                    i32(&mut o, *x)
                }
                o.extend_from_slice(&block.to_be_bytes())
            }
            Msg::Chat { id, text } => {
                o.push(7);
                u32(&mut o, *id);
                string(&mut o, text)?
            }
            Msg::Ping => o.push(8),
            Msg::Pong => o.push(9),
            Msg::Announce {
                world_name,
                host_name,
                port,
                players,
                version,
            } => {
                o.push(10);
                string(&mut o, world_name)?;
                string(&mut o, host_name)?;
                o.extend_from_slice(&port.to_be_bytes());
                o.push(*players);
                u32(&mut o, *version)
            }
        }
        Ok(())
    })();
    result.expect("Msg contains an oversized string");
    o
}

pub fn decode(bytes: &[u8]) -> Result<Msg, String> {
    if bytes.is_empty() || bytes.len() > MAX_FRAME {
        return Err("invalid frame size".into());
    }
    let mut r = Reader { b: bytes, p: 1 };
    let tag = bytes[0];
    let msg = match tag {
        0 => Msg::Hello {
            name: r.string()?,
            version: r.u32()?,
        },
        1 => {
            let id = r.u32()?;
            let seed = r.u64()?;
            let world = r.string()?;
            let n = r.u8()? as usize;
            if n > 64 {
                return Err("too many players".into());
            }
            let mut p = Vec::with_capacity(n);
            for _ in 0..n {
                p.push((r.u32()?, r.string()?))
            }
            Msg::Welcome {
                player_id: id,
                seed,
                world_name: world,
                players: p,
            }
        }
        2 => Msg::Reject {
            reason: r.string()?,
        },
        3 => {
            let id = r.u32()?;
            let mut pos = [0.; 3];
            for x in &mut pos {
                *x = f64::from_be_bytes(r.take(8)?.try_into().unwrap())
            }
            let mut rot = [0.; 4];
            for x in &mut rot {
                *x = f32::from_be_bytes(r.take(4)?.try_into().unwrap())
            }
            let p = match r.u8()? {
                0 => None,
                1 => Some(r.u32()?),
                _ => return Err("invalid optional value".into()),
            };
            Msg::PlayerState {
                id,
                pos,
                rot,
                planet_frame: p,
            }
        }
        4 => Msg::PlayerJoined {
            id: r.u32()?,
            name: r.string()?,
        },
        5 => Msg::PlayerLeft { id: r.u32()? },
        6 => {
            let planet = r.u32()?;
            let mut pos = [0; 3];
            for x in &mut pos {
                *x = r.i32()?
            }
            let block = r.u16()?;
            Msg::BlockEdit { planet, pos, block }
        }
        7 => Msg::Chat {
            id: r.u32()?,
            text: r.string()?,
        },
        8 => Msg::Ping,
        9 => Msg::Pong,
        10 => {
            let w = r.string()?;
            let h = r.string()?;
            let port = r.u16()?;
            let players = r.u8()?;
            let version = r.u32()?;
            Msg::Announce {
                world_name: w,
                host_name: h,
                port,
                players,
                version,
            }
        }
        _ => return Err("unknown message tag".into()),
    };
    r.done()?;
    Ok(msg)
}
pub fn write_frame(w: &mut impl Write, msg: &Msg) -> io::Result<()> {
    let body = encode(msg);
    if body.len() > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "frame too large",
        ));
    }
    w.write_all(&(body.len() as u32).to_be_bytes())?;
    w.write_all(&body)?;
    w.flush()
}
pub fn read_frame(r: &mut impl Read) -> io::Result<Msg> {
    let mut len = [0; 4];
    r.read_exact(&mut len)?;
    let n = u32::from_be_bytes(len) as usize;
    if n == 0 || n > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid frame length",
        ));
    }
    let mut b = vec![0; n];
    r.read_exact(&mut b)?;
    decode(&b).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}
