//! Catalogue metadata only. Script reads and compilation belong to script-js.

use std::io::{self, Write};
use std::path::{Component, PathBuf};
use std::sync::Arc;

use api::local_state::{
    CatalogueComponent, CatalogueReferenceError, CatalogueReferenceStore, FileCardReference,
    MAX_CATALOGUE_FILE_BYTES, MAX_CATALOGUE_FILE_ROWS, MAX_CATALOGUE_ROOT_BYTES,
};
use rusqlite::{params, Connection, OptionalExtension};

use super::sql::{Db, SqlFailure};
use super::*;

const DOMAIN: &str = "catalogue-references";
const PIECE: usize = 32 * 1024;
// Replacing the largest value includes its old pages, B-tree splits and ledgers.
const PAGES: u64 = 2 * (MAX_CATALOGUE_FILE_BYTES as u64 / sql::PAGE_SIZE) + 64;

pub(super) const SCHEMA: &str = "
CREATE TABLE catalogue_head (
    component INTEGER PRIMARY KEY CHECK (component BETWEEN 0 AND 2),
    version BLOB NOT NULL CHECK (length(version) = 8),
    digest BLOB NOT NULL CHECK (length(digest) = 32),
    bytes INTEGER NOT NULL CHECK (bytes BETWEEN 0 AND 4194304),
    rows INTEGER NOT NULL CHECK (rows BETWEEN 0 AND 190650)
) STRICT;
CREATE TABLE catalogue_piece (
    component INTEGER NOT NULL REFERENCES catalogue_head(component),
    position INTEGER NOT NULL CHECK (position BETWEEN 0 AND 127),
    body BLOB NOT NULL CHECK (length(body) BETWEEN 1 AND 32768),
    PRIMARY KEY (component, position)
) STRICT, WITHOUT ROWID;
";

#[derive(Clone, Copy)]
enum Part {
    Files = 0,
    Root = 1,
    Deferred = 2,
}

impl Part {
    fn source(self) -> &'static str {
        match self {
            Self::Files => "js-scripts.json",
            Self::Root => "rs2b0t-path",
            Self::Deferred => "rs2b0t-import",
        }
    }

    fn limit(self) -> usize {
        match self {
            Self::Root => MAX_CATALOGUE_ROOT_BYTES,
            Self::Files | Self::Deferred => MAX_CATALOGUE_FILE_BYTES,
        }
    }
}

#[derive(Clone)]
struct Packed {
    head: Head,
    body: Arc<[u8]>,
}

#[derive(Clone, Copy)]
struct Head {
    version: StoreVersion,
    digest: files::ContentDigest,
    bytes: usize,
    rows: usize,
}

struct Port {
    service: Arc<LocalStateService>,
}

impl LocalStateService {
    /// The catalogue port shares this service's single SQL owner and connection.
    pub fn catalogue_references(self: &Arc<Self>) -> Arc<dyn CatalogueReferenceStore> {
        Arc::new(Port {
            service: Arc::clone(self),
        })
    }
}

fn refusal(path: PathBuf, detail: impl std::fmt::Display) -> CatalogueReferenceError {
    CatalogueReferenceError::Refused {
        path,
        detail: Arc::from(format!("{detail}. Correct the source and restart, or move it aside explicitly before retrying. Nothing was overwritten")),
    }
}

fn state_error(root: &StateRoot, detail: impl std::fmt::Display) -> CatalogueReferenceError {
    CatalogueReferenceError::Refused {
        path: root.database_file(),
        detail: Arc::from(detail.to_string()),
    }
}

fn sql_error(error: rusqlite::Error) -> SqlFailure {
    sql::wait_of(&error).map_or_else(
        || sql::refused_by(error, "catalogue references"),
        SqlFailure::Wait,
    )
}

fn damaged() -> SqlFailure {
    SqlFailure::refused(
        FailureReason::Schema,
        "catalogue references have inconsistent rows; restore the paired state backup",
    )
}

