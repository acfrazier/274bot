use crate::core::BufferedDigest as Sha256;
use crate::core::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;

struct Session {
    seen: [bool; 256],
    stamp: u64,
    marks: u64,
    metadata: Sha256,
}
impl Session {
    fn new() -> Self {
        Self {
            seen: [false; 256],
            stamp: 0,
            marks: 0,
            metadata: Sha256::new(),
        }
    }
    fn accept(&mut self, r: &Record, line: u64) -> Result<()> {
        let op = r.op;
        if op == b'#' {
            return Ok(());
        }
        need(
            self.seen[b'v' as usize] || (line == 1 && op == b'v'),
            "version must be first record",
        )?;
        if matches!(op, b'v' | b'X' | b'I' | b'x') {
            need(!self.seen[op as usize], "duplicate session metadata")?;
            self.seen[op as usize] = true;
        }
        if op == b'v' {
            need(r.nums == [0x10500, 3], "unsupported version")?;
        }
        if op == b'c' {
            need(r.nums[0] >= self.stamp, "nonmonotonic timestamp")?;
            self.stamp = r.nums[0];
            self.marks = add(self.marks, 1)?;
        }
        if matches!(op, b'X' | b'I' | b'R' | b'S') {
            self.metadata.update([op]);
            if matches!(op, b'X' | b'S') {
                self.metadata.update((r.bytes.len() as u64).to_be_bytes());
                self.metadata.update(r.bytes);
            } else {
                self.metadata
                    .update((r.nums.len() as u64 * 8).to_be_bytes());
                for n in &r.nums {
                    self.metadata.update(n.to_be_bytes());
                }
            }
        }
        Ok(())
    }
    fn finish(&self, raw: bool) -> Result<()> {
        need(
            [b'v', b'X', b'I'].iter().all(|x| self.seen[*x as usize])
                && (!raw || self.seen[b'x' as usize]),
            "missing direct-preload metadata",
        )
    }
}
pub fn raw_pass(e: &Value, g: &mut Guard) -> Result<Value> {
    let mut pointers = Map::new();
    let mut s = Session::new();
    let mut events = Sha256::new();
    let mut traces_hash = Sha256::new();
    let (
        mut traces,
        mut total,
        mut allocations,
        mut frees,
        mut unknown,
        mut temporary,
        mut last,
        mut high,
    ) = (0, 0, 0, 0, 0, 0, 0, 0);
    let mut stream = Input::open(e, g.limits.get("raw"), g.limits.get("line") as usize)?;
    let mut data = vec![];
    while let Some((line, _)) = stream.next(&mut data, g)? {
        let r = parse(&data, true, g.limits.get("line") as usize)?;
        s.accept(&r, line)?;
        let f = &r.nums;
        match r.op {
            b't' => {
                need(f[1] <= traces, "raw parent reference")?;
                traces = add(traces, 1)?;
                g.limits.count("traces", traces as usize)?;
                emit(&mut traces_hash, b't', f);
            }
            b'+' => {
                let (size, trace, ptr) = (f[0], f[1], f[2]);
                need(ptr != 0, "raw duplicate-live/null pointer")?;
                need(trace <= traces, "raw trace reference")?;
                g.limits.count("pointers", pointers.len() + 1)?;
                need(
                    pointers.insert((ptr, 0), (size, trace), g)?,
                    "raw duplicate-live/null pointer",
                )?;
                high = high.max(pointers.len());
                total = add(total, size)?;
                allocations = add(allocations, 1)?;
                last = ptr;
                emit(&mut events, b'+', &[size, trace]);
            }
            b'-' => {
                let temp = f[0] == last;
                last = 0;
                if let Some((size, trace)) = pointers.remove((f[0], 0), g)? {
                    total = sub(total, size)?;
                    frees = add(frees, 1)?;
                    temporary = add(temporary, temp as u64)?;
                    emit(&mut events, b'-', &[size, trace]);
                } else {
                    unknown = add(unknown, 1)?;
                }
            }
            b'c' => emit(&mut events, b'c', f),
            _ => {}
        }
    }
    s.finish(true)?;
    let mut sum = 0;
    for (size, _) in pointers.values() {
        g.pulse(0)?;
        sum = add(sum, size)?;
    }
    need(sum == total, "raw final sum")?;
    let lines = stream.lines;
    stream.finish(g)?;
    Ok(
        json!({"bytes":total,"count":pointers.len(),"allocations":allocations,"frees":frees,"unknown_frees":unknown,"temporary":temporary,"max_live_pointers":high,"events":digest(events),"traces":traces,"trace_digest":digest(traces_hash),"metadata":digest(s.metadata),"lines":lines}),
    )
}
#[derive(Default)]
pub struct Arena<T> {
    pub offsets: Vec<(usize, usize)>,
    pub data: Vec<T>,
}
impl<T: Clone> Arena<T> {
    fn new() -> Self {
        Self {
            offsets: vec![(0, 0)],
            data: vec![],
        }
    }
    fn push(&mut self, v: &[T]) {
        self.offsets.push((self.data.len(), v.len()));
        self.data.extend_from_slice(v);
    }
    pub fn get(&self, i: u64) -> Result<&[T]> {
        let (o, n) = *self.offsets.get(i as usize).ok_or("table reference")?;
        Ok(&self.data[o..o + n])
    }
}
pub struct Tables {
    pub strings: Arena<u8>,
    pub ips: Arena<u64>,
    pub traces: Vec<[u64; 2]>,
    pub desc: Vec<[u64; 4]>,
    pub trace_calls: Vec<u64>,
    keys: Map,
    pub suppressions: Vec<Vec<u8>>,
}
impl Tables {
    pub fn new() -> Self {
        Self {
            strings: Arena::new(),
            ips: Arena::new(),
            traces: vec![[0, 0]],
            desc: vec![],
            trace_calls: vec![0],
            keys: Map::new(),
            suppressions: vec![],
        }
    }
    pub fn string(&self, n: u64) -> Result<&[u8]> {
        self.strings.get(n)
    }
    pub fn ip(&self, n: u64) -> Result<&[u64]> {
        self.ips.get(n)
    }
    pub fn define(&mut self, r: &Record, g: &mut Guard) -> Result<()> {
        let f = &r.nums;
        match r.op {
            b's' => {
                g.limits
                    .count("string_bytes", self.strings.data.len() + r.bytes.len())?;
                g.limits.count("strings", self.strings.offsets.len())?;
                self.strings.push(r.bytes);
            }
            b'i' => {
                need(f[1] < self.strings.offsets.len() as u64, "module reference")?;
                for i in (2..f.len()).step_by(3) {
                    g.pulse(0)?;
                    need(
                        f[i] < self.strings.offsets.len() as u64,
                        "function reference",
                    )?;
                    if i + 1 < f.len() {
                        need(
                            f[i + 1] < self.strings.offsets.len() as u64,
                            "file reference",
                        )?;
                    }
                }
                g.limits.count("ips", self.ips.offsets.len())?;
                self.ips.push(f);
            }
            b't' => {
                need(
                    f[0] < self.ips.offsets.len() as u64 && f[1] < self.traces.len() as u64,
                    "trace IP/parent reference",
                )?;
                g.limits.count("traces", self.traces.len())?;
                self.traces.push([f[0], f[1]]);
                self.trace_calls.push(0);
            }
            b'a' => {
                need(
                    f[1] < self.traces.len() as u64,
                    "descriptor trace reference",
                )?;
                g.limits.count("descriptors", self.desc.len() + 1)?;
                need(
                    self.keys.insert((f[0], f[1]), (0, 0), g)?,
                    "duplicate size/trace descriptor",
                )?;
                self.desc.push([f[0], f[1], 0, 0]);
            }
            b'S' => self.suppressions.push(r.bytes.to_vec()),
            _ => return Err("definition opcode"),
        }
        Ok(())
    }
    pub fn stack(&self, mut trace: u64, g: &mut Guard) -> Result<Vec<u64>> {
        let mut frames = vec![];
        while trace != 0 {
            g.pulse(0)?;
            need(
                frames.len() < g.limits.get("depth") as usize,
                "stack depth cap",
            )?;
            let [ip, parent] = *self.traces.get(trace as usize).ok_or("trace reference")?;
            frames.push(ip);
            trace = parent;
        }
        Ok(frames)
    }
}
#[derive(Clone, Default)]
pub struct Snapshot {
    pub rows: BTreeMap<u64, [u64; 3]>,
    pub descriptors: Vec<[u64; 6]>,
    pub bytes: u64,
    pub count: u64,
}
impl Snapshot {
    pub fn value(&self) -> Value {
        json!({"rows":self.rows,"descriptors":self.descriptors,"bytes":self.bytes,"count":self.count})
    }
}
fn snapshot(t: &Tables, total: u64, count: u64, g: &mut Guard) -> Result<Snapshot> {
    let mut out = Snapshot {
        bytes: total,
        count,
        ..Default::default()
    };
    let (mut sum, mut nsum) = (0, 0);
    for (index, [size, trace, n, calls]) in t.desc.iter().enumerate() {
        g.pulse(0)?;
        if *n == 0 {
            continue;
        }
        let v = mul(*n, *size)?;
        let r = out.rows.entry(*trace).or_insert([0, 0, 0]);
        r[0] = add(r[0], v)?;
        r[1] = add(r[1], *n)?;
        r[2] = t.trace_calls[*trace as usize];
        out.descriptors
            .push([index as u64, *trace, *size, *n, *calls, v]);
        sum = add(sum, v)?;
        nsum = add(nsum, *n)?;
    }
    need(sum == total && nsum == count, "snapshot sums")?;
    Ok(out)
}
pub fn interpreted(
    e: &Value,
    t: &mut Tables,
    request: Option<u64>,
    checkpoints: Option<&BTreeMap<String, u64>>,
    g: &mut Guard,
) -> Result<(Value, BTreeMap<String, Snapshot>)> {
    let mut s = Session::new();
    let (mut events, mut th) = (Sha256::new(), Sha256::new());
    let mut stream = Input::open(
        e,
        g.limits.get("interpreted"),
        g.limits.get("line") as usize,
    )?;
    let (mut total, mut count, mut allocations, mut frees, mut temporary, mut last, mut high) =
        (0, 0, 0, 0, 0, 0, 0);
    let mut peak = json!({"line":0,"offset":0,"timestamp_ms":0,"bytes":0,"count":0});
    let (mut mark, mut requested, mut next) = (Value::Null, Value::Null, Value::Null);
    // Retain only the final event position, not a fresh allocated JSON object
    // for every allocation/free. Serialize once at the existing result boundary.
    let mut last_event: Option<(u64, u64, u64)> = None;
    let mut defs = [0usize; 256];
    for x in [b's', b'i', b't'] {
        defs[x as usize] = 1;
    }
    let mut snapshots = BTreeMap::new();
    if checkpoints.is_some() {
        for d in &mut t.desc {
            g.pulse(0)?;
            d[2] = 0;
            d[3] = 0;
        }
        for c in &mut t.trace_calls {
            g.pulse(0)?;
            *c = 0;
        }
    }
    let mut data = vec![];
    while let Some((line, offset)) = stream.next(&mut data, g)? {
        let r = parse(&data, false, g.limits.get("line") as usize)?;
        s.accept(&r, line)?;
        let f = &r.nums;
        let op = r.op;
        if matches!(op, b's' | b'i' | b't' | b'a' | b'S') {
            if checkpoints.is_none() {
                t.define(&r, g)?;
            } else {
                let i = defs[op as usize];
                let equal = match op {
                    b's' => t.string(i as u64)? == r.bytes,
                    b'i' => t.ip(i as u64)? == f,
                    b't' => t.traces.get(i).is_some_and(|x| x.as_slice() == f),
                    b'a' => t.desc.get(i).is_some_and(|x| &x[..2] == f),
                    b'S' => t.suppressions.get(i).is_some_and(|x| x == r.bytes),
                    _ => false,
                };
                need(equal, "second pass definition mismatch")?;
            }
            defs[op as usize] += 1;
            if op == b't' {
                let addr = if f[0] != 0 { t.ip(f[0])?[0] } else { 0 };
                emit(&mut th, b't', &[addr, f[1]]);
            }
        } else if matches!(op, b'+' | b'-') {
            let i = f[0];
            need(
                i < defs[b'a' as usize] as u64,
                "descriptor reference before use",
            )?;
            let d = &mut t.desc[i as usize];
            let (size, trace) = (d[0], d[1]);
            if op == b'+' {
                d[2] = add(d[2], 1)?;
                d[3] = add(d[3], 1)?;
                t.trace_calls[trace as usize] = add(t.trace_calls[trace as usize], 1)?;
                allocations = add(allocations, 1)?;
                total = add(total, size)?;
                count = add(count, 1)?;
                high = high.max(d[2]);
                last = i;
                if total > peak["bytes"].as_u64().unwrap() {
                    peak = json!({"line":line,"offset":offset,"timestamp_ms":s.stamp,"bytes":total,"count":count});
                }
            } else {
                need(d[2] > 0, "descriptor underflow")?;
                d[2] -= 1;
                total = sub(total, size)?;
                count = sub(count, 1)?;
                frees = add(frees, 1)?;
                temporary = add(temporary, (i == last) as u64)?;
                last = 0;
            }
            last_event = Some((line, offset, s.stamp));
            emit(&mut events, op, &[size, trace]);
        } else if op == b'c' {
            mark = json!({"line":line,"offset":offset,"timestamp_ms":s.stamp,"ordinal":s.marks,"bytes":total,"count":count});
            emit(&mut events, b'c', &[s.stamp]);
            if let Some(req) = request {
                if s.stamp <= req {
                    requested = mark.clone();
                } else if next.is_null() {
                    next = mark.clone();
                }
            }
        }
        if let Some(cp) = checkpoints {
            let names: Vec<_> = cp
                .iter()
                .filter(|(_, p)| **p == line)
                .map(|(n, _)| n.clone())
                .collect();
            if !names.is_empty() {
                let snap = snapshot(t, total, count, g)?;
                for n in names {
                    snapshots.insert(n, snap.clone());
                }
            }
        }
    }
    s.finish(false)?;
    need(!mark.is_null(), "no actual timestamp mark")?;
    if let Some(req) = request {
        need(
            req <= mark["timestamp_ms"].as_u64().unwrap() && !requested.is_null(),
            "requested time outside actual marks",
        )?;
    }
    if let Some(cp) = checkpoints {
        snapshots.insert("eof".into(), snapshot(t, total, count, g)?);
        if cp["peak"] == 0 {
            snapshots.insert("peak".into(), Snapshot::default());
        }
    }
    let eof = json!({"line":stream.lines,"offset":stream.offset,"bytes":total,"count":count});
    let sha = stream.finish(g)?;
    let mut definitions = serde_json::Map::new();
    for op in [b's', b'i', b't', b'a', b'S'] {
        if defs[op as usize] > 0 {
            definitions.insert((op as char).to_string(), json!(defs[op as usize]));
        }
    }
    let last_event = last_event.map(
        |(line, offset, stamp)| json!({"line":line,"offset":offset,"previous_timestamp_ms":stamp}),
    );
    Ok((
        json!({"peak":peak,"last_mark":mark,"requested":requested,"next_mark":next,"last_event":last_event,"eof":eof,"events":digest(events),"metadata":digest(s.metadata),"trace_digest":digest(th),"allocations":allocations,"frees":frees,"temporary":temporary,"max_multiplicity":high,"definitions":definitions,"sha256":sha}),
        snapshots,
    ))
}
pub const NEW: [&[u8]; 4] = [
    b"operator new(unsigned long)",
    b"operator new[](unsigned long)",
    b"operator new(unsigned int)",
    b"operator new[](unsigned int)",
];
const STOP: [&[u8]; 3] = [
    b"main",
    b"__libc_start_main",
    b"__static_initialization_and_destruction_0",
];
pub fn pretty(s: &[u8], g: &mut Guard) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut depth = 0i64;
    for &c in s {
        g.pulse(1)?;
        if matches!(c, b'<' | b'>') && out.len() >= 8 {
            let mut suffix = b"operator".to_vec();
            if out.last() == Some(&c) {
                suffix.push(c);
            }
            if out.ends_with(&suffix) {
                out.push(c);
                continue;
            }
        }
        if c == b'<' {
            depth += 1;
            if depth == 1 {
                out.push(c);
            }
        } else if c == b'>' {
            depth -= 1;
        }
        if depth == 0 {
            out.push(c);
        }
    }
    Ok(out)
}
fn basename(s: &[u8]) -> &[u8] {
    s.rsplit(|c| *c == b'/').next().unwrap()
}
pub fn canonical(t: &Tables, trace: u64, g: &mut Guard) -> Result<Vec<u8>> {
    if trace == 0 {
        return Ok(b"??".to_vec());
    }
    let mut pieces = vec![];
    let mut total = 0usize;
    for id in t.stack(trace, g)? {
        if id == 0 {
            break;
        }
        let ip = t.ip(id)?;
        let fun = if ip.len() > 2 { t.string(ip[2])? } else { b"" };
        if NEW.contains(&fun) {
            continue;
        }
        let mut s = if ip.len() > 2 && ip[2] != 0 {
            pretty(fun, g)?
        } else {
            format!("0x{:x}", ip[0]).into_bytes()
        };
        if ip.len() >= 5 && ip[3] != 0 {
            s.extend_from_slice(b" (");
            s.extend_from_slice(basename(t.string(ip[3])?));
            s.push(b')');
        }
        s.push(b';');
        for i in (5..ip.len()).step_by(3) {
            g.pulse(0)?;
            s.extend(pretty(t.string(ip[i])?, g)?);
            s.extend_from_slice(b" (");
            s.extend_from_slice(basename(t.string(ip[i + 1])?));
            s.extend_from_slice(b");");
            need(
                total + s.len() <= g.limits.get("line") as usize,
                "rendered stack cap",
            )?;
        }
        total += s.len();
        need(total <= g.limits.get("line") as usize, "rendered stack cap")?;
        pieces.push(s);
        if STOP.contains(&fun) {
            break;
        }
    }
    let mut out = Vec::with_capacity(total);
    for p in pieces.iter().rev() {
        g.pulse(p.len())?;
        out.extend_from_slice(p);
    }
    Ok(out)
}
pub fn reconcile(e: &Value, t: &Tables, s: &Snapshot, g: &mut Guard) -> Result<Value> {
    let mut expected = BTreeMap::<Vec<u8>, u64>::new();
    for (trace, row) in &s.rows {
        g.pulse(0)?;
        if row[0] > 0 {
            let k = canonical(t, *trace, g)?;
            let n = expected.entry(k).or_default();
            *n = add(*n, row[0])?;
        }
    }
    let mut stream = Input::open(e, g.limits.get("peak"), g.limits.get("line") as usize)?;
    let mut data = vec![];
    let (mut total, mut rows, mut positive) = (0, 0, 0);
    while stream.next(&mut data, g)?.is_some() {
        let line = &data[..data.len() - 1];
        let i = line
            .iter()
            .rposition(|c| *c == b' ')
            .ok_or("canonical cost syntax")?;
        let token = &line[i + 1..];
        need(
            !token.is_empty() && token.len() <= 19 && token.iter().all(u8::is_ascii_digit),
            "canonical cost syntax",
        )?;
        let cost = checked(
            text(token)?
                .parse::<u64>()
                .map_err(|_| "signed aggregate overflow/underflow")?,
        )?;
        rows = add(rows, 1)?;
        total = add(total, cost)?;
        if cost > 0 {
            positive = add(positive, 1)?;
            let n = expected
                .get_mut(&line[..i])
                .ok_or("canonical positive stack mismatch")?;
            need(*n >= cost, "canonical positive stack mismatch")?;
            *n -= cost;
        }
    }
    stream.finish(g)?;
    for n in expected.values() {
        g.pulse(0)?;
        need(*n == 0, "canonical peak mismatch")?;
    }
    need(total == s.bytes, "canonical peak mismatch")?;
    Ok(json!({"rows":rows,"positive_rows":positive,"bytes":total}))
}
pub struct Analysis {
    pub raw: Value,
    pub first: Value,
    pub snapshots: BTreeMap<String, Snapshot>,
    pub tables: Tables,
    pub comparison: Value,
}
pub fn analyze(entries: &Value, request: Option<u64>, g: &mut Guard) -> Result<Analysis> {
    g.next(1)?;
    let raw = raw_pass(&entries["raw"], g)?;
    g.next(2)?;
    let mut t = Tables::new();
    let (first, _) = interpreted(&entries["interpreted"], &mut t, request, None, g)?;
    for key in ["events", "metadata", "trace_digest", "allocations", "frees"] {
        need(raw[key] == first[key], "raw conversion mismatch")?;
    }
    need(
        raw["traces"] == json!(t.traces.len() - 1),
        "raw trace count mismatch",
    )?;
    need(
        raw["bytes"] == first["eof"]["bytes"] && raw["count"] == first["eof"]["count"],
        "raw final population mismatch",
    )?;
    let mut cp = BTreeMap::new();
    for n in ["peak", "last_mark"] {
        cp.insert(n.into(), first[n]["line"].as_u64().unwrap());
    }
    if request.is_some() {
        cp.insert(
            "requested".into(),
            first["requested"]["line"].as_u64().unwrap(),
        );
    }
    g.next(3)?;
    let (second, snapshots) = interpreted(&entries["interpreted"], &mut t, request, Some(&cp), g)?;
    need(first == second, "interpreted passes differ")?;
    let comparison = reconcile(&entries["peak"], &t, &snapshots["peak"], g)?;
    Ok(Analysis {
        raw,
        first,
        snapshots,
        tables: t,
        comparison,
    })
}
