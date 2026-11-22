//! Just enough ZeroMQ (ZMTP 3.0 over TCP, NULL security) and Jupyter messaging
//! to talk to a kernel: DEALERs for the shell, stdin and control channels and a
//! SUB for IOPub.

use crate::json::{self, obj, Value};
use crate::sha256::hmac_hex;
use std::io::{self, Read, Write};
use std::net::TcpStream;

pub struct Socket {
    stream: TcpStream,
}

impl Socket {
    pub fn connect(addr: &str, socket_type: &str) -> io::Result<Socket> {
        Socket::connect_as(addr, socket_type, None)
    }

    /// With an identity, the kernel's ROUTER knows this connection by it: the
    /// shell and stdin sockets share one, so the kernel can ask for input
    /// (`readline()`) on the stdin channel of whoever sent the request.
    pub fn connect_as(addr: &str, socket_type: &str, identity: Option<&[u8]>) -> io::Result<Socket> {
        let mut stream = TcpStream::connect(addr)?;
        stream.set_nodelay(true)?;
        // Greeting: signature, version 3.0, mechanism NULL, as-server 0, filler.
        let mut greeting = [0u8; 64];
        greeting[0] = 0xFF;
        greeting[9] = 0x7F;
        greeting[10] = 3;
        greeting[12..16].copy_from_slice(b"NULL");
        stream.write_all(&greeting)?;
        let mut peer = [0u8; 64];
        stream.read_exact(&mut peer)?;
        if peer[0] != 0xFF || peer[9] != 0x7F || peer[10] < 3 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, crate::i18n::tr("the kernel does not speak ZMTP 3", "el kernel no habla ZMTP 3")));
        }
        // READY with our socket type, then the peer's READY.
        let mut ready = vec![5u8];
        ready.extend_from_slice(b"READY");
        ready.push(11);
        ready.extend_from_slice(b"Socket-Type");
        ready.extend_from_slice(&(socket_type.len() as u32).to_be_bytes());
        ready.extend_from_slice(socket_type.as_bytes());
        if let Some(id) = identity {
            ready.push(8);
            ready.extend_from_slice(b"Identity");
            ready.extend_from_slice(&(id.len() as u32).to_be_bytes());
            ready.extend_from_slice(id);
        }
        let mut sock = Socket { stream };
        sock.write_frame(&ready, false, true)?;
        loop {
            let (flags, _) = sock.read_frame()?;
            if flags & 4 != 0 {
                break; // the peer's READY command
            }
        }
        Ok(sock)
    }

    fn write_frame(&mut self, body: &[u8], more: bool, command: bool) -> io::Result<()> {
        let mut flags = (more as u8) | ((command as u8) << 2);
        let mut head = Vec::with_capacity(9);
        if body.len() > 255 {
            flags |= 2;
            head.push(flags);
            head.extend_from_slice(&(body.len() as u64).to_be_bytes());
        } else {
            head.push(flags);
            head.push(body.len() as u8);
        }
        self.stream.write_all(&head)?;
        self.stream.write_all(body)
    }

    fn read_frame(&mut self) -> io::Result<(u8, Vec<u8>)> {
        let mut flags = [0u8; 1];
        self.stream.read_exact(&mut flags)?;
        let len = if flags[0] & 2 != 0 {
            let mut n = [0u8; 8];
            self.stream.read_exact(&mut n)?;
            u64::from_be_bytes(n) as usize
        } else {
            let mut n = [0u8; 1];
            self.stream.read_exact(&mut n)?;
            n[0] as usize
        };
        let mut body = vec![0u8; len];
        self.stream.read_exact(&mut body)?;
        Ok((flags[0], body))
    }

    pub fn send(&mut self, parts: &[&[u8]]) -> io::Result<()> {
        for (i, part) in parts.iter().enumerate() {
            self.write_frame(part, i + 1 < parts.len(), false)?;
        }
        self.stream.flush()
    }

    /// The next message (all its frames); commands such as heartbeats are skipped.
    pub fn recv(&mut self) -> io::Result<Vec<Vec<u8>>> {
        let mut parts = Vec::new();
        loop {
            let (flags, body) = self.read_frame()?;
            if flags & 4 != 0 {
                continue;
            }
            parts.push(body);
            if flags & 1 == 0 {
                return Ok(parts);
            }
        }
    }

    /// A second handle on the same connection (one thread reads, another writes).
    pub fn try_clone(&self) -> io::Result<Socket> {
        Ok(Socket { stream: self.stream.try_clone()? })
    }

    /// SUB: receive everything the kernel publishes.
    pub fn subscribe_all(&mut self) -> io::Result<()> {
        self.send(&[&[1u8]])
    }
}

