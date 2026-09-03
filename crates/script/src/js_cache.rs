//! Content-addressed JS cache under `~/.274bot/js-cache`. Origin bytes
//! (TS or JS) are SHA-256 hashed; hits return the cached object without
//! calling `transpile_ts`. Misses transpile `.ts` origins (plain `.js`
//! is stored verbatim) and write `objects/<hex>.js` plus `manifest.json`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::load::transpile_ts;
use crate::rs2b0t_registry::{ScriptKind, ScriptSource};

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
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct Manifest {
    objects: HashMap<String, ManifestEntry>,
}

/// SHA-256 content cache for transpiled JS.
pub struct JsCache {
    root: PathBuf,
}

/// Default operator cache root (`~/.274bot/js-cache`).
pub fn default_js_cache_root() -> PathBuf {
    crate::bot_file("js-cache")
}

impl JsCache {
    pub fn new(root: PathBuf) -> Self {
        JsCache { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn object_path(&self, sha256: &str) -> PathBuf {
        self.root.join("objects").join(format!("{sha256}.js"))
    }

    /// SHA-256 of origin bytes (the cache key). Does not read or write.
    pub fn origin_sha(bytes: &[u8]) -> String {
        hex_sha256(bytes)
    }

    /// True when `objects/<sha>.js` already exists for these origin bytes.
    pub fn is_cached(&self, bytes: &[u8]) -> bool {
        self.object_path(&hex_sha256(bytes)).is_file()
    }

    fn manifest_path(&self) -> PathBuf {
        self.root.join("manifest.json")
    }

    /// Return cached JS for `bytes` keyed by SHA-256. On miss, transpile
    /// `.ts` origins (or store `.js` verbatim), write the object file, and
    /// update `manifest.json`. Never writes beside `origin` on disk.
    pub fn get_or_transpile(
        &self,
        origin: &Path,
        bytes: &[u8],
        meta: CacheMeta,
    ) -> Result<CachedJs, String> {
        self.ensure_layout()?;
        let sha256 = hex_sha256(bytes);
        let object = self.object_path(&sha256);
        if object.is_file() {
            let js = std::fs::read_to_string(&object)
                .map_err(|e| format!("read cached {}: {e}", object.display()))?;
            return Ok(CachedJs { sha256, js });
        }

        let text = std::str::from_utf8(bytes)
            .map_err(|_| format!("origin {} is not UTF-8", origin.display()))?;
        let media = origin_media(origin);
        let js = match media.as_str() {
            "js" => text.to_string(),
            _ => transpile_ts(text)?,
        };

        vault::write_private_file(&object, js.as_bytes())
            .map_err(|e| format!("write cached {}: {e}", object.display()))?;

        let mut manifest = self.read_manifest();
        manifest.objects.insert(
            sha256.clone(),
            ManifestEntry {
                sha256: sha256.clone(),
                origin: origin.to_string_lossy().into_owned(),
                kind: script_kind_label(meta.kind),
                source: script_source_label(meta.source),
                media,
                shape: meta.shape,
            },
        );
        self.write_manifest(&manifest)?;

        Ok(CachedJs { sha256, js })
    }

    fn ensure_layout(&self) -> Result<(), String> {
        let objects = self.root.join("objects");
        std::fs::create_dir_all(&objects)
            .map_err(|e| format!("cache layout {}: {e}", self.root.display()))?;
        let marker = objects.join(".keep");
        if !marker.exists() {
            vault::write_private_file(&marker, b"")
                .map_err(|e| format!("cache layout {}: {e}", self.root.display()))?;
        }
        Ok(())
    }

    fn read_manifest(&self) -> Manifest {
        let path = self.manifest_path();
        let Ok(raw) = std::fs::read_to_string(&path) else {
            return Manifest::default();
        };
        serde_json::from_str(&raw).unwrap_or_default()
    }

    fn write_manifest(&self, manifest: &Manifest) -> Result<(), String> {
        let json =
            serde_json::to_string_pretty(manifest).map_err(|e| format!("manifest.json: {e}"))?;
        vault::write_private_file(&self.manifest_path(), json.as_bytes())
            .map_err(|e| format!("manifest.json: {e}"))
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn origin_media(origin: &Path) -> String {
    match origin.extension().and_then(|s| s.to_str()) {
        Some("js") | Some("mjs") | Some("cjs") => "js".to_string(),
        _ => "ts".to_string(),
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
