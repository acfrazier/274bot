use std::fmt;
use std::sync::Arc;

use script::surface::extension::{
    ExtensionDock, ExtensionFunction, ExtensionNamespace, RunBinding, EXTENSION_MAX_DEPTH,
    EXTENSION_MAX_TEXT_BYTES,
};

/// A validated immutable set of extension functions and plain namespaces.
#[derive(Debug, Clone)]
pub struct ExtensionRegistry {
    functions: Vec<ExtensionFunction>,
    namespaces: Vec<ExtensionNamespace>,
}

/// A rejected host-owned extension registry declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionRegistryError {
    detail: &'static str,
}

impl fmt::Display for ExtensionRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid scenario extension registry: {}",
            self.detail
        )
    }
}

impl std::error::Error for ExtensionRegistryError {}

impl ExtensionRegistry {
    /// Validate and retain one host-owned extension declaration.
    pub fn new(
        functions: Vec<ExtensionFunction>,
        namespaces: Vec<ExtensionNamespace>,
    ) -> Result<Self, ExtensionRegistryError> {
        for (index, function) in functions.iter().enumerate() {
            if !valid_path(function.name) || reserved_root(function.name) {
                return Err(registry_error(
                    "extension function name is invalid or reserved",
                ));
            }
            if functions[..index]
                .iter()
                .any(|prior| prior.name == function.name)
            {
                return Err(registry_error("extension function name is duplicated"));
            }
        }
        for (index, namespace) in namespaces.iter().enumerate() {
            if !valid_path(namespace.name) || reserved_root(namespace.name) {
                return Err(registry_error(
                    "extension namespace name is invalid or reserved",
                ));
            }
            if namespaces[..index]
                .iter()
                .any(|prior| prior.name == namespace.name)
            {
                return Err(registry_error("extension namespace name is duplicated"));
            }
            for (constant_index, constant) in namespace.constants.iter().enumerate() {
                if !valid_component(constant.name) || constant.name.len() > EXTENSION_MAX_TEXT_BYTES
                {
                    return Err(registry_error("extension constant name is invalid"));
                }
                if namespace.constants[..constant_index]
                    .iter()
                    .any(|prior| prior.name == constant.name)
                {
                    return Err(registry_error("extension constant name is duplicated"));
                }
                if constant.value.validate_bounded().is_err() {
                    return Err(registry_error(
                        "extension constant exceeds plain-value bounds",
                    ));
                }
            }
        }
        for (index, function) in functions.iter().enumerate() {
            if functions[index + 1..]
                .iter()
                .any(|later| paths_overlap(function.name, later.name))
            {
                return Err(registry_error("extension function paths conflict"));
            }
            for namespace in &namespaces {
                if is_path_prefix(function.name, namespace.name) {
                    return Err(registry_error(
                        "extension function conflicts with a namespace",
                    ));
                }
                for constant in &namespace.constants {
                    let constant_path = format!("{}.{}", namespace.name, constant.name);
                    if paths_overlap(function.name, &constant_path) {
                        return Err(registry_error(
                            "extension function conflicts with a constant",
                        ));
                    }
                }
            }
        }
        for namespace in &namespaces {
            for constant in &namespace.constants {
                let constant_path = format!("{}.{}", namespace.name, constant.name);
                for other in &namespaces {
                    if other.name != namespace.name && paths_overlap(&constant_path, other.name) {
                        return Err(registry_error(
                            "extension constant conflicts with a namespace",
                        ));
                    }
                    for other_constant in &other.constants {
                        let other_path = format!("{}.{}", other.name, other_constant.name);
                        if constant_path != other_path && paths_overlap(&constant_path, &other_path)
                        {
                            return Err(registry_error("extension constant paths conflict"));
                        }
                    }
                }
            }
        }
        Ok(Self {
            functions,
            namespaces,
        })
    }

    pub(crate) fn functions(&self) -> &[ExtensionFunction] {
        &self.functions
    }

    pub(crate) fn namespaces(&self) -> &[ExtensionNamespace] {
        &self.namespaces
    }
}

/// The explicit per-Start extension installed for one scenario VM.
#[derive(Debug, Clone)]
pub struct LuauScenarioExtension {
    dock: Arc<ExtensionDock>,
    binding: RunBinding,
    registry: Arc<ExtensionRegistry>,
    case_name: Arc<str>,
    role_name: Arc<str>,
}

impl LuauScenarioExtension {
    /// Bind one fresh dock, registry, selected case and role to a Start.
    pub fn new(
        dock: Arc<ExtensionDock>,
        binding: RunBinding,
        registry: Arc<ExtensionRegistry>,
        case_name: impl Into<Arc<str>>,
        role_name: impl Into<Arc<str>>,
    ) -> Self {
        Self {
            dock,
            binding,
            registry,
            case_name: case_name.into(),
            role_name: role_name.into(),
        }
    }

    pub(crate) fn dock(&self) -> &Arc<ExtensionDock> {
        &self.dock
    }

    pub(crate) fn binding(&self) -> RunBinding {
        self.binding
    }

    pub(crate) fn registry(&self) -> &ExtensionRegistry {
        &self.registry
    }

    pub(crate) fn case_name(&self) -> &str {
        &self.case_name
    }

    pub(crate) fn role_name(&self) -> &str {
        &self.role_name
    }
}

fn registry_error(detail: &'static str) -> ExtensionRegistryError {
    ExtensionRegistryError { detail }
}

fn reserved_root(name: &str) -> bool {
    matches!(name.split('.').next(), Some("await" | "case" | "role"))
}

fn valid_path(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= EXTENSION_MAX_TEXT_BYTES
        && name.split('.').count() <= EXTENSION_MAX_DEPTH
        && name.split('.').all(valid_component)
}

fn valid_component(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn is_path_prefix(prefix: &str, path: &str) -> bool {
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|suffix| suffix.starts_with('.'))
}

fn paths_overlap(left: &str, right: &str) -> bool {
    is_path_prefix(left, right) || is_path_prefix(right, left)
}
