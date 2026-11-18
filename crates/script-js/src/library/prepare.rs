//! Static preparation of one script origin.
//!
//! The origin is classified from its syntax, the API family and declared host
//! capabilities are fixed here, and compile-time declarations are erased from
//! the execution source. Start carries the result in [`CheckedLoad`]; no later
//! stage reads the API family from execution JavaScript again.

use std::fmt;
use std::path::Path;
use std::sync::Arc;

use script::manifest::{DeclaredHostCapabilities, ScriptManifest};

use crate::library::manifest::{self, CompatShape, ExtractionError, ModuleSources, ScriptAnalysis};
use crate::library::shape::{ApiFamily, LoadShape};

/// The checked load identity of a prepared script: its loader shape, API
/// family and declared host capabilities. Only preparation creates one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckedLoad {
    shape: LoadShape,
    family: ApiFamily,
    capabilities: DeclaredHostCapabilities,
}

impl CheckedLoad {
    /// The loader shape: native tick or one compat shape, never `Reject`.
    pub fn shape(self) -> LoadShape {
        self.shape
    }

    /// The checked API family.
    pub fn family(self) -> ApiFamily {
        self.family
    }

    /// The host capabilities that the origin declares.
    pub fn capabilities(self) -> DeclaredHostCapabilities {
        self.capabilities
    }

    /// A manifest-form native card: it ticks through the v2 native API.
    pub fn v2_native(self) -> bool {
        self.shape == LoadShape::NativeTick && self.family == ApiFamily::V2
    }
}

/// Why an origin cannot register.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OriginRefusal {
    /// The origin is not a native manifest-form or recognized compat script.
    NotBotShape,
    /// A declaration, source syntax or module-graph error.
    Declaration(ExtractionError),
    /// A compat script names an explicit `apiVersion` other than 1.
    CompatVersion(String),
    /// The execution source does not compile.
    Compilation(String),
}

impl OriginRefusal {
    /// The typed declaration error, when the refusal has one.
    pub fn declaration(&self) -> Option<&ExtractionError> {
        match self {
            Self::Declaration(error) => Some(error),
            Self::NotBotShape | Self::CompatVersion(_) | Self::Compilation(_) => None,
        }
    }
}

impl fmt::Display for OriginRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotBotShape => f.write_str("not a bot shape"),
            Self::Declaration(error) => error.fmt(f),
            Self::CompatVersion(reason) | Self::Compilation(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for OriginRefusal {}

/// One checked origin: its load identity, the native manifest, and the
/// execution source when declarations were erased.
#[derive(Debug, Clone)]
pub struct CheckedOrigin {
    checked: CheckedLoad,
    manifest: Option<Arc<ScriptManifest>>,
    erased: Option<String>,
}

impl CheckedOrigin {
    pub fn checked(&self) -> CheckedLoad {
        self.checked
    }

    pub fn manifest(&self) -> Option<&Arc<ScriptManifest>> {
        self.manifest.as_ref()
    }

    /// The source that the host compiles: `origin` with each compile-time
    /// declaration erased, or `origin` itself when it has none.
    pub fn execution<'a>(&'a self, origin: &'a str) -> &'a str {
        self.erased.as_deref().unwrap_or(origin)
    }
}

/// Check an origin file and its sibling files on disk.
pub fn check_origin(path: &Path, origin: &str) -> Result<CheckedOrigin, OriginRefusal> {
    check_origin_with(path, origin, ModuleSources::Disk)
}

fn check_origin_with(
    path: &Path,
    origin: &str,
    sources: ModuleSources,
) -> Result<CheckedOrigin, OriginRefusal> {
    let analysis = manifest::analyze_with_sources(path, origin, sources)
        .map_err(OriginRefusal::Declaration)?;
    match analysis {
        ScriptAnalysis::NotBotShape => Err(OriginRefusal::NotBotShape),
        ScriptAnalysis::Native {
            manifest,
            erased_spans,
            ..
        } => Ok(CheckedOrigin {
            checked: CheckedLoad {
                shape: LoadShape::NativeTick,
                family: ApiFamily::V2,
                capabilities: manifest.capabilities(),
            },
            manifest: Some(Arc::new(manifest)),
            erased: Some(manifest::erase_declarations(origin, &erased_spans)),
        }),
        ScriptAnalysis::Compat {
            shape,
            capabilities,
            erased_spans,
        } => {
            check_compat_version(origin)?;
            Ok(CheckedOrigin {
                checked: CheckedLoad {
                    shape: match shape {
                        CompatShape::DefineBot => LoadShape::CompatDefineBot,
                        CompatShape::Class => LoadShape::CompatClass,
                    },
                    family: ApiFamily::V1,
                    capabilities,
                },
                manifest: None,
                erased: (!erased_spans.is_empty())
                    .then(|| manifest::erase_declarations(origin, &erased_spans)),
            })
        }
    }
}

/// The API family of an origin that checks, for failure notices only.
pub(crate) fn checked_family(path: &Path, origin: &str) -> Option<ApiFamily> {
    check_origin(path, origin)
        .ok()
        .map(|checked| checked.checked.family)
}

/// A compat script keeps its optional explicit `apiVersion = 1` export.
fn check_compat_version(origin: &str) -> Result<(), OriginRefusal> {
    match crate::library::shape::compat_declared_api_version(origin) {
        Ok(None | Some(1)) => Ok(()),
        Ok(Some(2)) => Err(OriginRefusal::CompatVersion(
            "api-version-conflict: v2 with compatibility shape".into(),
        )),
        Ok(Some(other)) => Err(OriginRefusal::CompatVersion(format!(
            "api-version-unsupported: {other}"
        ))),
        Err(diag) => Err(OriginRefusal::CompatVersion(diag.to_string())),
    }
}

/// Check one raw origin (siblings on disk) and compile its execution source
/// without the cache: the JavaScript to evaluate and its checked identity.
/// Direct raw-source Starts use this; library cards compile through the cache.
pub fn compile_origin(path: &Path, origin: &str) -> Result<(String, CheckedLoad), OriginRefusal> {
    let checked = check_origin(path, origin)?;
    let js = compile_execution(path, checked.execution(origin))?;
    Ok((js, checked.checked))
}

fn compile_execution(path: &Path, execution: &str) -> Result<String, OriginRefusal> {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("js" | "mjs" | "cjs") => Ok(execution.to_owned()),
        _ => crate::library::shape::transpile_ts(execution).map_err(OriginRefusal::Compilation),
    }
}

