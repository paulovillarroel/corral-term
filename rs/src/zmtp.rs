//! Just enough ZeroMQ (ZMTP 3.0 over a Unix socket or TCP, NULL security) and
//! Jupyter messaging to talk to a kernel: DEALERs for the shell, stdin and control
//! channels and a SUB for IOPub.

use crate::json::{self, obj, Value};
use crate::sha256::hmac_hex;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::os::unix::net::UnixStream;
use std::time::Duration;

/// How long a kernel has to answer the ZMTP handshake.
const HANDSHAKE: Duration = Duration::from_secs(10);

/// A connection to one of the kernel's channels. An address that is a path is a Unix
/// socket ("ipc" in the connection file; Ark names it ip:port, as it does a TCP
/// address): only processes that can open its folder reach it. Anything else is TCP.
enum Stream {
    Tcp(TcpStream),
    Unix(UnixStream),
}

impl Stream {
    fn connect(addr: &str) -> io::Result<Stream> {
        if addr.starts_with('/') {
            return Ok(Stream::Unix(UnixStream::connect(addr)?));
        }
        let s = TcpStream::connect(addr)?;
        s.set_nodelay(true)?;
        Ok(Stream::Tcp(s))
    }

    fn set_read_timeout(&self, limit: Option<Duration>) -> io::Result<()> {
        match self {
            Stream::Tcp(s) => s.set_read_timeout(limit),
            Stream::Unix(s) => s.set_read_timeout(limit),
        }
    }

    fn try_clone(&self) -> io::Result<Stream> {
        Ok(match self {
            Stream::Tcp(s) => Stream::Tcp(s.try_clone()?),
            Stream::Unix(s) => Stream::Unix(s.try_clone()?),
        })
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Stream::Tcp(s) => s.read(buf),
            Stream::Unix(s) => s.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Stream::Tcp(s) => s.write(buf),
            Stream::Unix(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Stream::Tcp(s) => s.flush(),
            Stream::Unix(s) => s.flush(),
        }
    }
}

pub struct Socket {
    stream: Stream,
}

impl Socket {
    pub fn connect(addr: &str, socket_type: &str) -> io::Result<Socket> {
        Socket::connect_as(addr, socket_type, None)
    }

    /// With an identity, the kernel's ROUTER knows this connection by it: the
    /// shell and stdin sockets share one, so the kernel can ask for input
    /// (`readline()`) on the stdin channel of whoever sent the request.
    pub fn connect_as(addr: &str, socket_type: &str, identity: Option<&[u8]>) -> io::Result<Socket> {
        Socket::connect_within(addr, socket_type, identity, HANDSHAKE)
    }

