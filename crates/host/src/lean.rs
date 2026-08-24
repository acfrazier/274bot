//! Lean channel: a socket + Isaac + packet buffer that cold-logins
//! (opcode 16) and drains the server stream without a `Client`, a World,
//! caches, or ifaces. The host runs one per extra bot; snapshot decode is
//! intentionally thin (pid, scene state, REBUILD_NORMAL origin tile).
//!
//! The handshake is the 274 cold-login path verbatim minus the client
//! shell: probe 14 + login-server byte, server seed, RSA login block, then
//! a response-2 grant. [`Lean::pump`] consumes whatever frames are buffered
//! by `SERVER_PROT_SIZES` and skips the opcodes a lean channel does not
//! understand; it never blocks and never sends a keepalive.

use std::io;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use client::client::{ClientConfig, LoginError};
use client::io::{ClientStream, Isaac, Packet, ServerProt, SERVER_PROT_SIZES};
use client::util::JString;
use client::{LOGIN_RSAE, LOGIN_RSAN};
use num_bigint::BigUint;

/// What a lean channel knows about the game after login / pump.
pub struct LeanSnapshot {
    pub pid: i32,
    pub tile_x: i32,
    pub tile_z: i32,
    pub scene_state: i32,
}

/// Login response codes, socket failures, and malformed frames.
#[derive(Debug)]
pub enum LeanError {
    /// Socket read/write failure (connection lost, EOF, timeout).
    Io(io::Error),
    /// Login rejected: the 274 response code plus the Java title lines.
    Login(LoginError),
    /// A frame larger than the 5000-byte read buffer (the Java client's
    /// AIOOBE → T2); the channel is unusable from here.
    FrameTooLarge { ptype: i32, psize: i32 },
}

/// The 274 client version sent in the login wrapper (`Client.java`).
const CLIENT_VERSION: i32 = 274;

/// Login-server response 1 ("try again") retries, each after Java's 2 s
/// wait. The real client recurses forever; a lean login must not hang the
/// host thread.
const LOGIN_RETRIES: usize = 5;

/// JAG archives whose CRC values go out in the login wrapper (slots 1-8;
/// slot 0 has no pack file and stays 0), the same list and layout as the
/// 274 client.
const JAG_FILES: [&str; 8] = [
    "title",
    "config",
    "interface",
    "media",
    "versionlist",
    "textures",
    "wordenc",
    "sounds",
];

pub struct Lean {
    stream: ClientStream,
    /// Isaac for inbound ptype decode, seeded with the login seed + 50
    /// (the outbound seed + 50 the server encrypts with).
    random_in: Isaac,
    /// Outbound packet builder; unused by the pump (sends are a later
    /// task) but kept so the handshake state the channel needs is here.
    #[allow(dead_code)] // held for the send path (Task 2+)
    out: Packet,
    /// 5000-byte read buffer; the header byte lives at data[0] and is
    /// overwritten by the payload, exactly like `Client::in`.
    incoming: Packet,
    /// `-1` = a new frame header is due; otherwise the decoded ptype /
    /// size of a frame the previous pump left partial.
    ptype: i32,
    psize: i32,
    snapshot: LeanSnapshot,
}

