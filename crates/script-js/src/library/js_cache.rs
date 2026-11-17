//! Content-addressed JS cache under `~/.274bot/js-cache`. Origin bytes
//! (TS or JS) are SHA-256 hashed; hits return the cached object without
//! compiling again. Objects live under the `host-compile-v1` namespace, split
//! by origin media and module role:
//! `host-compile-v1/{js,ts}/{entry,sibling}/objects/<origin-sha>.js`, each
//! directory with its own provenance `manifest.json`. An entry object holds
//! the execution source with compile-time declarations erased; a sibling
//! object keeps every export. Objects outside the namespace are never hits.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::transpile_ts;
use script::card::{ScriptKind, ScriptSource};

/// One cached transpile result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedJs {
    pub sha256: String,
    pub js: String,
}

/// Provenance written into `manifest.json` on cache miss.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheMeta {
    pub kind: ScriptKind,
    pub source: ScriptSource,
    pub shape: Option<String>,
    /// Provenance only — not part of the SHA cache key.
    pub api_family: Option<String>,
}

/// On-disk manifest entry for one cached object.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
struct ManifestEntry {
    sha256: String,
    origin: String,
    kind: String,
    source: String,
    media: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    shape: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    api_family: Option<String>,
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct Manifest {
    objects: HashMap<String, ManifestEntry>,
}

/// The module role of a cached object. An entry and a sibling compiled from
/// the same origin bytes never share an object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheRole {
    /// The card's entry module, with compile-time declarations erased.
    Entry,
    /// An ordinary sibling module, compiled from its origin unchanged.
    Sibling,
}

impl CacheRole {
    fn dir(self) -> &'static str {
        match self {
            Self::Entry => "entry",
            Self::Sibling => "sibling",
        }
    }
}

/// The host transform namespace. A future incompatible transform changes it.
const HOST_COMPILE_NAMESPACE: &str = "host-compile-v1";

/// SHA-256 content cache for compiled JS.
#[derive(Debug)]
pub struct JsCache {
    root: PathBuf,
}

/// Default operator cache root (`~/.274bot/js-cache`).
pub fn default_js_cache_root() -> PathBuf {
    script::bot_file("js-cache")
}

impl JsCache {
    pub fn new(root: PathBuf) -> Self {
        JsCache { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn namespace(&self, role: CacheRole, origin: &Path) -> PathBuf {
        self.root
            .join(HOST_COMPILE_NAMESPACE)
            .join(origin_media(origin))
            .join(role.dir())
    }

    /// The object file for `origin` (its media) in `role`, keyed by `sha256`.
    pub fn object_path(&self, role: CacheRole, origin: &Path, sha256: &str) -> PathBuf {
        self.namespace(role, origin)
            .join("objects")
            .join(format!("{sha256}.js"))
    }

    /// SHA-256 of origin bytes (the cache key). Does not read or write.
    pub fn origin_sha(bytes: &[u8]) -> String {
        hex_sha256(bytes)
    }

    /// True when the `role` object for these origin bytes already exists.
    pub fn is_cached(&self, role: CacheRole, origin: &Path, bytes: &[u8]) -> bool {
        self.object_path(role, origin, &hex_sha256(bytes)).is_file()
    }

    /// Return cached JS for `origin_text`, keyed by its SHA-256 within
    /// `role` and the origin's media. On miss, compile `execution` (the
    /// origin with compile-time declarations erased, or the origin itself):
    /// transpile `.ts` media, store JS media verbatim, write the object and
    /// update that directory's `manifest.json`. Never writes beside `origin`.
    pub fn get_or_compile(
        &self,
        role: CacheRole,
        origin: &Path,
        origin_text: &str,
        execution: &str,
        meta: CacheMeta,
    ) -> Result<CachedJs, String> {
        let namespace = self.namespace(role, origin);
        self.ensure_layout(&namespace)?;
        let sha256 = hex_sha256(origin_text.as_bytes());
        let object = self.object_path(role, origin, &sha256);
        if object.is_file() {
            let js = std::fs::read_to_string(&object)
                .map_err(|e| format!("read cached {}: {e}", object.display()))?;
            return Ok(CachedJs { sha256, js });
        }

        let media = origin_media(origin);
        let js = match media {
            "js" => execution.to_string(),
            _ => transpile_ts(execution)?,
        };

        vault::write_private_file(&object, js.as_bytes())
            .map_err(|e| format!("write cached {}: {e}", object.display()))?;

        let manifest_path = namespace.join("manifest.json");
        let mut manifest = read_manifest(&manifest_path);
        manifest.objects.insert(
            sha256.clone(),
            ManifestEntry {
                sha256: sha256.clone(),
                origin: origin.to_string_lossy().into_owned(),
                kind: script_kind_label(meta.kind),
                source: script_source_label(meta.source),
                media: media.to_string(),
                shape: meta.shape,
                api_family: meta.api_family,
            },
        );
        write_manifest(&manifest_path, &manifest)?;

        Ok(CachedJs { sha256, js })
    }

    fn ensure_layout(&self, namespace: &Path) -> Result<(), String> {
        let objects = namespace.join("objects");
        std::fs::create_dir_all(&objects)
            .map_err(|e| format!("cache layout {}: {e}", self.root.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut dir = objects.as_path();
            loop {
                std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
                    .map_err(|e| format!("cache mode {}: {e}", dir.display()))?;
                if dir == self.root {
                    break;
                }
                match dir.parent() {
                    Some(parent) => dir = parent,
                    None => break,
                }
            }
        }
        let marker = objects.join(".keep");
        if !marker.exists() {
            vault::write_private_file(&marker, b"")
                .map_err(|e| format!("cache layout {}: {e}", self.root.display()))?;
        }
        Ok(())
    }
}

fn read_manifest(path: &Path) -> Manifest {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Manifest::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn write_manifest(path: &Path, manifest: &Manifest) -> Result<(), String> {
    let json = serde_json::to_string_pretty(manifest).map_err(|e| format!("manifest.json: {e}"))?;
    vault::write_private_file(path, json.as_bytes()).map_err(|e| format!("manifest.json: {e}"))
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn origin_media(origin: &Path) -> &'static str {
    match origin.extension().and_then(|s| s.to_str()) {
        Some("js") | Some("mjs") | Some("cjs") => "js",
        _ => "ts",
    }
}

fn script_kind_label(kind: ScriptKind) -> String {
    match kind {
        ScriptKind::Compat => "Compat".into(),
        ScriptKind::NativeTick => "NativeTick".into(),
        ScriptKind::Compiled => "Compiled".into(),
    }
}

fn script_source_label(source: ScriptSource) -> String {
    match source {
        ScriptSource::Catalog => "Catalog".into(),
        ScriptSource::File => "File".into(),
        ScriptSource::Builtin => "Builtin".into(),
    }
}