/// Test preparation result: execution JavaScript and its checked identity.
#[cfg(any(test, feature = "test-hooks"))]
#[derive(Debug, Clone)]
pub struct PreparedTestOrigin {
    pub js: String,
    pub checked: CheckedLoad,
}

/// Prepare raw test origin through the common origin check.
///
/// `file_name` selects the media (`.js`, `.mjs`, `.cjs` or `.ts`). The entry
/// lives at `/rs2b0t/bot/scripts/bot/<file_name>`, the URL space of
/// `siblings` (`(module URL, source)` pairs). A native source without a
/// manifest is refused with `manifest-required`; nothing fills a default.
#[cfg(any(test, feature = "test-hooks"))]
pub fn prepare_raw_origin_for_test(
    file_name: &str,
    origin: &str,
    siblings: &[(String, String)],
) -> Result<PreparedTestOrigin, OriginRefusal> {
    let path = std::path::PathBuf::from("/rs2b0t/bot/scripts/bot").join(file_name);
    let checked = check_origin_with(&path, origin, ModuleSources::Memory(siblings.to_vec()))?;
    let js = compile_execution(&path, checked.execution(origin))?;
    Ok(PreparedTestOrigin {
        js,
        checked: checked.checked,
    })
}

/// A test fixture manifest builder. It names the fixture, its settings
/// object literal and its declared capabilities, and returns manifest-form
/// native source. It never repairs a rejected module: the body must not carry
/// its own `manifest`, `apiVersion`, `SETTINGS` or `hostCapabilities` export.
#[cfg(any(test, feature = "test-hooks"))]
#[derive(Debug, Clone)]
pub struct NativeFixture<'a> {
    name: &'a str,
    settings: &'a str,
    capabilities: &'a str,
}

#[cfg(any(test, feature = "test-hooks"))]
impl<'a> NativeFixture<'a> {
    pub fn new(name: &'a str) -> Self {
        Self {
            name,
            settings: "{}",
            capabilities: "[]",
        }
    }

    /// The manifest `settings` object literal.
    pub fn settings(mut self, settings: &'a str) -> Self {
        self.settings = settings;
        self
    }

    /// The manifest `capabilities` array literal.
    pub fn capabilities(mut self, capabilities: &'a str) -> Self {
        self.capabilities = capabilities;
        self
    }

    /// The manifest declaration followed by `body`.
    pub fn source(&self, body: &str) -> String {
        format!(
            "export const manifest = {{ apiVersion: 2, name: '{name}', version: 'test', \
             author: 'test', description: 'Test fixture {name}', category: 'Tests', \
             tags: [], settings: {settings}, capabilities: {capabilities} }};\n{body}",
            name = self.name,
            settings = self.settings,
            capabilities = self.capabilities,
        )
    }
}

/// Manifest-form native source for a test body with empty settings.
#[cfg(any(test, feature = "test-hooks"))]
pub fn native_fixture(body: &str) -> String {
    NativeFixture::new("fixture").source(body)
}

#[cfg(test)]
#[path = "prepare_tests.rs"]
mod tests;