/// The parts of a kernel connection file this needs.
pub struct Connection {
    pub ip: String,
    pub key: Vec<u8>,
    pub shell: u16,
    pub iopub: u16,
    pub stdin: u16,
    pub control: u16,
}

impl Connection {
    pub fn read(path: &str) -> Result<Connection, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        let v = json::parse(&text)?;
        let port = |k: &str| v.get(k).num().map(|n| n as u16).ok_or_else(|| trf!("{k} is missing in {path}", "falta {k} en {path}"));
        Ok(Connection {
            ip: v.get("ip").str().unwrap_or("127.0.0.1").to_string(),
            key: v.get("key").str().unwrap_or("").as_bytes().to_vec(),
            shell: port("shell_port")?,
            iopub: port("iopub_port")?,
            stdin: v.get("stdin_port").num().unwrap_or(0.0) as u16,
            control: v.get("control_port").num().unwrap_or(0.0) as u16,
        })
    }

    pub fn addr(&self, port: u16) -> String {
        format!("{}:{}", self.ip, port)
    }
}

pub fn uuid() -> String {
    let h = crate::env::random_hex(16);
    format!("{}-{}-4{}-a{}-{}", &h[0..8], &h[8..12], &h[13..16], &h[17..20], &h[20..32])
}

/// A signed Jupyter message on a DEALER socket; returns its id.
pub fn send_message(sock: &mut Socket, key: &[u8], session: &str, msg_type: &str, content: &Value) -> io::Result<String> {
    send_reply(sock, key, session, msg_type, content, &Value::obj())
}

/// The same, answering `parent` (an input_reply answers the kernel's input_request).
pub fn send_reply(sock: &mut Socket, key: &[u8], session: &str, msg_type: &str, content: &Value, parent: &Value) -> io::Result<String> {
    let id = uuid();
    let header = obj(vec![
        ("msg_id", id.as_str().into()),
        ("session", session.into()),
        ("username", "corral".into()),
        ("date", iso_now().into()),
        ("msg_type", msg_type.into()),
        ("version", "5.3".into()),
    ])
    .to_json();
    let (parent, metadata, content) = (parent.to_json(), "{}".to_string(), content.to_json());
    let signature = if key.is_empty() {
        String::new()
    } else {
        hmac_hex(key, &[header.as_bytes(), parent.as_bytes(), metadata.as_bytes(), content.as_bytes()])
    };
    sock.send(&[b"<IDS|MSG>", signature.as_bytes(), header.as_bytes(), parent.as_bytes(),
                metadata.as_bytes(), content.as_bytes()])?;
    Ok(id)
}

/// Header and content of a Jupyter message (the signature isn't checked: local kernel).
pub fn parse_message(parts: &[Vec<u8>]) -> Option<(Value, Value)> {
    parse_full(parts).map(|(header, _, content)| (header, content))
}

/// Header, parent header and content.
pub fn parse_full(parts: &[Vec<u8>]) -> Option<(Value, Value, Value)> {
    let i = parts.iter().position(|p| p.as_slice() == b"<IDS|MSG>")?;
    let text = |k: usize| std::str::from_utf8(parts.get(i + k)?).ok().and_then(|t| json::parse(t).ok());
    Some((text(2)?, text(3).unwrap_or(Value::obj()), text(5)?))
}

/// UTC time as ISO 8601, without a date library.
fn iso_now() -> String {
    let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let secs = d.as_secs() as i64;
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    // Days to civil date (Howard Hinnant's algorithm).
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + (month <= 2) as i64;
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:06}Z",
            rem / 3600, rem % 3600 / 60, rem % 60, d.subsec_micros())
}