    /// The handshake has `limit` to finish: a kernel that stalls in the middle
    /// of it is an error, not a console that waits forever.
    fn connect_within(addr: &str, socket_type: &str, identity: Option<&[u8]>, limit: Duration) -> io::Result<Socket> {
        let mut stream = Stream::connect(addr)?;
        stream.set_read_timeout(Some(limit))?;
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
        sock.stream.set_read_timeout(None)?; // from here, the caller decides how long to wait
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
        // Grown as the bytes arrive: a damaged length must not reserve memory
        // it will never fill (a huge one would abort the process).
        let mut body = Vec::with_capacity(len.min(1 << 20));
        (&mut self.stream).take(len as u64).read_to_end(&mut body)?;
        if body.len() < len {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
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

/// Header and content of a message signed with `key` (see `parse_signed`).
pub fn parse_message(parts: &[Vec<u8>], key: &[u8]) -> Option<(Value, Value)> {
    parse_signed(parts, key).map(|(header, _, content)| (header, content))
}

/// Header, parent header and content of a message whose signature matches
/// `key`: the HMAC-SHA256, in hex, of its header, parent header, metadata and
/// content, as Jupyter signs them. Anything else on the port (a wrong or
/// missing signature) is dropped: None. With an empty key nothing is signed.
pub fn parse_signed(parts: &[Vec<u8>], key: &[u8]) -> Option<(Value, Value, Value)> {
    if !key.is_empty() {
        let i = parts.iter().position(|p| p.as_slice() == b"<IDS|MSG>")?;
        let signed: Vec<&[u8]> = (2..6).map(|k| parts.get(i + k).map(Vec::as_slice)).collect::<Option<_>>()?;
        let expected = hmac_hex(key, &signed);
        if !same(parts.get(i + 1)?, expected.as_bytes()) {
            return None;
        }
    }
    parse_full(parts)
}

/// Equal, in a time that doesn't depend on where they differ.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Read a kernel channel until `handle` says stop or the kernel is gone. When
/// the connection drops while `alive()` says the kernel still runs,
/// `reconnect()` opens it again (None gives up) and reading goes on there.
pub fn read_channel(mut sock: Socket, mut alive: impl FnMut() -> bool, mut reconnect: impl FnMut() -> Option<Socket>,
                    mut handle: impl FnMut(Vec<Vec<u8>>) -> bool) {
    loop {
        match sock.recv() {
            Ok(parts) => {
                if !handle(parts) {
                    return;
                }
            }
            Err(_) => {
                if !alive() {
                    return;
                }
                match reconnect() {
                    Some(s) => sock = s,
                    None => return,
                }
            }
        }
    }
}

/// Header, parent header and content, without looking at the signature.
fn parse_full(parts: &[Vec<u8>]) -> Option<(Value, Value, Value)> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{TcpListener, TcpStream};
    use std::thread;
    use std::time::Duration;

    // A broken exchange fails the test instead of hanging it.
    const WAIT: Option<Duration> = Some(Duration::from_secs(5));

    fn listen() -> (TcpListener, String) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap().to_string();
        (l, addr)
    }

    /// One frame as the peer reads it, decoded on its own (RFC 23).
    fn frame(s: &mut impl Read) -> (u8, Vec<u8>) {
        let mut flags = [0u8; 1];
        s.read_exact(&mut flags).unwrap();
        let len = if flags[0] & 0x02 != 0 {
            let mut n = [0u8; 8];
            s.read_exact(&mut n).unwrap();
            u64::from_be_bytes(n) as usize
        } else {
            let mut n = [0u8; 1];
            s.read_exact(&mut n).unwrap();
            n[0] as usize
        };
        let mut body = vec![0u8; len];
        s.read_exact(&mut body).unwrap();
        (flags[0], body)
    }

    fn put(s: &mut impl Write, flags: u8, body: &[u8]) {
        s.write_all(&[flags]).unwrap();
        if flags & 0x02 != 0 {
            s.write_all(&(body.len() as u64).to_be_bytes()).unwrap();
        } else {
            s.write_all(&[body.len() as u8]).unwrap();
        }
        s.write_all(body).unwrap();
    }

    /// A message's frames, as the peer reads them, until one without MORE.
    fn message(s: &mut impl Read) -> Vec<(u8, Vec<u8>)> {
        let mut out = vec![frame(s)];
        while out.last().unwrap().0 & 0x01 != 0 {
            out.push(frame(s));
        }
        out
    }

    /// The properties of a READY command: (name, value) in order.
    fn properties(ready: &[u8]) -> Vec<(String, Vec<u8>)> {
        let skip = 1 + ready[0] as usize;
        let mut p = &ready[skip..];
        let mut out = Vec::new();
        while !p.is_empty() {
            let n = p[0] as usize;
            let name = String::from_utf8(p[1..1 + n].to_vec()).unwrap();
            let len = u32::from_be_bytes(p[1 + n..5 + n].try_into().unwrap()) as usize;
            out.push((name, p[5 + n..5 + n + len].to_vec()));
            p = &p[5 + n + len..];
        }
        out
    }

    /// The kernel's side of the handshake: returns our greeting and READY.
    fn handshake(s: &mut TcpStream) -> ([u8; 64], Vec<u8>) {
        s.set_read_timeout(WAIT).unwrap();
        handshake_on(s)
    }

    fn handshake_on(s: &mut (impl Read + Write)) -> ([u8; 64], Vec<u8>) {
        let mut theirs = [0u8; 64];
        s.read_exact(&mut theirs).unwrap();
        let mut ours = [0u8; 64];
        ours[0] = 0xFF;
        ours[9] = 0x7F;
        ours[10] = 3;
        ours[11] = 1;
        ours[12..16].copy_from_slice(b"NULL");
        ours[32] = 1; // as-server
        s.write_all(&ours).unwrap();
        let (flags, ready) = frame(s);
        assert_eq!(flags, 0x04, "READY goes as a command frame");
        let mut answer = vec![5u8];
        answer.extend_from_slice(b"READY");
        answer.push(11);
        answer.extend_from_slice(b"Socket-Type");
        answer.extend_from_slice(&6u32.to_be_bytes());
        answer.extend_from_slice(b"ROUTER");
        put(s, 0x04, &answer);
        (theirs, ready)
    }

    /// Run `kernel` as the peer and connect to it.
    fn with_kernel<T: Send + 'static>(identity: Option<&[u8]>, kernel: impl FnOnce(&mut TcpStream) -> T + Send + 'static)
        -> (io::Result<Socket>, thread::JoinHandle<T>) {
        let (l, addr) = listen();
        let peer = thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            kernel(&mut s)
        });
        let sock = Socket::connect_as(&addr, "DEALER", identity);
        if let Ok(sock) = &sock {
            sock.stream.set_read_timeout(WAIT).unwrap();
        }
        (sock, peer)
    }

    #[test]
    fn a_path_is_a_unix_socket() {
        // Ark's "ipc" transport: the same handshake and frames, on a socket in a folder.
        use std::os::unix::net::UnixListener;
        let dir = std::env::temp_dir().join(format!("corral-zmtp-unix-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("kernel-1a2b3c4d:1");
        let l = UnixListener::bind(&path).unwrap();
        let peer = thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            s.set_read_timeout(WAIT).unwrap();
            let (_, ready) = handshake_on(&mut s);
            let got = message(&mut s);
            put(&mut s, 0x01, b"one");
            put(&mut s, 0x00, b"two");
            (ready, got)
        });
        let mut sock = Socket::connect_as(path.to_str().unwrap(), "DEALER", Some(b"corral-shell")).unwrap();
        sock.stream.set_read_timeout(WAIT).unwrap();
        sock.send(&[b"hello", b"kernel"]).unwrap();
        assert_eq!(sock.recv().unwrap(), vec![b"one".to_vec(), b"two".to_vec()]);
        let (ready, got) = peer.join().unwrap();
        assert!(properties(&ready).contains(&("Identity".to_string(), b"corral-shell".to_vec())));
        assert_eq!(got.into_iter().map(|(_, b)| b).collect::<Vec<_>>(), vec![b"hello".to_vec(), b"kernel".to_vec()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn greeting_and_ready_follow_zmtp_3() {
        let (sock, peer) = with_kernel(Some(b"corral-shell"), |s| handshake(s));
        sock.unwrap();
        let (greeting, ready) = peer.join().unwrap();
        assert_eq!(greeting[0], 0xFF);
        assert_eq!(&greeting[1..9], &[0u8; 8]);
        assert_eq!(greeting[9], 0x7F);
        assert_eq!(greeting[10], 3);
        assert_eq!(&greeting[12..16], b"NULL");
        assert!(greeting[16..32].iter().all(|&b| b == 0), "mechanism padded with zeros");
        assert_eq!(greeting[32], 0, "we are the client");
        assert!(greeting[33..].iter().all(|&b| b == 0), "filler");
        assert_eq!(&ready[..6], b"\x05READY");
        assert_eq!(properties(&ready), [("Socket-Type".to_string(), b"DEALER".to_vec()),
                                        ("Identity".to_string(), b"corral-shell".to_vec())]);
    }

    #[test]
    fn no_identity_sends_only_the_socket_type() {
        let (sock, peer) = with_kernel(None, |s| handshake(s).1);
        sock.unwrap();
        assert_eq!(properties(&peer.join().unwrap()), [("Socket-Type".to_string(), b"DEALER".to_vec())]);
    }

    #[test]
    fn refuses_a_peer_that_isnt_zmtp() {
        let (sock, peer) = with_kernel(None, |s| {
            let mut ours = [0u8; 64];
            s.read_exact(&mut ours).unwrap();
            let mut http = [b' '; 64];
            http[..17].copy_from_slice(b"HTTP/1.1 200 OK\r\n");
            s.write_all(&http).unwrap();
        });
        assert_eq!(sock.err().map(|e| e.kind()), Some(io::ErrorKind::InvalidData));
        peer.join().unwrap();
    }

    #[test]
    fn short_long_and_multipart_frames() {
        let long = vec![b'x'; 300];
        let (sock, peer) = with_kernel(None, |s| {
            handshake(s);
            let got = message(s);
            // The answer, with a heartbeat command between its frames.
            put(s, 0x01, b"one");
            put(s, 0x04, b"\x04PING\x00\x00");
            put(s, 0x02, &vec![7u8; 70_000]);
            got
        });
        let mut sock = sock.unwrap();
        sock.send(&[b"a", &long, b"", &[9u8; 255]]).unwrap();
        let parts = sock.recv().unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0], b"one");
        assert_eq!(parts[1], vec![7u8; 70_000]);
        let got = peer.join().unwrap();
        assert_eq!(got[0], (0x01, b"a".to_vec()));
        assert_eq!(got[1], (0x03, long), "past 255 bytes: LONG, 8-byte length");
        assert_eq!(got[2], (0x01, Vec::new()));
        assert_eq!(got[3], (0x00, vec![9u8; 255]), "255 bytes still fit a short frame");
    }

    #[test]
    fn subscribe_all_is_one_frame() {
        let (sock, peer) = with_kernel(None, |s| {
            handshake(s);
            message(s)
        });
        sock.unwrap().subscribe_all().unwrap();
        assert_eq!(peer.join().unwrap(), [(0x00, vec![1u8])]);
    }

    #[test]
    fn a_kernel_that_goes_away_is_an_error() {
        // Gone between messages.
        let (sock, peer) = with_kernel(None, |s| {
            handshake(s);
        });
        let mut sock = sock.unwrap();
        peer.join().unwrap();
        assert!(sock.recv().is_err());
        // Gone halfway through one.
        let (sock, peer) = with_kernel(None, |s| {
            handshake(s);
            put(s, 0x01, b"first of two");
        });
        let mut sock = sock.unwrap();
        peer.join().unwrap();
        assert!(sock.recv().is_err());
    }

    #[test]
    fn a_damaged_length_is_an_error_not_a_crash() {
        let (sock, peer) = with_kernel(None, |s| {
            handshake(s);
            s.write_all(&[0x02]).unwrap();
            s.write_all(&(1u64 << 40).to_be_bytes()).unwrap(); // a terabyte
            s.write_all(b"only this").unwrap();
        });
        let mut sock = sock.unwrap();
        peer.join().unwrap();
        assert_eq!(sock.recv().err().map(|e| e.kind()), Some(io::ErrorKind::UnexpectedEof));
    }

    #[test]
    fn messages_are_signed_over_the_four_parts() {
        let key = b"8e5f6d1c-key".to_vec();
        let (sock, peer) = with_kernel(None, |s| {
            handshake(s);
            message(s)
        });
        let mut sock = sock.unwrap();
        let content = obj(vec![("code", "1 + 1".into())]);
        let parent = obj(vec![("msg_id", "parent-1".into())]);
        let id = send_reply(&mut sock, &key, "session-1", "execute_request", &content, &parent).unwrap();
        let frames = peer.join().unwrap();
        let flags: Vec<u8> = frames.iter().map(|f| f.0).collect();
        assert_eq!(flags, [1, 1, 1, 1, 1, 0]);
        let f: Vec<&[u8]> = frames.iter().map(|f| f.1.as_slice()).collect();
        assert_eq!(f[0], b"<IDS|MSG>");
        // HMAC-SHA256 over header, parent header, metadata and content, in hex.
        let expected = hmac_hex(&key, &[&[f[2], f[3], f[4], f[5]].concat()]);
        assert_eq!(f[1], expected.as_bytes());
        let header = json::parse(std::str::from_utf8(f[2]).unwrap()).unwrap();
        assert_eq!(header.get("msg_id").str(), Some(id.as_str()));
        assert_eq!(header.get("session").str(), Some("session-1"));
        assert_eq!(header.get("msg_type").str(), Some("execute_request"));
        assert_eq!(header.get("version").str(), Some("5.3"));
        assert!(header.get("username").str().is_some());
        assert!(header.get("date").str().is_some_and(|d| d.ends_with('Z')));
        assert_eq!(json::parse(std::str::from_utf8(f[3]).unwrap()).unwrap(), parent);
        assert_eq!(f[4], b"{}");
        assert_eq!(json::parse(std::str::from_utf8(f[5]).unwrap()).unwrap(), content);
    }

    #[test]
    fn without_a_key_the_signature_is_empty() {
        let (sock, peer) = with_kernel(None, |s| {
            handshake(s);
            message(s)
        });
        send_message(&mut sock.unwrap(), b"", "s", "kernel_info_request", &Value::obj()).unwrap();
        let frames = peer.join().unwrap();
        assert_eq!(frames.len(), 6);
        assert!(frames[1].1.is_empty());
    }

    #[test]
    fn parse_full_finds_the_delimiter() {
        let parts: Vec<Vec<u8>> = [&b"route-1"[..], b"route-2", b"<IDS|MSG>", b"sig", br#"{"msg_type":"stream"}"#,
                                   br#"{"msg_id":"p"}"#, b"{}", br#"{"text":"hi"}"#, b"buffer"]
            .iter().map(|p| p.to_vec()).collect();
        let (header, parent, content) = parse_full(&parts).unwrap();
        assert_eq!(header.get("msg_type").str(), Some("stream"));
        assert_eq!(parent.get("msg_id").str(), Some("p"));
        assert_eq!(content.get("text").str(), Some("hi"));
        // A broken parent header doesn't lose the message.
        let mut broken = parts.clone();
        broken[5] = b"not json".to_vec();
        assert!(parse_full(&broken).unwrap().1.get("msg_id").is_null());
        assert!(parse_full(&parts[3..]).is_none(), "no delimiter");
        assert!(parse_full(&parts[..6]).is_none(), "no content");
        let mut bad = parts.clone();
        bad[4] = b"{".to_vec();
        assert!(parse_full(&bad).is_none(), "a header that isn't JSON");
    }

    #[test]
    fn uuid_and_date_shapes() {
        let u = uuid();
        let groups: Vec<&str> = u.split('-').collect();
        assert_eq!(groups.iter().map(|g| g.len()).collect::<Vec<_>>(), [8, 4, 4, 4, 12]);
        assert!(u.chars().all(|c| c == '-' || c.is_ascii_hexdigit()));
        assert!(groups[2].starts_with('4') && groups[3].starts_with('a'), "{u}");
        assert_ne!(uuid(), u);
        let d = iso_now(); // 2026-11-23T10:15:00.123456Z
        assert_eq!(d.len(), 27, "{d}");
        assert_eq!((&d[4..5], &d[7..8], &d[10..11], &d[13..14], &d[16..17], &d[19..20], &d[26..]), ("-", "-", "T", ":", ":", ".", "Z"));
        assert!(d[..4].parse::<u32>().unwrap() >= 2024);
    }

    #[test]
    fn reads_a_connection_file() {
        let path = std::env::temp_dir().join(format!("corral-zmtp-test-{}.json", std::process::id()));
        std::fs::write(&path, r#"{"ip":"127.0.0.1","key":"abc","shell_port":5001,"iopub_port":5002,
                                  "stdin_port":5003,"control_port":5004,"hb_port":5005,"signature_scheme":"hmac-sha256"}"#).unwrap();
        let c = Connection::read(path.to_str().unwrap()).unwrap();
        assert_eq!((c.ip.as_str(), c.key.as_slice()), ("127.0.0.1", &b"abc"[..]));
        assert_eq!((c.shell, c.iopub, c.stdin, c.control), (5001, 5002, 5003, 5004));
        assert_eq!(c.addr(c.shell), "127.0.0.1:5001");
        std::fs::write(&path, r#"{"ip":"127.0.0.1","iopub_port":5002}"#).unwrap();
        assert!(Connection::read(path.to_str().unwrap()).is_err(), "no shell port");
        let _ = std::fs::remove_file(&path);
        assert!(Connection::read(path.to_str().unwrap()).is_err(), "no file");
    }
}

#[cfg(test)]
mod handshake_tests {
    use super::*;
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    /// A peer that sends its greeting and then says nothing more.
    #[test]
    fn a_kernel_stalling_in_the_handshake_is_an_error() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let peer = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut greeting = [0u8; 64];
            greeting[0] = 0xFF;
            greeting[9] = 0x7F;
            greeting[10] = 3;
            greeting[12..16].copy_from_slice(b"NULL");
            s.write_all(&greeting).unwrap();
            std::thread::sleep(Duration::from_secs(1)); // no READY
        });
        let started = Instant::now();
        let result = Socket::connect_within(&addr, "DEALER", None, Duration::from_millis(300));
        assert!(result.is_err(), "the handshake finished without READY");
        assert!(started.elapsed() < Duration::from_millis(900), "it waited {:?}", started.elapsed());
        peer.join().unwrap();
    }
}