impl Port {
    fn read<T: Send + 'static>(
        &self,
        part: Part,
        decode: impl Fn(Packed, &StateRoot) -> Result<T, CatalogueReferenceError> + Send + 'static,
    ) -> Result<T, CatalogueReferenceError> {
        let shared = Arc::clone(&self.service.shared);
        self.service
            .profile_sql("catalogue hydration", move |db| {
                let mut notices = Vec::new();
                let result = hydrate(db, part, &mut notices)?;
                for (level, line) in notices {
                    shared.notice(level, format_args!("{line}"));
                }
                let mut state = shared.state.lock();
                state.truncate.due |= db.wal_truncate_due();
                if let Ok(value) = &result {
                    state.clock = state.clock.max(Some(value.head.version));
                }
                drop(state);
                Ok(result.and_then(|packed| decode(packed, &db.root)))
            })
            .map_err(|error| state_error(&self.service.shared.root, error))?
    }

    fn save(
        &self,
        part: Part,
        expected: Option<StoreVersion>,
        body: Vec<u8>,
        rows: usize,
    ) -> Result<StoreVersion, CatalogueReferenceError> {
        let request = self.service.shared.minter.mint().map_err(|_| {
            CatalogueReferenceError::Unavailable("the request ID space is spent".into())
        })?;
        let shared = Arc::clone(&self.service.shared);
        let body: Arc<[u8]> = body.into();
        let digest = files::digest(&body);
        self.service
            .profile_sql("catalogue edit", move |db| {
                #[cfg(any(test, feature = "test-support"))]
                {
                    let (stall, fault) = {
                        let state = shared.state.lock();
                        (state.write_stall.clone(), state.write_fault.clone())
                    };
                    if let Some(stall) = stall {
                        stall();
                    }
                    if fault.is_some_and(|fault| fault()) {
                        return Ok(Err(state_error(
                            &db.root,
                            "the catalogue edit was not written: injected write failure",
                        )));
                    }
                }
                let clock = shared.state.lock().clock;
                let result = db.commit(
                    request,
                    clock,
                    None,
                    || !shared.state.lock().abandoned,
                    |conn| {
                        let head = read_head(conn, part)?;
                        Ok(sql::Loaded {
                            head: head.map_or(StoreVersion::INITIAL, |head| head.version),
                            observed: Some(head),
                        })
                    },
                    |head: &Option<Head>| {
                        if expected
                            .is_some_and(|version| head.is_none_or(|head| head.version != version))
                        {
                            return Err(ErrorCode::Conflict {
                                expected: expected.expect("checked expected version"),
                                found: head.map_or(StoreVersion::INITIAL, |head| head.version),
                            });
                        }
                        Ok(Staged {
                            part,
                            previous: *head,
                            body: Arc::clone(&body),
                            digest,
                            rows,
                        })
                    },
                )?;
                let (version, truncate_due) = match result {
                    sql::Commit::Committed {
                        durable,
                        truncate_due,
                        ..
                    } => {
                        let durable = durable.expect("a committed catalogue edit has a head");
                        shared.notice(
                            Level::Info,
                            format_args!(
                                "local state: catalogue edit component={} request={} version={}",
                                part.source(),
                                request.get(),
                                durable.version
                            ),
                        );
                        (durable.version, truncate_due)
                    }
                    sql::Commit::Unchanged { staged, .. } => (
                        staged
                            .previous
                            .expect("unchanged values have a head")
                            .version,
                        false,
                    ),
                    sql::Commit::Declined {
                        code: ErrorCode::Conflict { .. },
                        ..
                    } => return Ok(Err(CatalogueReferenceError::Conflict)),
                    sql::Commit::Declined { code, detail, .. } => {
                        return Ok(Err(state_error(
                            &db.root,
                            detail.unwrap_or_else(|| Arc::from(code.to_string())),
                        )))
                    }
                    sql::Commit::Unresolved { detail, .. } => {
                        return Ok(Err(state_error(&db.root, detail)))
                    }
                    sql::Commit::Abandoned => {
                        return Ok(Err(CatalogueReferenceError::Unavailable(
                            "the edit was cancelled at shutdown".into(),
                        )))
                    }
                };
                let mut state = shared.state.lock();
                state.clock = state.clock.max(Some(version));
                state.truncate.due |= truncate_due;
                Ok(Ok(version))
            })
            .map_err(|error| state_error(&self.service.shared.root, error))?
    }
}

impl CatalogueReferenceStore for Port {
    fn file_cards(
        &self,
    ) -> Result<CatalogueComponent<Arc<[FileCardReference]>>, CatalogueReferenceError> {
        self.read(Part::Files, |packed, root| {
            let rows = decode_files(&packed.body, packed.head.rows).ok_or_else(|| {
                state_error(
                    root,
                    "invalid file reference encoding; restore the paired state backup",
                )
            })?;
            Ok(CatalogueComponent {
                version: packed.head.version,
                value: rows.into(),
            })
        })
    }

