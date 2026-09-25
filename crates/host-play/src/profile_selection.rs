use super::*;
impl ProfileOptions {
    pub fn resolve(&self, saved_revision: Option<u16>) -> Result<ProfileSelection, String> {
        self.resolve_with_env(saved_revision, &ProfileEnvironment::capture())
    }

    pub fn resolve_with_env(
        &self,
        saved_revision: Option<u16>,
        env: &ProfileEnvironment,
    ) -> Result<ProfileSelection, String> {
        let cli_revision = self.revision.as_deref().map(parse_revision).transpose()?;
        let named = self
            .profile
            .as_deref()
            .map(ServerSelection::parse)
            .transpose()?;
        let selection = if let Some(named) = named {
            if self.prod && named.target() != BotTarget::Prod {
                return Err("--prod conflicts with the selected local --profile".into());
            }
            if cli_revision.is_some_and(|rev| rev != named.revision()) {
                return Err("--revision conflicts with the selected --profile".into());
            }
            named
        } else {
            let env_named = env
                .profile
                .as_deref()
                .map(ServerSelection::parse)
                .transpose()?;
            let target = if self.prod {
                BotTarget::Prod
            } else {
                env_named.map(ServerSelection::target).unwrap_or_else(|| {
                    client::bot_target::bot_target_from_env(env.target.as_deref())
                })
            };
            let revision = if let Some(revision) = cli_revision {
                revision
            } else if let Some(named) = env_named {
                named.revision()
            } else if let Some(revision) = env.revision.as_deref() {
                parse_revision(revision)?
            } else if target == BotTarget::Prod {
                ClientRevision::R289
            } else if let Some(revision) = saved_revision {
                parse_revision(&revision.to_string())?
            } else {
                ClientRevision::R274
            };
            match (target, revision) {
                (BotTarget::Local, ClientRevision::R274) => ServerSelection::Local274,
                (BotTarget::Local, ClientRevision::R289) => ServerSelection::Local289,
                (BotTarget::Prod, ClientRevision::R274) => {
                    return Err("public revision 274 is unavailable; use revision 289".into())
                }
                (BotTarget::Prod, ClientRevision::R289) => ServerSelection::Public289,
            }
        };
        let home = if selection.target() == BotTarget::Prod {
            env.home
                .clone()
                .filter(|home| !home.as_os_str().is_empty())
                .ok_or("public-289 requires an operator home directory")?
        } else {
            env.home.clone().unwrap_or_default()
        };
        let bot_dir = home.join(".274bot");
        let public_worlds = if selection.target() == BotTarget::Prod {
            Some(Arc::new(PublicWorlds::load(&bot_dir.join("worlds.json"))?))
        } else {
            None
        };
        let is_289 = selection.revision() == ClientRevision::R289;
        let engine_dir = self
            .engine_dir
            .clone()
            .or_else(|| env.engine_dir.clone())
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
        let cache_dir = self.cache_dir.clone().unwrap_or_else(|| {
            if selection.target() == BotTarget::Prod {
                unpack_dir.clone()
            } else {
                engine_dir.join("data/pack/client")
            }
        });
        let game_host = self.host.clone().unwrap_or_else(|| {
            public_worlds.as_ref().map_or_else(
                || client::world_host_for(selection.target()).into(),
                |worlds| worlds.worlds[0].host.clone(),
            )
        });
        let game_port = self.port.unwrap_or_else(|| {
            public_worlds.as_ref().map_or(
                match selection {
                    ServerSelection::Local274 => 43594,
                    ServerSelection::Local289 => 44594,
                    ServerSelection::Public289 => 443,
                },
                |worlds| {
                    worlds
                        .worlds
                        .iter()
                        .find(|w| w.host == game_host)
                        .map_or(worlds.worlds[0].port, |w| w.port)
                },
            )
        });
        let asset_host = self.asset_host.clone().unwrap_or_else(|| {
            public_worlds
                .as_ref()
                .map_or_else(|| game_host.clone(), |worlds| worlds.worlds[0].host.clone())
        });
        let asset_port = self.http_port.unwrap_or_else(|| {
            public_worlds.as_ref().map_or(
                match selection {
                    ServerSelection::Local274 => 80,
                    ServerSelection::Local289 => 1080,
                    ServerSelection::Public289 => 443,
                },
                |worlds| {
                    worlds
                        .worlds
                        .iter()
                        .find(|w| w.host == asset_host)
                        .map_or(worlds.worlds[0].port, |w| w.port)
                },
            )
        });
        if selection.target() == BotTarget::Local {
            crate::validate_play_host(&game_host, BotTarget::Local).map_err(str::to_string)?;
            crate::validate_play_host(&asset_host, BotTarget::Local).map_err(str::to_string)?;
        } else if public_worlds.as_ref().is_none_or(|worlds| {
            worlds.by_endpoint(&game_host, game_port).is_none()
                || worlds.by_endpoint(&asset_host, asset_port).is_none()
        }) {
            return Err(
                "public-289 game/asset endpoints must appear in the configured worlds.json".into(),
            );
        }
        if game_port == 0 || asset_port == 0 {
            return Err("profile ports must be nonzero".into());
        }
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
        let vault_path = self.vault_path.clone().unwrap_or_else(|| {
            bot_dir.join(match selection {
                ServerSelection::Local274 => "vault",
                ServerSelection::Local289 => "vault-289",
                ServerSelection::Public289 => "vault-prod",
            })
        });
        let working_dir = env
            .working_dir
            .clone()
            .map(Ok)
            .unwrap_or_else(std::env::current_dir)
            .map_err(|e| format!("profile working directory: {e}"))?;
        let absolute = |path: PathBuf| {
            if path.is_absolute() {
                path
            } else {
                working_dir.join(path)
            }
        };
        let engine_dir = absolute(engine_dir);
        let guarded = parse_guarded_local_world(selection, game_port, &engine_dir);
        let world_members = world_members_from_guarded(self.world_members, guarded.as_ref());
        // Only the bundled public endpoints imply bundled content facts at
        // selection time. A custom listed endpoint must prove its content
        // identity when bound before receiving those facts.
        let bundled_public_endpoints = selection == ServerSelection::Public289
            && [(&game_host, game_port), (&asset_host, asset_port)]
                .into_iter()
                .all(|(host, port)| {
                    port == 443 && (host == "w1.rs2b2t.com" || host == "w2.rs2b2t.com")
                });
        let supported_server = (endpoint_flags_absent(self)
            && selection != ServerSelection::Public289)
            || bundled_public_endpoints
            || matching_local_world_supports_facts(
                selection,
                &game_host,
                &asset_host,
                asset_port,
                &world_members,
                guarded.as_ref(),
            );
        Ok(ProfileSelection {
            selection,
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

/// One guarded local `world.json` read. Public selections never match.
/// Revision, `node.port`, and a real `node.members` bool must agree with the
/// selected loopback profile. `web.port` is recorded for fact qualification
/// and is not required for WORLD membership.
struct GuardedLocalWorld {
    members: bool,
    web_port: Option<u64>,
    path: PathBuf,
    sha256: String,
    bytes: u64,
}

fn parse_guarded_local_world(
    selection: ServerSelection,
    game_port: u16,
    engine_dir: &Path,
) -> Option<GuardedLocalWorld> {
    let want_rev = match selection {
        ServerSelection::Local274 => 274u64,
        ServerSelection::Local289 => 289u64,
        ServerSelection::Public289 => return None,
    };
    let path = engine_dir.join("data/config/world.json");
    let text = std::fs::read_to_string(&path).ok()?;
    let value = serde_json::from_str::<serde_json::Value>(&text).ok()?;
    let engine = value.get("engine")?;
    let node = value.get("node")?;
    let revision = engine.get("revision").and_then(|v| v.as_u64())?;
    if revision != want_rev {
        return None;
    }
    let port = node.get("port").and_then(|v| v.as_u64())?;
    if port != u64::from(game_port) {
        return None;
    }
    let members = node.get("members").and_then(|v| v.as_bool())?;
    let web_port = value
        .get("web")
        .and_then(|web| web.get("port"))
        .and_then(|v| v.as_u64());
    let sha256 = format!("{:x}", Sha256::digest(text.as_bytes()));
    Some(GuardedLocalWorld {
        members,
        web_port,
        path,
        sha256,
        bytes: text.len() as u64,
    })
}

/// Guarded local world.json bind. NODE_MEMBERS is not applied. Public
/// profiles never inherit. Explicit `--world-members` wins and does not
/// become a fact-trust signal.
fn world_members_from_guarded(
    explicit: Option<bool>,
    guarded: Option<&GuardedLocalWorld>,
) -> WorldMembersFact {
    if let Some(members) = explicit {
        return WorldMembersFact::Known {
            members,
            source: WorldMembersSource::ExplicitOverride,
        };
    }
    let Some(world) = guarded else {
        return WorldMembersFact::Unknown;
    };
    WorldMembersFact::Known {
        members: world.members,
        source: WorldMembersSource::LocalWorldJson {
            path: world.path.clone(),
            sha256: world.sha256.clone(),
            bytes: world.bytes,
        },
    }
}

fn endpoint_flags_absent(options: &ProfileOptions) -> bool {
    options.host.is_none()
        && options.port.is_none()
        && options.asset_host.is_none()
        && options.http_port.is_none()
}

fn matching_local_world_supports_facts(
    selection: ServerSelection,
    game_host: &str,
    asset_host: &str,
    asset_port: u16,
    world_members: &WorldMembersFact,
    guarded: Option<&GuardedLocalWorld>,
) -> bool {
    selection.target() == BotTarget::Local
        && crate::is_loopback_host(game_host)
        && crate::is_loopback_host(asset_host)
        && matches!(
            world_members,
            WorldMembersFact::Known {
                source: WorldMembersSource::LocalWorldJson { .. },
                ..
            }
        )
        && guarded.and_then(|world| world.web_port) == Some(u64::from(asset_port))
}
