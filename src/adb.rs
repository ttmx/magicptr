//! Just enough of the ADB wire protocol to push a file and hold one stream open.

use std::fs;
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

use base64::Engine;
use rsa::pkcs8::{DecodePrivateKey, EncodePrivateKey, LineEnding};
use rsa::traits::PublicKeyParts;
use rsa::{BigUint, Pkcs1v15Sign, RsaPrivateKey};

const A_CNXN: u32 = 0x4e58_4e43;
const A_AUTH: u32 = 0x4854_5541;
const A_OPEN: u32 = 0x4e45_504f;
const A_OKAY: u32 = 0x5941_4b4f;
const A_CLSE: u32 = 0x4553_4c43;
const A_WRTE: u32 = 0x4554_5257;
const AUTH_TOKEN: u32 = 1;
const AUTH_SIGNATURE: u32 = 2;
const AUTH_RSAPUBLICKEY: u32 = 3;
const MAX_PAYLOAD: usize = 4096;
const SHA1_DIGEST_INFO: [u8; 15] = [
    0x30, 0x21, 0x30, 0x09, 0x06, 0x05, 0x2b, 0x0e, 0x03, 0x02, 0x1a, 0x05, 0x00, 0x04, 0x14,
];

const REPLY_TIMEOUT: Duration = Duration::from_secs(5);
const PROMPT_TIMEOUT: Duration = Duration::from_secs(60);
const ACK_TIMEOUT: Duration = Duration::from_secs(4);

fn error(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::Other, message.into())
}

/// Loads the key the box is asked to trust, creating it on first use.
pub fn load_or_create_key(path: &Path) -> io::Result<RsaPrivateKey> {
    if let Ok(pem) = fs::read_to_string(path) {
        return RsaPrivateKey::from_pkcs8_pem(&pem).map_err(|e| error(format!("{}: {e}", path.display())));
    }
    crate::log!("creating a key for the box to trust, this can take a minute");
    let key = RsaPrivateKey::new(&mut rand::rngs::OsRng, 2048).map_err(|e| error(e.to_string()))?;
    let pem = key.to_pkcs8_pem(LineEnding::LF).map_err(|e| error(e.to_string()))?;
    fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?.write_all(pem.as_bytes())?;
    Ok(key)
}

/// The public key in the form adbd stores: a precomputed Montgomery layout, base64 encoded.
fn android_public_key(key: &RsaPrivateKey) -> Vec<u8> {
    let n = key.n();
    let size = key.size();
    let padded = |value: BigUint| {
        let mut bytes = value.to_bytes_le();
        bytes.resize(size, 0);
        bytes
    };
    let n0 = u32::from_le_bytes(padded(n.clone())[..4].try_into().unwrap());
    let mut inverse = n0; // Newton's iteration for n0's inverse modulo 2^32
    for _ in 0..5 {
        inverse = inverse.wrapping_mul(2u32.wrapping_sub(n0.wrapping_mul(inverse)));
    }
    let rr = (BigUint::from(1u8) << (size * 16)) % n;
    let exponent = u32::from_le_bytes(padded(key.e().clone())[..4].try_into().unwrap());

    let mut blob = Vec::new();
    blob.extend((size as u32 / 4).to_le_bytes());
    blob.extend(inverse.wrapping_neg().to_le_bytes());
    blob.extend(padded(n.clone()));
    blob.extend(padded(rr));
    blob.extend(exponent.to_le_bytes());
    let mut text = base64::engine::general_purpose::STANDARD.encode(blob).into_bytes();
    text.extend(b" magicptr@lgtv\0");
    text
}

pub struct Adb {
    sock: TcpStream,
    buf: Vec<u8>,
    local_id: u32,
    remote_id: u32,
    /// When the write the box has yet to acknowledge was sent.
    awaiting_ack: Option<Instant>,
    /// Lines queued behind it; the flag marks ones a newer line of the same kind may replace.
    pending: Vec<(String, bool)>,
}

impl AsRawFd for Adb {
    fn as_raw_fd(&self) -> RawFd {
        self.sock.as_raw_fd()
    }
}

impl Adb {
    pub fn connect(address: &str, key: &RsaPrivateKey) -> io::Result<Adb> {
        let address = address.to_socket_addrs()?.next().ok_or_else(|| error("no such host"))?;
        let sock = TcpStream::connect_timeout(&address, REPLY_TIMEOUT)?;
        sock.set_nodelay(true)?;
        // Notice a box that restarted within about 20 s, rather than at the next pointer movement.
        for (level, option, value) in [
            (libc::SOL_SOCKET, libc::SO_KEEPALIVE, 1),
            (libc::IPPROTO_TCP, libc::TCP_KEEPIDLE, 10),
            (libc::IPPROTO_TCP, libc::TCP_KEEPINTVL, 5),
            (libc::IPPROTO_TCP, libc::TCP_KEEPCNT, 2),
        ] {
            let value: libc::c_int = value;
            unsafe { libc::setsockopt(sock.as_raw_fd(), level, option, &value as *const _ as *const _, 4) };
        }
        let mut adb = Adb { sock, buf: Vec::new(), local_id: 0, remote_id: 0, awaiting_ack: None, pending: Vec::new() };
        adb.handshake(key)?;
        Ok(adb)
    }

    fn send(&mut self, command: u32, arg0: u32, arg1: u32, data: &[u8]) -> io::Result<()> {
        let checksum: u32 = data.iter().map(|&byte| byte as u32).sum();
        let mut message = Vec::with_capacity(24 + data.len());
        for word in [command, arg0, arg1, data.len() as u32, checksum, !command] {
            message.extend(word.to_le_bytes());
        }
        message.extend(data);
        self.sock.write_all(&message)
    }