#[cfg(test)]
mod signed_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};

    fn signed(key: &[u8], content: &str) -> Vec<Vec<u8>> {
        let (h, p, m) = (r#"{"msg_type":"stream","msg_id":"1"}"#, r#"{"msg_id":"0"}"#, "{}");
        let sig = if key.is_empty() { String::new() } else { hmac_hex(key, &[h.as_bytes(), p.as_bytes(), m.as_bytes(), content.as_bytes()]) };
        [&b"<IDS|MSG>"[..], sig.as_bytes(), h.as_bytes(), p.as_bytes(), m.as_bytes(), content.as_bytes()].iter().map(|x| x.to_vec()).collect()
    }

    #[test]
    fn only_messages_signed_with_the_key_get_through() {
        let key = b"a1b2c3-key";
        let good = signed(key, r#"{"text":"hi"}"#);
        assert_eq!(parse_signed(&good, key).unwrap().2.get("text").str(), Some("hi"));
        assert!(parse_message(&good, key).is_some());

        let mut tampered = good.clone();
        tampered[5] = br#"{"text":"rm -rf"}"#.to_vec();
        assert!(parse_signed(&tampered, key).is_none(), "content changed after signing");
        assert!(parse_signed(&signed(b"other-key", r#"{"text":"hi"}"#), key).is_none(), "signed with another key");
        assert!(parse_signed(&signed(b"", r#"{"text":"hi"}"#), key).is_none(), "not signed at all");
        let mut upper = good.clone();
        upper[1] = String::from_utf8(good[1].clone()).unwrap().to_uppercase().into_bytes();
        assert!(parse_signed(&upper, key).is_none(), "the hex is lowercase");
        assert!(parse_signed(&good[..5], key).is_none(), "missing content");
        // With an empty key, as a connection file can say, nothing is signed.
        assert!(parse_signed(&signed(b"", "{}"), b"").is_some());
    }

    /// The kernel side of one connection: handshake, then each message as frames.
    fn serve(s: &mut TcpStream, messages: &[&[&[u8]]]) {
        let mut theirs = [0u8; 64];
        s.read_exact(&mut theirs).unwrap();
        let mut ours = [0u8; 64];
        ours[0] = 0xFF;
        ours[9] = 0x7F;
        ours[10] = 3;
        ours[12..16].copy_from_slice(b"NULL");
        s.write_all(&ours).unwrap();
        let mut head = [0u8; 2];
        s.read_exact(&mut head).unwrap();
        let mut ready = vec![0u8; head[1] as usize];
        s.read_exact(&mut ready).unwrap();
        let answer = b"\x05READY\x0bSocket-Type\x00\x00\x00\x03PUB";
        s.write_all(&[0x04, answer.len() as u8]).unwrap();
        s.write_all(answer).unwrap();
        for m in messages {
            for (i, part) in m.iter().enumerate() {
                s.write_all(&[if i + 1 < m.len() { 1 } else { 0 }, part.len() as u8]).unwrap();
                s.write_all(part).unwrap();
            }
        }
    }

    /// The connection drops while the kernel runs: reading goes on over a new one.
    #[test]
    fn a_dropped_channel_is_opened_again() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let kernel = std::thread::spawn(move || {
            let (mut first, _) = listener.accept().unwrap();
            serve(&mut first, &[&[b"one"]]);
            drop(first); // the connection drops
            let (mut second, _) = listener.accept().unwrap();
            serve(&mut second, &[&[b"two"]]);
            std::thread::sleep(std::time::Duration::from_millis(200));
        });
        let sock = Socket::connect(&addr, "SUB").unwrap();
        let (mut got, mut reconnects) = (Vec::new(), 0);
        read_channel(sock, || true, || {
            reconnects += 1;
            if reconnects > 1 { None } else { Socket::connect(&addr, "SUB").ok() }
        }, |parts| {
            got.push(parts[0].clone());
            got.len() < 2
        });
        kernel.join().unwrap();
        assert_eq!(got, vec![b"one".to_vec(), b"two".to_vec()]);
        assert_eq!(reconnects, 1);
    }

    /// When the kernel is gone, a dropped connection ends the reading.
    #[test]
    fn a_kernel_that_ended_is_not_called_again() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let kernel = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            serve(&mut s, &[&[b"last"]]);
        });
        let sock = Socket::connect(&addr, "SUB").unwrap();
        let mut got = Vec::new();
        read_channel(sock, || false, || panic!("tried to reconnect to an ended kernel"), |parts| {
            got.push(parts[0].clone());
            true
        });
        kernel.join().unwrap();
        assert_eq!(got, vec![b"last".to_vec()]);
    }
}
