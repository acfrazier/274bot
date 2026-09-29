use super::*;
use sha2::{Digest, Sha256};

impl ProfileOptions {
    pub fn resolve(&self, saved_revision: Option<u16>) -> Result<ProfileSelection, String> {
        self.resolve_with_env(saved_revision, &ProfileEnvironment::capture())
    }

    pub fn resolve_with_env(
        &self,
        saved_revision: Option<u16>,
        env: &ProfileEnvironment,
    ) -> Result<ProfileSelection, String> {
        if env.legacy_target.is_some() {
            return Err("BOT_TARGET was removed; use BOT_SERVER_PROFILE or --profile".into());
        }
        let home = env.home.clone().unwrap_or_default();
        let bot_dir = home.join(".274bot");
        let servers = crate::Servers::load(&bot_dir, &home)?;
        if self.profile.is_some() && self.rs2b2t {
            return Err("--rs2b2t conflicts with --profile".into());
        }
        let cli_named = self.profile.is_some() || self.rs2b2t;
        let env_named = !cli_named && env.profile.is_some();
        let requested = if let Some(name) = &self.profile {
            name.clone()
        } else if self.rs2b2t {
            "rs2b2t".into()
        } else if let Some(name) = &env.profile {
            name.clone()
        } else if let Some(revision) = self.revision.as_deref() {
            match parse_revision(revision)? {
                ClientRevision::R274 => "local-274".into(),
                ClientRevision::R289 => "local-289".into(),
            }
        } else {
            match env
                .revision
                .as_deref()
                .map(parse_revision)
                .transpose()?
                .or_else(|| {
                    saved_revision.and_then(|revision| parse_revision(&revision.to_string()).ok())
                }) {
                Some(ClientRevision::R289) => "local-289".into(),
                _ => "local-274".into(),
            }
        };
        let mut profile = servers.resolve(&requested)?;
        let revision_override = if cli_named {
            self.revision.as_deref()
        } else if env_named {
            self.revision.as_deref().or(env.revision.as_deref())
        } else {
            None
        };
        if let Some(revision) = revision_override.map(parse_revision).transpose()? {
            if revision.as_i32() as u16 != profile.revision {
                return Err("--revision conflicts with the selected --profile".into());
            }
        }

        let original_worlds = profile.worlds.clone();
        if profile.client_transport() == client::Transport::Wss
            && (self.host.is_some()
                || self.port.is_some()
                || self.asset_host.is_some()
                || self.http_port.is_some())
        {
            let game_host = self
                .host
                .as_deref()
                .unwrap_or(original_worlds[0].host.as_str());
            let game_port = self.port.unwrap_or(original_worlds[0].port);
            let chosen = original_worlds
                .iter()
                .find(|world| world.host == game_host && world.port == game_port)
                .ok_or("wss profiles connect only to worlds in their roster")?;
            if self
                .asset_host
                .as_deref()
                .is_some_and(|host| host != chosen.asset_host)
                || self.http_port.is_some_and(|port| port != chosen.asset_port)
            {
                return Err("wss asset overrides must match the selected roster world".into());
            }
            if let Some(index) = profile
                .worlds
                .iter()
                .position(|world| world.number == chosen.number)
            {
                profile.worlds.rotate_left(index);
            }
        } else {
            let world = profile
                .worlds
                .first_mut()
                .expect("validated nonempty roster");
            if let Some(host) = &self.host {
                world.host.clone_from(host);
            }
            if let Some(port) = self.port {
                world.port = port;
            }
            if let Some(host) = &self.asset_host {
                world.asset_host.clone_from(host);
            } else if self.host.is_some() {
                world.asset_host.clone_from(&world.host);
            }
            if let Some(port) = self.http_port {
                world.asset_port = port;
            }
        }

        let transport = profile.client_transport();
        let class = profile_class(
            transport,
            profile.worlds.iter().map(|world| world.host.as_str()),
        );
        if transport == client::Transport::Tcp
            && class == ProfileClass::Remote
            && !profile.allow_plaintext_offhost
        {
            return Err(
                "non-loopback tcp requires allow_plaintext_offhost in the selected profile".into(),
            );
        }
        let first = profile.worlds.first().expect("validated nonempty roster");
        let game_host = first.host.clone();
        let game_port = first.port;
        let asset_host = first.asset_host.clone();
        let asset_port = first.asset_port;
        let revision = parse_revision(&profile.revision.to_string())?;
        let is_289 = revision == ClientRevision::R289;
        let engine_dir = self
            .engine_dir
            .clone()
            .or_else(|| env.engine_dir.clone())
            .or_else(|| match &profile.login_key {
                LoginKey::EngineDir { engine_dir } => Some(engine_dir.clone()),
                _ => None,
            })
            .unwrap_or_else(|| {
                home.join(if is_289 {
                    "experiments/lostcity-289/engine"
                } else {
                    "experiments/Server/engine"
                })
            });
        let unpack_dir = self
            .unpack_dir
            .clone()
            .or_else(|| env.unpack_dir.clone())
            .unwrap_or_else(|| bot_dir.join(if is_289 { "unpack-289" } else { "unpack" }));
        let cache_dir = self.cache_dir.clone().unwrap_or_else(|| unpack_dir.clone());
        let nav_pack_overridden = self.nav_pack.is_some() || env.nav_pack.is_some();
        let nav_flags_overridden = self.nav_flags.is_some() || env.nav_flags.is_some();
        let nav_pack = self
            .nav_pack
            .clone()
            .or_else(|| env.nav_pack.clone())
            .unwrap_or_else(|| {
                bot_dir.join(if is_289 {
                    "289/274bot.navpack"
                } else {
                    "274bot.navpack"
                })
            });
        let nav_flags = self
            .nav_flags
            .clone()
            .or_else(|| env.nav_flags.clone())
            .unwrap_or_else(|| nav_pack.with_extension("navflags"));
        let content_dir = self.content_dir.clone().unwrap_or_else(|| {
            engine_dir
                .parent()
                .unwrap_or(Path::new("."))
                .join("content")
        });
        let vault_path = self
            .vault_path
            .clone()
            .unwrap_or_else(|| bot_dir.join(&profile.vault));
        let working_dir = env
            .working_dir
            .clone()
            .map(Ok)
            .unwrap_or_else(std::env::current_dir)
            .map_err(|error| format!("profile working directory: {error}"))?;
        let absolute = |path: PathBuf| {
            if path.is_absolute() {
                path
            } else {
                working_dir.join(path)
            }
        };
        let engine_dir = absolute(engine_dir);
        let guarded = (class == ProfileClass::Local)
            .then(|| parse_guarded_local_world(profile.revision, &engine_dir))
            .flatten();
        let public_worlds =
            (transport == client::Transport::Wss).then(|| Arc::new(profile.public_worlds()));
        let explicit_members = self.world_members.or(profile.members);
        let world_members =
            resolve_world_members(explicit_members, guarded.as_ref(), public_worlds.as_deref());
        let endpoint_overridden = self.host.is_some()
            || self.port.is_some()
            || self.asset_host.is_some()
            || self.http_port.is_some();
        let supported_server = ((profile.name == "local-274" || profile.name == "local-289")
            && class == ProfileClass::Local
            && !endpoint_overridden)
            || (profile.name == "rs2b2t"
                && transport == client::Transport::Wss
                && matches!(
                    &world_members,
                    WorldMembersFact::Known {
                        source: WorldMembersSource::Rs2b2tWorlds,
                        ..
                    }
                ))
            || matching_local_world_supports_facts(
                profile.revision,
                &game_host,
                &asset_host,
                &world_members,
            );

        Ok(ProfileSelection {
            profile,
            class,
            game_host,
            game_port,
            asset_host,
            asset_port,
            engine_dir,
            cache_dir: absolute(cache_dir),
            unpack_dir: absolute(unpack_dir),
            nav_pack: absolute(nav_pack),
            nav_flags: absolute(nav_flags),
            content_dir: absolute(content_dir),
            vault_path: absolute(vault_path),
            catalog_root: self
                .catalog_root
                .clone()
                .or_else(|| env.catalog_root.clone())
                .map(absolute),
            cache_manifest: self
                .cache_manifest
                .clone()
                .or_else(|| env.cache_manifest.clone())
                .map(absolute),
            rsa_modulus: env.rsa_modulus.clone(),
            rsa_exponent: env.rsa_exponent.clone(),
            nav_pack_overridden,
            nav_flags_overridden,
            world_members,
            public_worlds,
            supported_server,
        })
    }
}

