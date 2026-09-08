//! Shared bounded storage, grammar, streaming identity and child guards.
#[path = "batched.rs"]
mod batched;
#[cfg(test)]
#[path = "migration_tests.rs"]
mod migration_tests;
#[cfg(test)]
#[path = "core_tests.rs"]
mod tests;
#[cfg(test)]
static OPEN_COUNTS: [AtomicUsize; 3] = [
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
];
pub use batched::BufferedDigest;
#[path = "sort.rs"]
mod sort;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
pub use sort::guarded_sort;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    collections::hash_map::RandomState,
    fs::{File, OpenOptions},
    hash::{BuildHasher, Hash, Hasher},
    io::{BufRead, BufReader},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
};
pub type Result<T> = std::result::Result<T, &'static str>;
pub fn need(b: bool, e: &'static str) -> Result<()> {
    if b { Ok(()) } else { Err(e) }
}
pub const MIB: usize = 1 << 20;
pub static USED: AtomicUsize = AtomicUsize::new(0);
pub static HIGH: AtomicUsize = AtomicUsize::new(0);
pub static CAP: AtomicUsize = AtomicUsize::new(256 * MIB);
// Charge every live allocation, including JSON, selected output, sort workspace,
// hash slots and transient old+new reallocations. More conservative than merely
// charging tables. Reserve allocator metadata/alignment per allocation as well.
pub struct Charged;
fn charge_size(l: Layout) -> usize {
    l.size()
        .checked_add(l.align())
        .and_then(|n| n.checked_add(32))
        .unwrap_or(usize::MAX)
}
fn allocation_failure(message: &[u8]) -> ! {
    unsafe {
        libc::write(1, message.as_ptr().cast(), message.len());
        libc::_exit(1)
    }
}
fn acquire(n: usize) {
    let old = USED.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| {
        v.checked_add(n)
            .filter(|x| *x <= CAP.load(Ordering::SeqCst))
    });
    match old {
        Ok(v) => {
            HIGH.fetch_max(v + n, Ordering::SeqCst);
        }
        Err(_) => allocation_failure(b"{\"error\":\"table byte cap\"}\n"),
    }
}
unsafe impl GlobalAlloc for Charged {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        acquire(charge_size(l));
        let p = unsafe { System.alloc(l) };
        if p.is_null() {
            allocation_failure(b"{\"error\":\"allocation failure\"}\n")
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) };
        USED.fetch_sub(charge_size(l), Ordering::SeqCst);
    }
    unsafe fn realloc(&self, p: *mut u8, old: Layout, size: usize) -> *mut u8 {
        let new = Layout::from_size_align(size, old.align()).unwrap();
        // Explicit allocate/copy/free makes transient charging independent of
        // the system allocator's in-place realloc decisions.
        let q = unsafe { self.alloc(new) };
        unsafe {
            std::ptr::copy_nonoverlapping(p, q, old.size().min(size));
            self.dealloc(p, old)
        };
        q
    }
}
#[derive(Clone)]
pub struct Limits(pub Value);
impl Limits {
    pub fn new(over: &Value) -> Result<Self> {
        let mut v = json!({"table_bytes":256*MIB,"traces":2000000,"ips":500000,"descriptors":1000000,"pointers":2000000,"strings":200000,"string_bytes":64*MIB,"line":MIB,"depth":512,"raw":3*1024*MIB,"interpreted":1024*MIB,"peak":64*MIB,"output":32*MIB,"scratch":64*MIB,"cpu":180,"wall":300,"rss":512*MIB,"address":768*MIB,"admission_memory":768*MIB,"admission_disk":1024*MIB,"free_disk":512*MIB});
        for (k, x) in over.as_object().ok_or("invalid limit override")? {
            let n = x.as_u64().ok_or("invalid limit override")?;
            need(
                n > 0 && v.get(k).and_then(Value::as_u64).is_some_and(|m| n <= m),
                "invalid limit override",
            )?;
            v[k] = x.clone();
        }
        Ok(Self(v))
    }
    pub fn get(&self, k: &str) -> u64 {
        self.0[k].as_u64().unwrap()
    }
    pub fn count(&self, k: &str, n: usize) -> Result<()> {
        need(
            n as u64 <= self.get(k),
            match k {
                "strings" => "strings cap",
                "ips" => "ips cap",
                "traces" => "traces cap",
                "descriptors" => "descriptors cap",
                "pointers" => "pointers cap",
                "string_bytes" => "string_bytes cap",
                _ => "table cap",
            },
        )
    }
}
pub fn checked(n: u64) -> Result<u64> {
    need(n <= i64::MAX as u64, "signed aggregate overflow/underflow")?;
    Ok(n)
}
pub fn add(a: u64, b: u64) -> Result<u64> {
    checked(
        a.checked_add(b)
            .ok_or("signed aggregate overflow/underflow")?,
    )
}
pub fn sub(a: u64, b: u64) -> Result<u64> {
    a.checked_sub(b)
        .ok_or("signed aggregate overflow/underflow")
}
pub fn mul(a: u64, b: u64) -> Result<u64> {
    checked(
        a.checked_mul(b)
            .ok_or("signed aggregate overflow/underflow")?,
    )
}
pub fn hex(b: &[u8]) -> Result<u64> {
    need(!b.is_empty() && b.len() <= 16, "invalid uint64 hex")?;
    let mut n = 0u64;
    for c in b {
        let x = match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            _ => return Err("invalid uint64 hex"),
        };
        n = n
            .checked_mul(16)
            .and_then(|n| n.checked_add(x as u64))
            .ok_or("invalid uint64 hex")?;
    }
    Ok(n)
}
pub fn text(b: &[u8]) -> Result<&str> {
    std::str::from_utf8(b).map_err(|_| "invalid UTF-8 string")
}
#[derive(Debug, PartialEq)]
pub struct Record<'a> {
    pub op: u8,
    pub nums: Vec<u64>,
    pub bytes: &'a [u8],
}
pub fn parse(line: &[u8], raw: bool, cap: usize) -> Result<Record<'_>> {
    need(line.len() <= cap, "line cap")?;
    need(line.ends_with(b"\n"), "truncated record (missing newline)")?;
    let line = &line[..line.len() - 1];
    let mut r = Record {
        op: b'#',
        nums: Vec::new(),
        bytes: &[],
    };
    if line.is_empty() || line[0] == b'#' {
        return Ok(r);
    }
    need(line.len() == 1 || line[1] == b' ', "record separator")?;
    r.op = line[0];
    let p = if line.len() > 1 { &line[2..] } else { &[] };
    if r.op == b'A' {
        return Err("attach unsupported");
    }
    if matches!(r.op, b's' | b'x' | b'm') {
        need(
            if raw { r.op != b's' } else { r.op == b's' },
            "wrong stream opcode",
        )?;
        let i = p
            .iter()
            .position(|c| *c == b' ')
            .ok_or("truncated sized string")?;
        let n = hex(&p[..i])?;
        need(n <= (p.len() - i - 1) as u64, "truncated sized string")?;
        r.bytes = &p[i + 1..i + 1 + n as usize];
        text(r.bytes)?;
        need(
            !r.bytes.contains(&0),
            "NUL string unsupported by canonical printer",
        )?;
        let tail = &p[i + 1 + n as usize..];
        if r.op == b'm' && r.bytes != b"-" {
            need(tail.starts_with(b" "), "missing module ranges")?;
            for x in tail[1..].split(|c| *c == b' ') {
                r.nums.push(hex(x)?)
            }
            need(
                r.nums.len() >= 3 && r.nums.len() % 2 == 1,
                "module range arity",
            )?;
            for pair in r.nums[1..].chunks_exact(2) {
                r.nums[0]
                    .checked_add(pair[0])
                    .and_then(|n| n.checked_add(pair[1]))
                    .ok_or("module address overflow")?;
            }
        } else {
            need(tail.is_empty(), "extra string fields")?;
        }
        return Ok(r);
    }
    if matches!(r.op, b'X' | b'S') {
        need(line.len() >= 2 && !p.contains(&0), "metadata shape")?;
        r.bytes = p;
        return Ok(r);
    }
    let arity = match r.op {
        b'v' | b'I' | b't' => 2,
        b'R' | b'c' | b'-' => 1,
        b'+' => {
            if raw {
                3
            } else {
                1
            }
        }
        b'a' if !raw => 2,
        b'i' if !raw => 0,
        _ => return Err("unknown opcode"),
    };
    for x in p.split(|c| *c == b' ') {
        r.nums.push(hex(x)?)
    }
    if arity == 0 {
        let n = r.nums.len();
        need(n == 2 || n == 3 || (n >= 5 && (n - 2) % 3 == 0), "IP arity")?;
    } else {
        need(r.nums.len() == arity, "numeric arity")?;
    }
    Ok(r)
}
pub fn emit(h: &mut BufferedDigest, op: u8, values: &[u64]) {
    h.update([op]);
    for n in values {
        h.update(n.to_be_bytes())
    }
}
pub trait FinishDigest {
    fn finish(self) -> String;
}
impl FinishDigest for Sha256 {
    fn finish(self) -> String {
        format!("{:x}", self.finalize())
    }
}
impl FinishDigest for BufferedDigest {
    fn finish(self) -> String {
        BufferedDigest::finish(self)
    }
}
pub fn digest(h: impl FinishDigest) -> String {
    h.finish()
}
pub fn hash(b: &[u8]) -> String {
    digest(Sha256::new_with_prefix(b))
}
pub fn mono() -> f64 {
    clock(libc::CLOCK_MONOTONIC)
}
pub fn cpu() -> f64 {
    clock(libc::CLOCK_PROCESS_CPUTIME_ID)
}
fn clock(id: libc::clockid_t) -> f64 {
    let mut t = std::mem::MaybeUninit::<libc::timespec>::uninit();
    assert_eq!(unsafe { libc::clock_gettime(id, t.as_mut_ptr()) }, 0);
    let t = unsafe { t.assume_init() };
    t.tv_sec as f64 + t.tv_nsec as f64 / 1e9
}
pub fn rss() -> u64 {
    let mut r = std::mem::MaybeUninit::<libc::rusage>::uninit();
    assert_eq!(
        unsafe { libc::getrusage(libc::RUSAGE_SELF, r.as_mut_ptr()) },
        0
    );
    let n = unsafe { r.assume_init() }.ru_maxrss as u64;
    if cfg!(target_os = "macos") {
        n
    } else {
        n * 1024
    }
}
pub struct Guard {
    pub limits: Limits,
    pub phase: u64,
    pub start: f64,
    pub start_cpu: f64,
    last: f64,
    work: usize,
    bytes: usize,
    pub measurements: Vec<Value>,
    pub line: u64,
    pub offset: u64,
    pub records: u64,
    pub protocol: bool,
}
impl Guard {
    pub fn new(limits: Limits, protocol: bool) -> Self {
        let now = mono();
        Self {
            limits,
            phase: 0,
            start: now,
            start_cpu: cpu(),
            last: now,
            work: 0,
            bytes: 0,
            measurements: vec![],
            line: 0,
            offset: 0,
            records: 0,
            protocol,
        }
    }
    pub fn check(&mut self) -> Result<()> {
        let now = mono();
        need(
            now - self.start <= self.limits.get("wall") as f64,
            "wall guard",
        )?;
        need(
            cpu() - self.start_cpu <= self.limits.get("cpu") as f64,
            "CPU guard",
        )?;
        need(rss() <= self.limits.get("rss"), "RSS guard")?;
        self.last = now;
        Ok(())
    }
    pub fn pulse(&mut self, bytes: usize) -> Result<()> {
        self.work += 1;
        self.bytes += bytes;
        if self.work >= 1024 || self.bytes >= 65536 {
            self.work = 0;
            self.bytes = 0;
            if mono() - self.last >= 0.1 {
                self.check()?;
            }
        }
        Ok(())
    }
    pub fn next(&mut self, n: u64) -> Result<()> {
        self.check()?;
        need(n == self.phase + 1 && n <= 4, "phase sequence")?;
        let now = mono();
        let c = cpu();
        if self.phase > 0 {
            self.measurements.push(json!({"phase":self.phase,"wall_s":now-self.start,"cpu_s":c-self.start_cpu,"cumulative_peak_rss_bytes":rss(),"charged_capacity_bytes":USED.load(Ordering::SeqCst),"high_charged_capacity_bytes":HIGH.load(Ordering::SeqCst),"line":self.line,"offset":self.offset,"records":self.records}));
        }
        self.phase = n;
        self.start = now;
        self.start_cpu = c;
        self.line = 0;
        self.offset = 0;
        self.records = 0;
        if self.protocol {
            send(&json!({"phase":n,"wall_start":now,"cpu_start":c}))?;
        }
        Ok(())
    }
}
pub fn send(v: &Value) -> Result<()> {
    use std::io::Write;
    let stdout = std::io::stdout();
    let mut o = stdout.lock();
    serde_json::to_writer(&mut o, v).map_err(|_| "protocol write")?;
    o.write_all(b"\n")
        .and_then(|_| o.flush())
        .map_err(|_| "protocol write")
}
pub fn entry(v: &Value, cap: u64) -> Result<(&str, u64, &str)> {
    let o = v.as_object().ok_or("input entry schema")?;
    need(
        o.len() == 3
            && o.contains_key("path")
            && o.contains_key("bytes")
            && o.contains_key("sha256"),
        "input entry schema",
    )?;
    let p = v["path"].as_str().ok_or("input path")?;
    let n = v["bytes"].as_u64().ok_or("input size cap")?;
    need(n > 0 && n <= cap, "input size cap")?;
    let h = v["sha256"].as_str().ok_or("hash format")?;
    need(
        h.len() == 64 && h.bytes().all(|c| matches!(c,b'0'..=b'9'|b'a'..=b'f')),
        "hash format",
    )?;
    Ok((p, n, h))
}
fn identity(m: &std::fs::Metadata) -> (u64, u64, u64, i64, i64, i64, i64) {
    (
        m.dev(),
        m.ino(),
        m.size(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec(),
    )
}
pub struct Input {
    reader: BufReader<File>,
    before: std::fs::Metadata,
    size: u64,
    expected: String,
    pub lines: u64,
    pub offset: u64,
    pub sha: Sha256,
    hashed_remaining: usize,
    cap: usize,
}
impl Input {
    pub fn open(v: &Value, cap: u64, line: usize) -> Result<Self> {
        let (p, n, h) = entry(v, cap)?;
        #[cfg(test)]
        for (index, name) in ["raw", "interpreted", "peak"].iter().enumerate() {
            if Path::new(p).file_name().is_some_and(|s| s == *name) {
                OPEN_COUNTS[index].fetch_add(1, Ordering::SeqCst);
            }
        }
        let f = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(p)
            .map_err(|_| "input open")?;
        let before = f.metadata().map_err(|_| "input stat")?;
        need(before.is_file() && before.len() == n, "input identity/size")?;
        Ok(Self {
            reader: BufReader::with_capacity(65536, f),
            before,
            size: n,
            expected: h.to_owned(),
            lines: 0,
            offset: 0,
            sha: Sha256::new(),
            hashed_remaining: 0,
            cap: line,
        })
    }
    pub fn next(&mut self, line: &mut Vec<u8>, g: &mut Guard) -> Result<Option<(u64, u64)>> {
        line.clear();
        let start = self.offset;
        loop {
            let chunk = self.reader.fill_buf().map_err(|_| "input read")?;
            if chunk.is_empty() {
                if line.is_empty() {
                    return Ok(None);
                }
                return Err("truncated record");
            }
            // Hash each newly filled bounded read buffer once, not each line.
            // Success still requires consumption of exactly the frozen size.
            if self.hashed_remaining == 0 {
                self.sha.update(chunk);
                self.hashed_remaining = chunk.len();
            }
            let n = chunk
                .iter()
                .position(|c| *c == b'\n')
                .map_or(chunk.len(), |i| i + 1);
            need(line.len() + n <= self.cap, "line cap")?;
            let end = chunk[n - 1] == b'\n';
            line.extend_from_slice(&chunk[..n]);
            self.hashed_remaining -= n;
            self.offset = self.offset.checked_add(n as u64).ok_or("input grew")?;
            need(self.offset <= self.size, "input grew")?;
            self.reader.consume(n);
            g.pulse(n)?;
            if end {
                break;
            }
        }
        self.lines += 1;
        g.line = self.lines;
        g.offset = self.offset;
        g.records += 1;
        g.pulse(0)?;
        Ok(Some((self.lines, start)))
    }
    pub fn finish(self, g: &mut Guard) -> Result<String> {
        g.check()?;
        let after = self.reader.get_ref().metadata().map_err(|_| "input stat")?;
        need(
            identity(&self.before) == identity(&after),
            "input changed during pass",
        )?;
        let h = digest(self.sha);
        need(
            self.offset == self.size && h == self.expected,
            "input hash mismatch",
        )?;
        Ok(h)
    }
}
#[derive(Clone, Copy, Default)]
struct Slot {
    state: u8,
    key: (u64, u64),
    value: (u64, u64),
}
pub struct Map {
    slots: Vec<Slot>,
    len: usize,
    tombs: usize,
    seed: RandomState,
    pub high_capacity: usize,
}
impl Map {
    pub fn new() -> Self {
        Self {
            slots: vec![],
            len: 0,
            tombs: 0,
            seed: RandomState::new(),
            high_capacity: 0,
        }
    }
    pub fn len(&self) -> usize {
        self.len
    }
    fn index(&self, key: (u64, u64), g: &mut Guard) -> Result<(usize, bool)> {
        let mut h = self.seed.build_hasher();
        key.hash(&mut h);
        let mut i = h.finish() as usize & (self.slots.len() - 1);
        let mut tomb = None;
        for _ in 0..4096 {
            g.pulse(0)?;
            let s = self.slots[i];
            if s.state == 0 {
                return Ok((tomb.unwrap_or(i), false));
            }
            if s.state == 2 {
                if tomb.is_none() {
                    tomb = Some(i)
                }
            } else if s.key == key {
                return Ok((i, true));
            }
            i = (i + 1) & (self.slots.len() - 1);
        }
        Err("map collision probe cap")
    }
    fn rehash(&mut self, n: usize, g: &mut Guard) -> Result<()> {
        let fresh = vec![Slot::default(); n];
        let old = std::mem::replace(&mut self.slots, fresh);
        self.tombs = 0;
        self.high_capacity = self.high_capacity.max(n);
        for s in old.iter().filter(|s| s.state == 1) {
            let (i, found) = self.index(s.key, g)?;
            need(!found, "map duplicate internal")?;
            self.slots[i] = *s;
        }
        Ok(())
    }
    pub fn insert(&mut self, key: (u64, u64), value: (u64, u64), g: &mut Guard) -> Result<bool> {
        if self.slots.is_empty() {
            self.rehash(16, g)?;
        }
        if (self.len + self.tombs + 1) * 10 > self.slots.len() * 7 {
            let n = if (self.len + 1) * 10 > self.slots.len() * 7 {
                self.slots
                    .len()
                    .checked_mul(2)
                    .ok_or("map capacity overflow")?
            } else {
                self.slots.len()
            };
            self.rehash(n, g)?;
        }
        let (i, found) = self.index(key, g)?;
        if found {
            return Ok(false);
        }
        if self.slots[i].state == 2 {
            self.tombs -= 1;
        }
        self.slots[i] = Slot {
            state: 1,
            key,
            value,
        };
        self.len += 1;
        Ok(true)
    }
    pub fn remove(&mut self, key: (u64, u64), g: &mut Guard) -> Result<Option<(u64, u64)>> {
        if self.slots.is_empty() {
            return Ok(None);
        }
        let (i, found) = self.index(key, g)?;
        if !found {
            return Ok(None);
        }
        self.slots[i].state = 2;
        self.len -= 1;
        self.tombs += 1;
        Ok(Some(self.slots[i].value))
    }
    pub fn values(&self) -> impl Iterator<Item = (u64, u64)> + '_ {
        self.slots.iter().filter(|s| s.state == 1).map(|s| s.value)
    }
}
pub fn bounded(path: &Path, cap: usize) -> Result<Vec<u8>> {
    use std::io::Read;
    let f = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| "metadata open")?;
    let m = f.metadata().map_err(|_| "metadata stat")?;
    need(
        m.is_file() && m.len() <= cap as u64,
        "metadata file cap/type",
    )?;
    let mut b = Vec::new();
    f.take(cap as u64 + 1)
        .read_to_end(&mut b)
        .map_err(|_| "metadata read")?;
    need(b.len() <= cap, "metadata file cap")?;
    Ok(b)
}
