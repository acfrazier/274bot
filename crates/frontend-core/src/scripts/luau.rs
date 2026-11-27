use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use script::manifest::ScriptManifest;

pub(super) const IMPORTS_KEY: &str = "luau_imports";

#[cfg(test)]
thread_local! {
    static SOURCE_READ_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

use super::LoadStage;

#[derive(Debug, Clone)]
pub(super) struct LuauCard {
    path: PathBuf,
    name: String,
    manifest: ScriptManifest,
}

impl LuauCard {
    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn name(&self) -> &str {
        &self.name
    }

    pub(super) fn display_name(&self) -> &str {
        self.manifest.name()
    }

    pub(super) fn description(&self) -> &str {
        self.manifest.description()
    }

    pub(super) fn category(&self) -> &str {
        self.manifest.category()
    }

    pub(super) fn tags(&self) -> &[String] {
        self.manifest.tags()
    }

    pub(super) fn manifest(&self) -> &ScriptManifest {
        &self.manifest
    }

    pub(super) fn identity_id(&self) -> String {
        script::card_identity_id(script::ScriptSource::File, &self.path, &self.name)
    }

    pub(super) fn identity_matches(&self, identity: &str) -> bool {
        self.path.to_string_lossy() == identity
    }

    pub(super) fn identity_key(&self) -> String {
        script::card_identity_key(script::ScriptSource::File, &self.path, &self.name)
    }

    pub(super) fn assignment(&self) -> vault::ScriptAssignment {
        script::card_assignment(
            script::ScriptSource::File,
            &self.path,
            self.display_name(),
        )
    }
}

#[derive(Debug, Clone)]
pub(super) struct LuauLoadFailure {
    identity_key: String,
    name: String,
    path: PathBuf,
    stage: LoadStage,
    diagnostic: String,
    line: Option<u32>,
    column: Option<u32>,
}

impl LuauLoadFailure {
    fn read(path: &Path, error: impl std::fmt::Display) -> Self {
        Self::new(
            path,
            LoadStage::Read,
            format!("luau-load: {error}"),
            None,
            None,
        )
    }

    fn manifest(path: &Path, error: script_luau::ManifestError) -> Self {
        Self::new(
            path,
            LoadStage::ParseTranspile,
            format!("luau-manifest: {error}"),
            u32::try_from(error.line()).ok(),
            u32::try_from(error.column()).ok(),
        )
    }

    fn too_large(path: &Path, bytes: u64) -> Self {
        Self::new(
            path,
            LoadStage::ParseTranspile,
            format!(
                "luau-manifest-file-too-large: file is {bytes} bytes, limit is {} bytes",
                script_luau::manifest::SOURCE_LIMIT_BYTES
            ),
            None,
            None,
        )
    }

    fn new(
        path: &Path,
        stage: LoadStage,
        diagnostic: String,
        line: Option<u32>,
        column: Option<u32>,
    ) -> Self {
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .filter(|stem| !stem.is_empty())
            .unwrap_or("Luau script")
            .to_owned();
        Self {
            identity_key: script::card_identity_key(script::ScriptSource::File, path, &name),
            name,
            path: path.to_path_buf(),
            stage,
            diagnostic,
            line,
            column,
        }
    }

    pub(super) fn identity_key(&self) -> &str {
        &self.identity_key
    }

    pub(super) fn name(&self) -> &str {
        &self.name
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn stage(&self) -> LoadStage {
        self.stage
    }

    pub(super) fn diagnostic(&self) -> &str {
        &self.diagnostic
    }

    pub(super) fn position(&self) -> (Option<u32>, Option<u32>) {
        (self.line, self.column)
    }

    pub(super) fn named_line(&self) -> String {
        format!("{}: {}", self.name, self.diagnostic)
    }

    pub(super) fn copyable_text(&self) -> String {
        let location = match (self.line, self.column) {
            (Some(line), Some(column)) => {
                format!("{}:{line}:{column}", self.path.display())
            }
            _ => self.path.display().to_string(),
        };
        format!("{}\n{}\n{}", self.name, location, self.diagnostic)
    }
}

pub(super) struct LuauFile {
    path: PathBuf,
    source: String,
    manifest: script_luau::LuauManifest,
}

impl LuauFile {
    pub(super) fn read(path: &Path) -> Result<Self, LuauLoadFailure> {
        let canonical = path
            .canonicalize()
            .map_err(|error| LuauLoadFailure::read(path, error))?;
        let metadata = fs::metadata(&canonical)
            .map_err(|error| LuauLoadFailure::read(&canonical, error))?;
        if metadata.len() > script_luau::manifest::SOURCE_LIMIT_BYTES as u64 {
            return Err(LuauLoadFailure::too_large(&canonical, metadata.len()));
        }
        #[cfg(test)]
        SOURCE_READ_COUNT.with(|count| count.set(count.get() + 1));

        let bytes = fs::read(&canonical)
            .map_err(|error| LuauLoadFailure::read(&canonical, error))?;
        if bytes.len() > script_luau::manifest::SOURCE_LIMIT_BYTES {
            return Err(LuauLoadFailure::too_large(&canonical, bytes.len() as u64));
        }
        let source = String::from_utf8(bytes)
            .map_err(|error| LuauLoadFailure::read(&canonical, error))?;
        let manifest = script_luau::read_manifest(&source)
            .map_err(|error| LuauLoadFailure::manifest(&canonical, error))?;
        Ok(Self {
            path: canonical,
            source,
            manifest,
        })
    }

    pub(super) fn card(&self) -> LuauCard {
        let name = self
            .path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .filter(|stem| !stem.is_empty())
            .unwrap_or("Luau script")
            .to_owned();
        LuauCard {
            path: self.path.clone(),
            name,
            manifest: self.manifest.manifest().clone(),
        }
    }

    pub(super) fn erased_source(&self) -> String {
        self.manifest.erased_source(&self.source)
    }
}

#[derive(Default)]
pub(super) struct LuauLibrary {
    enabled: bool,
    cards: Vec<LuauCard>,
    last_refusal: Option<LuauLoadFailure>,
}

impl LuauLibrary {
    #[cfg(test)]
    pub(super) fn reset_source_read_count_for_test() {
        SOURCE_READ_COUNT.with(|count| count.set(0));
    }

    #[cfg(test)]
    pub(super) fn source_read_count_for_test() -> usize {
        SOURCE_READ_COUNT.with(std::cell::Cell::get)
    }

    pub(super) fn enabled(&self) -> bool {
        self.enabled
    }

    pub(super) fn visible_cards(&self) -> &[LuauCard] {
        if self.enabled {
            &self.cards
        } else {
            &[]
        }
    }

    pub(super) fn card(&self, identity: &str) -> Option<&LuauCard> {
        self.cards
            .iter()
            .find(|card| card.identity_matches(identity))
    }

    pub(super) fn last_refusal(&self) -> Option<&LuauLoadFailure> {
        self.last_refusal.as_ref()
    }
    pub(super) fn clear_refusal(&mut self) {
        self.last_refusal = None;
    }

    pub(super) fn set_disabled(&mut self) {
        self.enabled = false;
    }

    pub(super) fn restore(&mut self, references: Option<&Value>) {
        self.enabled = true;
        self.cards.clear();
        self.last_refusal = None;
        let mut seen = HashSet::new();
        let Some(paths) = references.and_then(Value::as_array) else {
            return;
        };
        for path in paths.iter().filter_map(Value::as_str) {
            let path = Path::new(path);
            let identity = script::file_identity(path);
            if !seen.insert(identity) {
                continue;
            }
            match LuauFile::read(path) {
                Ok(file) => self.register(file.card()),
                Err(error) => self.last_refusal = Some(error),
            }
        }
    }

    pub(super) fn read_file(&mut self, path: &Path) -> Result<LuauFile, LuauLoadFailure> {
        match LuauFile::read(path) {
            Ok(file) => {
                self.last_refusal = None;
                Ok(file)
            }
            Err(error) => {
                self.last_refusal = Some(error.clone());
                Err(error)
            }
        }
    }

    pub(super) fn register(&mut self, card: LuauCard) {
        if let Some(existing) = self
            .cards
            .iter_mut()
            .find(|existing| existing.path == card.path)
        {
            *existing = card;
        } else {
            self.cards.push(card);
        }
        self.last_refusal = None;
    }
}