struct GuardedLocalWorld {
    members: bool,
    path: PathBuf,
    sha256: String,
    bytes: u64,
}

fn parse_guarded_local_world(revision: u16, engine_dir: &Path) -> Option<GuardedLocalWorld> {
    let path = engine_dir.join("data/config/world.json");
    let bytes = std::fs::read(&path).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    if value.get("engine")?.get("revision")?.as_u64()? != u64::from(revision) {
        return None;
    }
    let members = value.get("node")?.get("members")?.as_bool()?;
    let digest: [u8; 32] = Sha256::digest(&bytes).into();
    Some(GuardedLocalWorld {
        members,
        path,
        sha256: nav::pack::sha256_hex(&digest),
        bytes: bytes.len() as u64,
    })
}

fn resolve_world_members(
    explicit: Option<bool>,
    guarded: Option<&GuardedLocalWorld>,
    public_worlds: Option<&PublicWorlds>,
) -> WorldMembersFact {
    if let Some(members) = explicit {
        return WorldMembersFact::Known {
            members,
            source: WorldMembersSource::ExplicitOverride,
        };
    }
    if let Some(guarded) = guarded {
        return WorldMembersFact::Known {
            members: guarded.members,
            source: WorldMembersSource::LocalWorldJson {
                path: guarded.path.clone(),
                sha256: guarded.sha256.clone(),
                bytes: guarded.bytes,
            },
        };
    }
    if public_worlds.is_some_and(PublicWorlds::is_rs2b2t_members_roster) {
        return WorldMembersFact::Known {
            members: true,
            source: WorldMembersSource::Rs2b2tWorlds,
        };
    }
    WorldMembersFact::Unknown
}

fn matching_local_world_supports_facts(
    revision: u16,
    game_host: &str,
    asset_host: &str,
    world_members: &WorldMembersFact,
) -> bool {
    crate::is_loopback_host(game_host)
        && crate::is_loopback_host(asset_host)
        && matches!(
            world_members,
            WorldMembersFact::Known {
                source: WorldMembersSource::LocalWorldJson { .. },
                ..
            }
        )
        && matches!(revision, 274 | 289)
}