impl Lean {
    /// Cold-login handshake (opcode 16): probe 14 + login-server byte,
    /// seed, RSA login block, then a response-2 grant. No `Client`, no
    /// cache unpack, no `prepare_game`. Response 1 retries on a fresh
    /// connection after Java's 2 s wait, up to [`LOGIN_RETRIES`] times.
    pub fn login(
        config: &ClientConfig,
        user: &str,
        pass: &str,
        uid: i32,
    ) -> Result<Self, LeanError> {
        let mut attempts = 0;
        loop {
            match Self::login_attempt(config, user, pass, uid) {
                Ok(lean) => return Ok(lean),
                Err(LeanError::Login(ref e)) if e.code == 1 && attempts < LOGIN_RETRIES => {
                    attempts += 1;
                    thread::sleep(Duration::from_millis(2000));
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Drain every complete frame currently buffered: Isaac-decode `ptype`,
    /// size it from `SERVER_PROT_SIZES`, skip unknown opcodes, and update
    /// the snapshot for the packets a lean channel understands. Partial
    /// frames resume on the next call; nothing blocks.
    pub fn pump(&mut self) -> Result<(), LeanError> {
        loop {
            let Some(ptype) = self.read_frame()? else {
                return Ok(());
            };
            self.apply_packet(ptype);
        }
    }

    pub fn snapshot(&self) -> &LeanSnapshot {
        &self.snapshot
    }

    fn login_attempt(
        config: &ClientConfig,
        user: &str,
        pass: &str,
        uid: i32,
    ) -> Result<Lean, LeanError> {
        let mut stream = ClientStream::connect(&config.host, config.port).map_err(LeanError::Io)?;
        let userhash = JString::to_userhash(user);
        let login_server = ((userhash >> 16) & 0x1f) as i32;

        let mut out = Packet::alloc(1);
        out.p1(14);
        out.p1(login_server);
        stream.write(out.data(), 2).map_err(LeanError::Io)?;

        for _ in 0..8 {
            stream.read().map_err(LeanError::Io)?;
        }
        let mut response = stream.read().map_err(LeanError::Io)?;

        // Isaac for inbound decode; a non-zero probe response never
        // exchanged a seed, so the grant below would read raw ptypes (no
        // real 274 login server grants on the probe).
        let mut random_in = Isaac::new(&[0, 0, 0, 0]);
        let mut incoming = Packet::alloc(1);

        if response == 0 {
            stream
                .read_bytes(incoming.data_mut(), 0, 8)
                .map_err(LeanError::Io)?;
            incoming.pos = 0;
            let login_seed = incoming.g8();
            let mut seed = [
                login_random(),
                login_random(),
                (login_seed >> 32) as i32,
                (login_seed & 0xffff_ffff) as i32,
            ];

            out.pos = 0;
            out.p1(10);
            out.p4(seed[0]);
            out.p4(seed[1]);
            out.p4(seed[2]);
            out.p4(seed[3]);
            out.p4(uid);
            out.pjstr(user);
            out.pjstr(pass);
            let n = BigUint::from_str(LOGIN_RSAN).unwrap();
            let e = BigUint::from_str(LOGIN_RSAE).unwrap();
            out.rsaenc(&n, &e);

            let mut loginout = Packet::alloc(1);
            loginout.p1(16);
            loginout.p1((out.pos + 36 + 1 + 1 + 2) as i32);
            loginout.p1(255);
            loginout.p2(CLIENT_VERSION);
            loginout.p1(if config.lowmem { 1 } else { 0 });
            for checksum in jag_checksums(&config.cache_dir) {
                loginout.p4(checksum);
            }
            loginout.pdata(out.data(), 0, out.pos);

            out.random = Some(Isaac::new(&seed));
            for s in seed.iter_mut() {
                *s = s.wrapping_add(50);
            }
            random_in = Isaac::new(&seed);

            stream
                .write(loginout.data(), loginout.pos)
                .map_err(LeanError::Io)?;
            response = stream.read().map_err(LeanError::Io)?;
        }

        if response == 2 {
            let _ = stream.read().map_err(LeanError::Io)?; // staff level
            let _ = stream.read().map_err(LeanError::Io)?; // mouse tracking
            return Ok(Lean {
                stream,
                random_in,
                out,
                incoming,
                ptype: -1,
                psize: 0,
                snapshot: LeanSnapshot {
                    pid: 0,
                    tile_x: 0,
                    tile_z: 0,
                    scene_state: 0,
                },
            });
        }

        Err(LeanError::Login(login_error(response)))
    }

    /// One frame's header + payload from the stream, or `None` when no
    /// complete frame is buffered. Mirrors `Client::read_packet`: `ptype`
    /// and `psize` persist across calls so a partial frame resumes, and
    /// `available` is a peek so the reads never block.
    fn read_frame(&mut self) -> Result<Option<i32>, LeanError> {
        let mut available = self.stream.available().map_err(LeanError::Io)?;
        if available == 0 {
            return Ok(None);
        }

        if self.ptype == -1 {
            self.stream
                .read_bytes(self.incoming.data_mut(), 0, 1)
                .map_err(LeanError::Io)?;
            self.ptype = self.incoming.data()[0] as i32 & 0xff;
            self.ptype = self.ptype.wrapping_sub(self.random_in.next_int()) & 0xff;
            self.psize = SERVER_PROT_SIZES[self.ptype as usize];
            available -= 1;
        }

        if self.psize == -1 {
            if available <= 0 {
                return Ok(None);
            }
            self.stream
                .read_bytes(self.incoming.data_mut(), 0, 1)
                .map_err(LeanError::Io)?;
            self.psize = self.incoming.data()[0] as i32 & 0xff;
            available -= 1;
        }

        if self.psize == -2 {
            if available <= 1 {
                return Ok(None);
            }
            self.stream
                .read_bytes(self.incoming.data_mut(), 0, 2)
                .map_err(LeanError::Io)?;
            self.incoming.pos = 0;
            self.psize = self.incoming.g2();
            available -= 2;
        }

        if available < self.psize {
            return Ok(None);
        }

        // A length over the read buffer is the same AIOOBE the Java client
        // catches as a logic error.
        if self.psize as usize > self.incoming.length() {
            return Err(LeanError::FrameTooLarge {
                ptype: self.ptype,
                psize: self.psize,
            });
        }

        self.incoming.pos = 0;
        self.stream
            .read_bytes(self.incoming.data_mut(), 0, self.psize as usize)
            .map_err(LeanError::Io)?;
        let ptype = self.ptype;
        self.ptype = -1;
        Ok(Some(ptype))
    }

    /// Snapshot updates for the packets a lean channel decodes; everything
    /// else is consumed by size and skipped (no World, ifaces, or caches).
    fn apply_packet(&mut self, ptype: i32) {
        match ptype {
            ServerProt::UPDATE_PID => {
                self.snapshot.pid = self.incoming.g2();
                let _ = self.incoming.g1(); // members account
            }
            ServerProt::REBUILD_NORMAL => {
                let zone_x = self.incoming.g2();
                let zone_z = self.incoming.g2();
                // The client builds a scene here; a lean channel only
                // records the loading state and the build-area origin tile
                // (`map_build_base`). The player tile itself needs
                // local-player decode (a later task).
                self.snapshot.scene_state = 1;
                self.snapshot.tile_x = (zone_x - 6) * 8;
                self.snapshot.tile_z = (zone_z - 6) * 8;
            }
            _ => {}
        }
    }
}

/// The 274 response-code table, verbatim from `Client::login`.
fn login_error(response: i32) -> LoginError {
    let (mes1, mes2): (String, String) = match response {
        3 => (String::new(), "Invalid username or password.".into()),
        4 => (
            "Your account has been disabled.".into(),
            "Please check your message-centre for details.".into(),
        ),
        5 => (
            "Your account is already logged in.".into(),
            "Try again in 60 secs...".into(),
        ),
        6 => (
            "RuneScape has been updated!".into(),
            "Wrong RSA key - run tools/redeploy.sh and rebuild.".into(),
        ),
        7 => (
            "This world is full.".into(),
            "Please use a different world.".into(),
        ),
        8 => ("Unable to connect.".into(), "Login server offline.".into()),
        9 => (
            "Login limit exceeded.".into(),
            "Too many connections from your address.".into(),
        ),
        10 => ("Unable to connect.".into(), "Bad session id.".into()),
        11 => (String::new(), "Please try again.".into()),
        12 => (
            "You need a members account to login to this world.".into(),
            "Please subscribe, or use a different world.".into(),
        ),
        13 => (
            "Could not complete login.".into(),
            "Please try using a different world.".into(),
        ),
        14 => (
            "The server is being updated.".into(),
            "Please wait 1 minute and try again.".into(),
        ),
        16 => (
            "Login attempts exceeded.".into(),
            "Please wait 1 minute and try again.".into(),
        ),
        17 => (
            "You are standing in a members-only area.".into(),
            "To play on this world move to a free area first".into(),
        ),
        20 => (
            "Invalid loginserver requested".into(),
            "Please try using a different world.".into(),
        ),
        -1 => (
            "No response from server".into(),
            "Please try using a different world.".into(),
        ),
        _ => (
            "Unexpected server response".into(),
            "Please try using a different world.".into(),
        ),
    };
    LoginError {
        code: response,
        mes1,
        mes2,
    }
}

/// CRC of each JAG pack under `cache_dir`, 0 for a missing file — the
/// client's empty-cache fallback (`read_jag_checksums`).
fn jag_checksums(cache_dir: &str) -> [i32; 9] {
    let mut checksum = [0i32; 9];
    for (slot, name) in JAG_FILES.iter().enumerate() {
        if let Ok(bytes) = std::fs::read(format!("{cache_dir}/{name}")) {
            checksum[slot + 1] = Packet::getcrc(&bytes, 0, bytes.len());
        }
    }
    checksum
}

/// Stand-in for Java `(int)(Math.random() * 99999999)`: a non-negative
/// value below 100_000_000 for the login Isaac seed, same as the client.
fn login_random() -> i32 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut x = nanos ^ COUNTER.fetch_add(0x9e37_79b9_7f4a_7c15, Ordering::Relaxed);
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    let r = x.wrapping_mul(0x2545_f491_4f6c_dd1d);
    ((r >> 32) % 99_999_999) as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use client::io::ClientProt;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    /// A lean channel over a real socket pair with a known Isaac seed, so
    /// the test can encode the frames it feeds `pump`. The seed a real
    /// login exchanges is RSA-hidden, so a fake server cannot know it.
    fn lean_pair() -> (Lean, std::net::TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || listener.accept().unwrap().0);
        let stream = ClientStream::connect(&addr.ip().to_string(), addr.port()).unwrap();
        let srv = server.join().unwrap();
        // seed [0, 0, 0, 0] + the client's +50 inbound offset.
        let lean = Lean {
            stream,
            random_in: Isaac::new(&[50; 4]),
            out: Packet::alloc(1),
            incoming: Packet::alloc(1),
            ptype: -1,
            psize: 0,
            snapshot: LeanSnapshot {
                pid: 0,
                tile_x: 0,
                tile_z: 0,
                scene_state: 0,
            },
        };
        (lean, srv)
    }

    /// Encode one frame with the shared inbound Isaac: the wire opcode byte
    /// is `ptype + next_int()` (the pump decodes `raw - next_int()`).
    fn encode(ptype: i32, payload: &[u8], enc: &mut Isaac) -> Vec<u8> {
        let mut frame = Vec::with_capacity(1 + payload.len());
        frame.push(ptype.wrapping_add(enc.next_int()) as u8);
        frame.extend_from_slice(payload);
        frame
    }

    /// Pump until `ok` sees its condition or the cap passes, so a frame
    /// still in the loopback pipe on the first peek is not a flake (the
    /// pump itself never blocks). At most 100 tries, 1 ms apart.
    fn pump_until(lean: &mut Lean, ok: impl FnMut(&LeanSnapshot) -> bool) {
        let mut ok = ok;
        for _ in 0..100 {
            if ok(lean.snapshot()) {
                return;
            }
            lean.pump().unwrap();
            thread::sleep(Duration::from_millis(1));
        }
        panic!("pump never reached the expected snapshot state");
    }

    #[test]
    fn pump_rebuild_normal_updates_snapshot() {
        let (mut lean, mut srv) = lean_pair();
        let mut enc = Isaac::new(&[50; 4]);
        // Zone (48, 49) → build-area origin tiles (48-6)*8, (49-6)*8.
        let frame = encode(ServerProt::REBUILD_NORMAL, &[0, 48, 0, 49], &mut enc);
        srv.write_all(&frame).unwrap();

        pump_until(&mut lean, |s| s.scene_state == 1);
        assert_eq!(lean.snapshot().tile_x, 336);
        assert_eq!(lean.snapshot().tile_z, 344);
        assert_eq!(lean.snapshot().pid, 0);
    }

    #[test]
    fn pump_skips_unknown_and_resumes_partial_frames() {
        let (mut lean, mut srv) = lean_pair();
        let mut enc = Isaac::new(&[50; 4]);

        // UPDATE_PID (size 3): pid update.
        srv.write_all(&encode(ServerProt::UPDATE_PID, &[0, 7, 1], &mut enc))
            .unwrap();
        pump_until(&mut lean, |s| s.pid == 7);
        assert_eq!(lean.snapshot().scene_state, 0);

        // A zero-size unknown opcode: consumed and skipped.
        srv.write_all(&encode(4, &[], &mut enc)).unwrap();
        lean.pump().unwrap();
        assert_eq!(lean.snapshot().pid, 7);
        assert_eq!(lean.snapshot().scene_state, 0);

        // REBUILD_NORMAL (size 4) split across two writes: the first pump
        // must return Ok with the frame still partial, not block or apply.
        let rebuild = encode(ServerProt::REBUILD_NORMAL, &[0, 48, 0, 49], &mut enc);
        srv.write_all(&rebuild[..3]).unwrap();
        lean.pump().unwrap();
        assert_eq!(
            lean.snapshot().scene_state, 0,
            "partial frame must not apply"
        );

        // The rest of the payload completes the frame on a later pump.
        srv.write_all(&rebuild[3..]).unwrap();
        pump_until(&mut lean, |s| s.scene_state == 1);
        assert_eq!(lean.snapshot().tile_x, 336);
        assert_eq!(lean.snapshot().tile_z, 344);

        // An empty stream is a no-op, not an error.
        lean.pump().unwrap();
        assert_eq!(lean.snapshot().scene_state, 1);
    }

    #[test]
    fn lean_pump_does_not_emit_no_timeout() {
        let (mut lean, mut srv) = lean_pair();
        // Mirror post-login state: the outbound Isaac (raw seed; inbound
        // is +50) so a game_loop-style keepalive would be deterministic
        // on the wire and provably absent below.
        lean.out.random = Some(Isaac::new(&[0; 4]));

        for _ in 0..80 {
            lean.pump().unwrap();
        }

        // Writes drain through the ClientStream writer thread; poll a
        // moment so a mistaken keepalive cannot hide in the queue.
        let mut recv = Vec::new();
        srv.set_nonblocking(true).unwrap();
        for _ in 0..20 {
            let mut buf = [0u8; 1024];
            loop {
                match srv.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => recv.extend_from_slice(&buf[..n]),
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) => panic!("server read failed: {e}"),
                }
            }
            if !recv.is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }

        // The frame a game_loop keepalive puts on the wire: p1_enc of
        // NO_TIMEOUT with the outbound Isaac above.
        let mut enc = Isaac::new(&[0; 4]);
        let no_timeout = ClientProt::NO_TIMEOUT.id.wrapping_add(enc.next_int()) as u8;
        assert!(
            !recv.contains(&no_timeout),
            "pump sent a NO_TIMEOUT frame: {recv:?}"
        );
        assert!(
            recv.is_empty(),
            "pump must be read-only without host-supplied out bytes: {recv:?}"
        );
    }
}