    fn saved_root(&self) -> Result<CatalogueComponent<Option<PathBuf>>, CatalogueReferenceError> {
        self.read(Part::Root, |packed, root| {
            let text =
                std::str::from_utf8(&packed.body).map_err(|error| state_error(root, error))?;
            let value = if text.is_empty() {
                None
            } else {
                Some(checked_root(text).map_err(|reason| state_error(root, reason))?)
            };
            Ok(CatalogueComponent {
                version: packed.head.version,
                value,
            })
        })
    }

    fn import_deferred(&self) -> Result<CatalogueComponent<bool>, CatalogueReferenceError> {
        self.read(Part::Deferred, |packed, root| {
            let value = match packed.body.as_ref() {
                [0] => false,
                [1] => true,
                _ => {
                    return Err(state_error(
                        root,
                        "invalid defer choice; restore the paired state backup",
                    ))
                }
            };
            Ok(CatalogueComponent {
                version: packed.head.version,
                value,
            })
        })
    }

    fn save_file_cards(
        &self,
        expected: StoreVersion,
        rows: Arc<[FileCardReference]>,
    ) -> Result<StoreVersion, CatalogueReferenceError> {
        let body =
            encode_files(&rows).map_err(|reason| state_error(&self.service.shared.root, reason))?;
        self.save(Part::Files, Some(expected), body, rows.len())
    }

    fn save_root(
        &self,
        expected: Option<StoreVersion>,
        root: Option<PathBuf>,
    ) -> Result<StoreVersion, CatalogueReferenceError> {
        let text = root
            .as_ref()
            .map(|root| root.to_str().ok_or("catalogue root is not UTF-8"))
            .transpose()
            .map_err(|reason| state_error(&self.service.shared.root, reason))?
            .unwrap_or("");
        if root.is_some() {
            checked_root(text).map_err(|reason| state_error(&self.service.shared.root, reason))?;
        }
        if text.len() > MAX_CATALOGUE_ROOT_BYTES {
            return Err(state_error(
                &self.service.shared.root,
                "catalogue root exceeds 8192 bytes",
            ));
        }
        self.save(
            Part::Root,
            expected,
            text.as_bytes().to_vec(),
            usize::from(!text.is_empty()),
        )
    }

    fn save_deferred(
        &self,
        expected: Option<StoreVersion>,
        deferred: bool,
    ) -> Result<StoreVersion, CatalogueReferenceError> {
        self.save(Part::Deferred, expected, vec![u8::from(deferred)], 1)
    }
}

fn checked_root(text: &str) -> Result<PathBuf, &'static str> {
    let path = PathBuf::from(text);
    if !path.is_absolute() {
        return Err("not an absolute path");
    }
    if path
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err("the path contains `..`");
    }
    Ok(path)
}