    /// The next complete message in the buffer, if there is one.
    fn parse(&mut self) -> Option<(u32, u32, u32, Vec<u8>)> {
        if self.buf.len() < 24 {
            return None;
        }
        let word = |i: usize| u32::from_le_bytes(self.buf[i * 4..i * 4 + 4].try_into().unwrap());
        let (command, arg0, arg1, length) = (word(0), word(1), word(2), word(3) as usize);
        if self.buf.len() < 24 + length {
            return None;
        }
        let data = self.buf[24..24 + length].to_vec();
        self.buf.drain(..24 + length);
        Some((command, arg0, arg1, data))
    }

    fn fill(&mut self) -> io::Result<()> {
        let mut chunk = [0; 65536];
        match self.sock.read(&mut chunk)? {
            0 => Err(error("box closed the connection")),
            n => Ok(self.buf.extend(&chunk[..n])),
        }
    }

    /// The next message, waiting up to `timeout` for it.
    fn receive(&mut self, timeout: Duration) -> io::Result<(u32, u32, u32, Vec<u8>)> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(message) = self.parse() {
                return Ok(message);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(error("timed out waiting for the box"));
            }
            self.sock.set_read_timeout(Some(left))?;
            match self.fill() {
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
                other => other?,
            }
        }
    }

    /// Waits for `command` addressed to the current stream, skipping leftovers of earlier ones.
    fn expect(&mut self, command: u32) -> io::Result<u32> {
        loop {
            let (got, arg0, arg1, _) = self.receive(REPLY_TIMEOUT)?;
            if arg1 == self.local_id {
                return if got == command { Ok(arg0) } else { Err(error("box refused the request")) };
            }
        }
    }

    fn handshake(&mut self, key: &RsaPrivateKey) -> io::Result<()> {
        self.send(A_CNXN, 0x0100_0000, MAX_PAYLOAD as u32, b"host::\0")?;
        let mut signed = false;
        loop {
            let (command, arg0, _, data) = self.receive(if signed { PROMPT_TIMEOUT } else { REPLY_TIMEOUT })?;
            if command == A_CNXN {
                return Ok(());
            }
            if command != A_AUTH || arg0 != AUTH_TOKEN {
                return Err(error(format!("unexpected reply {command:#010x} during handshake")));
            }
            if !signed {
                let padding = Pkcs1v15Sign { hash_len: Some(data.len()), prefix: SHA1_DIGEST_INFO.into() };
                let signature = key.sign(padding, &data).map_err(|e| error(e.to_string()))?;
                self.send(A_AUTH, AUTH_SIGNATURE, 0, &signature)?;
                signed = true;
            } else {
                crate::log!("box does not know this TV yet: accept the debugging prompt on its screen");
                self.send(A_AUTH, AUTH_RSAPUBLICKEY, 0, &android_public_key(key))?;
            }
        }
    }

    /// Opens a stream to `service`, which becomes the one `write` and `pump` talk to.
    pub fn open(&mut self, service: &str) -> io::Result<()> {
        self.local_id += 1;
        self.send(A_OPEN, self.local_id, 0, format!("{service}\0").as_bytes())?;
        self.remote_id = self.expect(A_OKAY)?;
        // From here on the main loop owns the socket and must never stall on it.
        self.sock.set_read_timeout(Some(Duration::from_millis(500)))?;
        self.sock.set_write_timeout(Some(Duration::from_millis(500)))
    }

    /// Writes `data` to the file at `path` on the box.
    pub fn push(&mut self, path: &str, data: &[u8]) -> io::Result<()> {
        self.open(&format!("exec:cat > {path}"))?;
        for chunk in data.chunks(MAX_PAYLOAD) {
            self.send(A_WRTE, self.local_id, self.remote_id, chunk)?;
            self.expect(A_OKAY)?;
        }
        self.send(A_CLSE, self.local_id, self.remote_id, b"")?;
        self.expect(A_CLSE).map(|_| ())
    }

    pub fn write(&mut self, line: String, replaceable: bool) -> io::Result<()> {
        match self.pending.last_mut() {
            Some(last) if replaceable && last.1 => last.0 = line,
            _ => self.pending.push((line, replaceable)),
        }
        self.flush()
    }

    fn flush(&mut self) -> io::Result<()> {
        // ADB allows one unacknowledged write per stream, so batch what queued up meanwhile.
        if self.awaiting_ack.is_none() && !self.pending.is_empty() {
            let data: String = self.pending.drain(..).map(|(line, _)| line + "\n").collect();
            self.send(A_WRTE, self.local_id, self.remote_id, data.as_bytes())?;
            self.awaiting_ack = Some(Instant::now());
        }
        Ok(())
    }

    /// Handles whatever the box sent, returning the lines the stream printed; call when readable.
    pub fn pump(&mut self) -> io::Result<Vec<String>> {
        self.fill()?;
        let mut lines = Vec::new();
        while let Some((command, _, arg1, data)) = self.parse() {
            if arg1 != self.local_id {
                continue;
            }
            match command {
                A_OKAY => {
                    self.awaiting_ack = None;
                    self.flush()?;
                }
                A_WRTE => {
                    self.send(A_OKAY, self.local_id, self.remote_id, b"")?;
                    lines.extend(String::from_utf8_lossy(&data).lines().map(str::to_owned));
                }
                A_CLSE => return Err(error(format!("helper exited on the box: {}", lines.join(" ")))),
                _ => {}
            }
        }
        Ok(lines)
    }

    /// Fails if the box has stopped acknowledging writes.
    pub fn check(&self) -> io::Result<()> {
        match self.awaiting_ack {
            Some(sent) if sent.elapsed() > ACK_TIMEOUT => Err(error("box stopped acknowledging writes")),
            _ => Ok(()),
        }
    }
}
