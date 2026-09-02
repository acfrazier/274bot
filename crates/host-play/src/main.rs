//! `host-play` CLI: unlock a vault (passphrase from `BOT_VAULT_PASS` or
//! `--vault-pass`), load the named profiles, and run them through the host
//! kernel until the process is stopped. Missing vault/profiles are created
//! with target-aware passwords (local: `password = username` for auto-register;
//! prod: high-entropy secret).

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use host_play::{open_vault, profile_password, run, set_debug, PlayOptions};
use vault::{Profile, ProfileSettings, VaultError};

const DEFAULT_PORT: u16 = 43594;

fn default_cache_dir() -> String {
    client::cache_dir().display().to_string()
}

fn default_vault() -> PathBuf {
    match env::var("HOME") {
        Ok(home) => PathBuf::from(format!("{home}/.274bot/vault")),
        Err(_) => PathBuf::from(".274bot/vault"),
    }
}

struct Args {
    vault: PathBuf,
    pass: Option<String>,
    host: String,
    port: u16,
    cache: String,
    users: Vec<String>,
    lowmem: bool,
    mainland: bool,
}

fn usage() -> ! {
    eprintln!(
        "usage: host-play [--vault PATH] [--vault-pass PASS] \
         [--host HOST] [--port PORT] [--cache DIR] [--lowmem|--highmem] \
         [--mainland] [--debug] [--user USER]... (default user: test)"
    );
    std::process::exit(2);
}

fn value(it: &mut std::iter::Skip<env::Args>) -> String {
    it.next().unwrap_or_else(|| usage())
}

fn parse_args() -> Args {
    let mut args = Args {
        vault: default_vault(),
        pass: env::var("BOT_VAULT_PASS").ok(),
        host: host_play::default_world_host(),
        port: DEFAULT_PORT,
        cache: default_cache_dir(),
        users: Vec::new(),
        lowmem: true,
        mainland: env::var("BOT_MAINLAND").as_deref() == Ok("1"),
    };
    let mut it = env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--vault" => args.vault = PathBuf::from(value(&mut it)),
            "--vault-pass" => args.pass = Some(value(&mut it)),
            "--host" => args.host = value(&mut it),
            "--port" => args.port = value(&mut it).parse().unwrap_or_else(|_| usage()),
            "--cache" => args.cache = value(&mut it),
            "--lowmem" => args.lowmem = true,
            "--highmem" => args.lowmem = false,
            "--user" => args.users.push(value(&mut it)),
            "--mainland" => args.mainland = true,
            "--debug" => set_debug(true),
            "--prod" => {
                client::set_bot_target(client::BotTarget::Prod);
                args.host = host_play::default_world_host();
            }
            "--help" | "-h" => usage(),
            _ => usage(),
        }
    }
    if args.users.is_empty() {
        args.users.push("test".into());
    }
    args
}

fn main() -> ExitCode {
    let args = parse_args();
    let Some(pass) = args.pass else {
        eprintln!("host-play: no vault passphrase (set BOT_VAULT_PASS or --vault-pass)");
        return ExitCode::FAILURE;
    };

    let mut vault = match open_vault(&args.vault, &pass) {
        Ok(v) => v,
        Err(e) => {
            match e {
                VaultError::WrongPassphrase => {
                    eprintln!("host-play: wrong passphrase");
                }
                VaultError::Corrupt(msg) => {
                    eprintln!("host-play: corrupt vault: {msg}");
                }
                VaultError::EmptyPassphrase => {
                    eprintln!("host-play: empty passphrase");
                }
                other => {
                    eprintln!("host-play: vault {}: {other}", args.vault.display());
                }
            }
            return ExitCode::FAILURE;
        }
    };

    let mut profiles = Vec::new();
    for (i, username) in args.users.iter().enumerate() {
        match vault.get(username) {
            Some(p) => profiles.push(p.clone()),
            None => {
                let profile = Profile {
                    username: username.clone(),
                    password: profile_password(username),
                    uid: 274_000_000 + i as i32 + 1,
                    settings: ProfileSettings::default(),
                };
                if vault.upsert(profile.clone()).is_err() {
                    eprintln!("host-play: could not write vault {}", args.vault.display());
                    return ExitCode::FAILURE;
                }
                profiles.push(profile);
            }
        }
    }

    // `--highmem` runs every selected profile with lowmem off for this
    // session only; the vault blobs are left untouched.
    if !args.lowmem {
        for p in profiles.iter_mut() {
            p.settings.lowmem = false;
        }
    }

    if host_play::debug_enabled() {
        eprintln!(
            "host-play: running {} profile(s) via {}",
            profiles.len(),
            args.host
        );
    }
    let play = run(
        &PlayOptions {
            host: args.host,
            port: args.port,
            cache_dir: args.cache,
            lowmem: args.lowmem,
            mainland: args.mainland,
        },
        profiles,
    );
    play.join();
    ExitCode::SUCCESS
}
