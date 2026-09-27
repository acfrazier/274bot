use super::*;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerSelection {
    Local274,
    Local289,
    Public289,
}

impl ServerSelection {
    pub fn parse(name: &str) -> Result<Self, String> {
        match name {
            "local-274" => Ok(Self::Local274),
            "local-289" => Ok(Self::Local289),
            "public-289" => Ok(Self::Public289),
            "public-274" => Err("public revision 274 is unavailable; use public-289".into()),
            _ => Err(format!(
                "unsupported server profile {name:?}; use local-274, local-289 or public-289"
            )),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Local274 => "local-274",
            Self::Local289 => "local-289",
            Self::Public289 => "public-289",
        }
    }

    pub fn revision(self) -> ClientRevision {
        match self {
            Self::Local274 => ClientRevision::R274,
            Self::Local289 | Self::Public289 => ClientRevision::R289,
        }
    }

    pub fn target(self) -> BotTarget {
        match self {
            Self::Local274 | Self::Local289 => BotTarget::Local,
            Self::Public289 => BotTarget::Prod,
        }
    }
}

pub fn parse_revision(value: &str) -> Result<ClientRevision, String> {
    match value {
        "274" => Ok(ClientRevision::R274),
        "289" => Ok(ClientRevision::R289),
        _ => Err(format!("unsupported revision {value:?}; use 274 or 289")),
    }
}

/// WORLD membership bound to a selected endpoint. Unknown routes as false.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorldMembersFact {
    Unknown,
    Known {
        members: bool,
        source: WorldMembersSource,
    },
}

/// How a known [`WorldMembersFact`] was declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorldMembersSource {
    /// Guarded local `data/config/world.json` whose revision, port, and
    /// `node.members` bool all matched the selected loopback profile.
    LocalWorldJson {
        path: PathBuf,
        sha256: String,
        bytes: u64,
    },
    /// `--world-members true|false` on this profile.
    ExplicitOverride,
}

impl WorldMembersFact {
    /// Routing fact: unknown and known-free are both false.
    pub fn map_members(&self) -> bool {
        matches!(self, Self::Known { members: true, .. })
    }
}

/// Explicit launch overrides. No process state changes occur while parsing.
#[derive(Debug, Clone, Default)]
pub struct ProfileOptions {
    pub profile: Option<String>,
    pub revision: Option<String>,
    pub prod: bool,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub asset_host: Option<String>,
    pub http_port: Option<u16>,
    pub engine_dir: Option<PathBuf>,
    pub cache_dir: Option<PathBuf>,
    pub unpack_dir: Option<PathBuf>,
    pub nav_pack: Option<PathBuf>,
    pub nav_flags: Option<PathBuf>,
    pub content_dir: Option<PathBuf>,
    pub vault_path: Option<PathBuf>,
    pub catalog_root: Option<PathBuf>,
    pub cache_manifest: Option<PathBuf>,
    /// Operator-declared WORLD membership for this endpoint (`true`/`false`).
    /// Omission preserves the guarded local world.json bind / unknown public.
    pub world_members: Option<bool>,
}

/// Consume shared connection/resource flags; return frontend-specific args.
/// This makes flag order independent and gives both frontends one parser.
pub fn parse_profile_args(
    args: impl IntoIterator<Item = impl AsRef<str>>,
) -> Result<(ProfileOptions, Vec<String>), String> {
    let mut options = ProfileOptions::default();
    let mut rest = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let flag = arg.as_ref();
        if flag == "--prod" {
            options.prod = true;
            continue;
        }
        if !matches!(
            flag,
            "--profile"
                | "--revision"
                | "--host"
                | "--port"
                | "--asset-host"
                | "--http-port"
                | "--engine"
                | "--cache"
                | "--unpack"
                | "--nav-pack"
                | "--nav-flags"
                | "--content"
                | "--vault"
                | "--catalog"
                | "--cache-manifest"
                | "--world-members"
        ) {
            rest.push(flag.to_string());
            continue;
        }
        let value = args.next().ok_or_else(|| format!("{flag} needs a value"))?;
        let value = value.as_ref();
        if value.is_empty() || value.starts_with("--") {
            return Err(format!("{flag} needs a value"));
        }
        let port = || {
            value
                .parse::<u16>()
                .ok()
                .filter(|p| *p != 0)
                .ok_or_else(|| format!("{flag} needs a port from 1 to 65535"))
        };
        match flag {
            "--profile" => options.profile = Some(value.into()),
            "--revision" => {
                parse_revision(value)?;
                options.revision = Some(value.into());
            }
            "--host" => options.host = Some(value.into()),
            "--port" => options.port = Some(port()?),
            "--asset-host" => options.asset_host = Some(value.into()),
            "--http-port" => options.http_port = Some(port()?),
            "--engine" => options.engine_dir = Some(value.into()),
            "--cache" => options.cache_dir = Some(value.into()),
            "--unpack" => options.unpack_dir = Some(value.into()),
            "--nav-pack" => options.nav_pack = Some(value.into()),
            "--nav-flags" => options.nav_flags = Some(value.into()),
            "--content" => options.content_dir = Some(value.into()),
            "--vault" => options.vault_path = Some(value.into()),
            "--catalog" => options.catalog_root = Some(value.into()),
            "--cache-manifest" => options.cache_manifest = Some(value.into()),
            "--world-members" => {
                options.world_members = Some(match value {
                    "true" => true,
                    "false" => false,
                    _ => return Err("--world-members needs true or false".into()),
                });
            }
            _ => unreachable!(),
        }
    }
    Ok((options, rest))
}

/// Captured environment, injectable without mutating process settings in tests.
#[derive(Debug, Clone, Default)]
pub struct ProfileEnvironment {
    pub home: Option<PathBuf>,
    pub working_dir: Option<PathBuf>,
    pub profile: Option<String>,
    pub revision: Option<String>,
    pub target: Option<String>,
    pub engine_dir: Option<PathBuf>,
    pub unpack_dir: Option<PathBuf>,
    pub nav_pack: Option<PathBuf>,
    pub nav_flags: Option<PathBuf>,
    pub catalog_root: Option<PathBuf>,
    pub cache_manifest: Option<PathBuf>,
    pub rsa_modulus: Option<String>,
    pub rsa_exponent: Option<String>,
}

impl ProfileEnvironment {
    pub fn capture() -> Self {
        fn value(name: &str) -> Option<String> {
            std::env::var(name).ok().filter(|s| !s.is_empty())
        }
        Self {
            home: client::operator_home().ok().map(PathBuf::from),
            working_dir: std::env::current_dir().ok(),
            profile: value("BOT_SERVER_PROFILE"),
            revision: value("BOT_REVISION"),
            target: value("BOT_TARGET"),
            engine_dir: value("ENGINE_DIR").map(PathBuf::from),
            unpack_dir: value("CLIENT_UNPACK_DIR").map(PathBuf::from),
            nav_pack: value("NAV_PACK").map(PathBuf::from),
            nav_flags: value("NAV_FLAGS").map(PathBuf::from),
            catalog_root: script::rs2b0t_root(),
            cache_manifest: value("BOT_CACHE_MANIFEST").map(PathBuf::from),
            rsa_modulus: value("LOGIN_RSAN"),
            rsa_exponent: value("LOGIN_RSAE"),
        }
    }
}