struct Count(usize);
impl Write for Count {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        if self.0 > MAX_CATALOGUE_FILE_BYTES {
            return Err(io::Error::other("file references exceed 4194304 bytes"));
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn encode_files(rows: &[FileCardReference]) -> Result<Vec<u8>, String> {
    if rows.len() > MAX_CATALOGUE_FILE_ROWS {
        return Err(format!(
            "file references exceed {MAX_CATALOGUE_FILE_ROWS} rows"
        ));
    }
    serde_json::to_writer(Count(0), rows).map_err(|error| error.to_string())?;
    let bytes = rows
        .iter()
        .map(|row| 8 + row.name.len() + row.path.len())
        .sum();
    let mut body = Vec::with_capacity(bytes);
    for row in rows {
        body.extend_from_slice(&(row.name.len() as u32).to_be_bytes());
        body.extend_from_slice(&(row.path.len() as u32).to_be_bytes());
        body.extend_from_slice(row.name.as_bytes());
        body.extend_from_slice(row.path.as_bytes());
    }
    Ok(body)
}

fn decode_files(mut body: &[u8], rows: usize) -> Option<Vec<FileCardReference>> {
    if rows > MAX_CATALOGUE_FILE_ROWS || body.len() > MAX_CATALOGUE_FILE_BYTES {
        return None;
    }
    let mut result = Vec::with_capacity(rows);
    for _ in 0..rows {
        let lengths = body.get(..8)?;
        let name_len = u32::from_be_bytes(lengths[..4].try_into().ok()?) as usize;
        let path_len = u32::from_be_bytes(lengths[4..].try_into().ok()?) as usize;
        body = body.get(8..)?;
        let name = std::str::from_utf8(body.get(..name_len)?).ok()?.to_string();
        body = body.get(name_len..)?;
        let path = std::str::from_utf8(body.get(..path_len)?).ok()?.to_string();
        body = body.get(path_len..)?;
        result.push(FileCardReference { name, path });
    }
    body.is_empty().then_some(result)
}

fn read_head(conn: &Connection, part: Part) -> Result<Option<Head>, SqlFailure> {
    conn.query_row(
        "SELECT version,digest,bytes,rows FROM catalogue_head WHERE component=?1",
        [part as u8],
        |row| {
            let version: Vec<u8> = row.get(0)?;
            let digest: Vec<u8> = row.get(1)?;
            Ok(Head {
                version: StoreVersion::from_durable(
                    sql::decode_u64(&version).ok_or(rusqlite::Error::InvalidQuery)?,
                ),
                digest: digest
                    .try_into()
                    .map_err(|_| rusqlite::Error::InvalidQuery)?,
                bytes: row.get::<_, u32>(2)? as usize,
                rows: row.get::<_, u32>(3)? as usize,
            })
        },
    )
    .optional()
    .map_err(sql_error)
}

fn read_body(conn: &Connection, part: Part, head: Head) -> Result<Packed, SqlFailure> {
    if head.bytes > part.limit() || head.rows > MAX_CATALOGUE_FILE_ROWS {
        return Err(damaged());
    }
    let mut body = Vec::with_capacity(head.bytes);
    let mut stmt = conn
        .prepare("SELECT position,body FROM catalogue_piece WHERE component=?1 ORDER BY position")
        .map_err(sql_error)?;
    let mut rows = stmt.query([part as u8]).map_err(sql_error)?;
    let mut next = 0;
    while let Some(row) = rows.next().map_err(sql_error)? {
        let position = row.get::<_, u32>(0).map_err(sql_error)? as usize;
        let piece = row
            .get_ref(1)
            .map_err(sql_error)?
            .as_blob()
            .map_err(|_| damaged())?;
        if position != next || piece.len() > PIECE || body.len() + piece.len() > head.bytes {
            return Err(damaged());
        }
        body.extend_from_slice(piece);
        next += 1;
    }
    if body.len() != head.bytes || files::digest(&body) != head.digest {
        return Err(damaged());
    }
    Ok(Packed {
        head,
        body: body.into(),
    })
}

fn write_value(conn: &Connection, part: Part, head: Head, body: &[u8]) -> rusqlite::Result<()> {
    conn.execute("INSERT INTO catalogue_head (component,version,digest,bytes,rows) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(component) DO UPDATE SET version=excluded.version,digest=excluded.digest,bytes=excluded.bytes,rows=excluded.rows", params![part as u8, sql::be(head.version.get()), head.digest.as_slice(), head.bytes as u32, head.rows as u32])?;
    conn.execute(
        "DELETE FROM catalogue_piece WHERE component=?1",
        [part as u8],
    )?;
    let mut stmt =
        conn.prepare("INSERT INTO catalogue_piece (component,position,body) VALUES (?1,?2,?3)")?;
    for (position, piece) in body.chunks(PIECE).enumerate() {
        stmt.execute(params![part as u8, position as u32, piece])?;
    }
    Ok(())
}

fn hydrate(
    db: &mut Db,
    part: Part,
    notices: &mut Vec<(Level, String)>,
) -> Result<Result<Packed, CatalogueReferenceError>, SqlFailure> {
    let mut evidence = sql::Evidence {
        source: db.root.path().join(part.source()),
        digest: None,
        domain: DOMAIN,
        ledger_source: part.source().to_string(),
        what: "catalogue references",
    };
    let conn = db.conn.as_mut().ok_or_else(sql::closed)?;
    let tx = conn.transaction().map_err(sql_error)?;
    if let Some(head) = read_head(&tx, part)? {
        let packed = read_body(&tx, part, head)?;
        let ledger = sql::read_ledger_of(&tx, DOMAIN, part.source())?;
        tx.commit().map_err(sql_error)?;
        if let Some(ledger) = ledger {
            db.settle_plain_import(&evidence, ledger, notices);
        }
        return Ok(Ok(packed));
    }
    if sql::read_ledger_of(&tx, DOMAIN, part.source())?.is_some() {
        return Err(damaged());
    }
    drop(tx);
    let guard = db.guard()?;
    let raw = match vault::read_private_file(&evidence.source, part.limit() as u64) {
        Ok(raw) => {
            evidence.digest = Some(files::digest(raw.as_bytes()));
            Some(raw)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Ok(Err(refusal(evidence.source, error))),
    };
    let mut diagnostic = None;
    let (body, count) = match part {
        Part::Files => {
            let rows: Vec<FileCardReference> = match raw.as_deref() {
                None => Vec::new(),
                Some(raw) => {
                    let document = match api::json::value_and_duplicate_fields(raw) {
                        Ok((document, None)) => document,
                        Ok((_, Some(duplicate))) => {
                            return Ok(Err(refusal(evidence.source, duplicate)))
                        }
                        Err(error) => return Ok(Err(refusal(evidence.source, error))),
                    };
                    match serde_json::from_value(document) {
                        Ok(rows) => rows,
                        Err(error) => return Ok(Err(refusal(evidence.source, error))),
                    }
                }
            };
            match encode_files(&rows) {
                Ok(body) => (body, rows.len()),
                Err(error) => return Ok(Err(refusal(evidence.source, error))),
            }
        }
        Part::Root => {
            let text = raw.as_deref().unwrap_or("").trim();
            if !text.is_empty() {
                if let Err(reason) = checked_root(text) {
                    diagnostic = Some(format!("{}: {reason}; no saved catalogue root was imported. Choose the checkout again", evidence.source.display()));
                    (Vec::new(), 0)
                } else {
                    (text.as_bytes().to_vec(), 1)
                }
            } else {
                (Vec::new(), 0)
            }
        }
        Part::Deferred => (
            vec![u8::from(
                raw.as_deref().is_some_and(|text| text.trim() == "deferred"),
            )],
            1,
        ),
    };
    let conn = db.conn.as_mut().ok_or_else(sql::closed)?;
    let tx = sql::begin_write_reserved(
        conn,
        &db.root,
        &guard,
        "importing catalogue references",
        PAGES,
    )?;
    if read_head(&tx, part)?.is_some() {
        return Err(SqlFailure::Wait(sql::WaitCause::Busy));
    }
    let version = sql::next_version(&tx, None)?;
    let head = Head {
        version,
        digest: files::digest(&body),
        bytes: body.len(),
        rows: count,
    };
    write_value(&tx, part, head, &body).map_err(sql_error)?;
    tx.execute(
        "UPDATE store_meta SET sequence=?1 WHERE id=1",
        [sql::be(version.get())],
    )
    .map_err(sql_error)?;
    sql::insert_import(&tx, &evidence)?;
    tx.commit().map_err(sql_error)?;
    drop(guard);
    if let Some(line) = diagnostic {
        notices.push((Level::Warn, line));
    }
    notices.push((
        Level::Info,
        format!(
            "local state: catalogue import source={} version={} disposition={}",
            part.source(),
            version,
            if raw.is_some() { "imported" } else { "absent" }
        ),
    ));
    if evidence.digest.is_some() {
        db.settle_plain_import(
            &evidence,
            sql::Ledger {
                digest: evidence.digest,
                evidence: "pending".into(),
            },
            notices,
        );
    }
    Ok(Ok(Packed {
        head,
        body: body.into(),
    }))
}

struct Staged {
    part: Part,
    previous: Option<Head>,
    body: Arc<[u8]>,
    digest: files::ContentDigest,
    rows: usize,
}

impl sql::Staging for Staged {
    type Durable = Option<Head>;
    const DOMAIN: LocalStateDomain = LocalStateDomain::LoadoutsReferences;
    const HEAD: &'static str = DOMAIN;
    const ACTION: &'static str = "catalogue-edit";
    const MAX_PAGES: u64 = PAGES;
    fn unchanged(&self) -> bool {
        self.previous
            .is_some_and(|head| head.digest == self.digest && head.rows == self.rows)
    }
    fn digest(&self) -> files::ContentDigest {
        self.digest
    }
    fn set_version(&mut self, _version: StoreVersion) {}
    fn write(&self, tx: &Connection, version: StoreVersion) -> rusqlite::Result<()> {
        write_value(
            tx,
            self.part,
            self.durable(version, self.digest)
                .expect("staging constructs a head"),
            &self.body,
        )
    }
    fn durable(&self, version: StoreVersion, digest: files::ContentDigest) -> Option<Head> {
        Some(Head {
            version,
            digest,
            bytes: self.body.len(),
            rows: self.rows,
        })
    }
}
