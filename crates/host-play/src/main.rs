//! `host-play` CLI: resolve and bind one immutable server profile, load its
//! shared client template, then unlock the selected vault and run its accounts.

use std::env;
use std::process::ExitCode;

use host_play::{
    open_vault, parse_profile_args, profile_password_for, run_with_template, set_debug,
    ProfileOptions, SharedClientTemplate,
};
use vault::{Profile, ProfileSettings, VaultError};

#[derive(Debug)]
struct Args {
    profile: ProfileOptions,
    pass: Option<String>,
    users: Vec<String>,
    lowmem: bool,
    mainland: bool,
}

fn usage() -> ! {
    eprintln!(
        "usage: host-play [--profile local-274|local-289|public-289] \
         [--revision 274|289] [--prod] [--host HOST] [--port PORT] \
         [--asset-host HOST] [--http-port PORT] [--engine PATH] \
         [--cache DIR] [--unpack DIR] [--nav-pack PATH] [--nav-flags PATH] \
         [--content DIR] [--vault PATH] [--catalog DIR] [--cache-manifest PATH] \
         [--vault-pass PASS] [--lowmem|--highmem] [--mainland] [--debug] \
         [--user USER]... (default user: test)"
    );
    std::process::exit(2);
}

fn parse_args_from<I>(args: I) -> Result<Args, String>
where
    I: IntoIterator<Item = String>,
{
    let (profile, remaining) = parse_profile_args(args)?;
    let mut parsed = Args {
        profile,
        pass: env::var("BOT_VAULT_PASS").ok(),
        users: Vec::new(),
        lowmem: true,
        mainland: env::var("BOT_MAINLAND").as_deref() == Ok("1"),
    };
    let mut it = remaining.into_iter();
    while let Some(arg) = it.next() {
        let mut value = || {
            it.next()
                .ok_or_else(|| format!("host-play: missing value for {arg}"))
        };
        match arg.as_str() {
            "--vault-pass" => parsed.pass = Some(value()?),
            "--lowmem" => parsed.lowmem = true,
            "--highmem" => parsed.lowmem = false,
            "--user" => parsed.users.push(value()?),
            "--mainland" => parsed.mainland = true,
            "--debug" => set_debug(true),
            "--help" | "-h" => usage(),
            _ => return Err(format!("host-play: unknown argument: {arg}")),
        }
    }
    if parsed.users.is_empty() {
        parsed.users.push("test".into());
    }
    Ok(parsed)
}

fn main() -> ExitCode {
    let args = match parse_args_from(env::args().skip(1)) {
        Ok(args) => args,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::FAILURE;
        }
    };

    // Resolve, bind and decode before opening or creating a vault. A 289
    // profile remains constructible, but the host boundary refuses gameplay
    // before any credential or filesystem mutation.
    let selection = match args.profile.resolve(None) {
        Ok(selection) => selection,
        Err(msg) => {
            eprintln!("host-play: server profile: {msg}");
            return ExitCode::FAILURE;
        }
    };
    let profile = match selection.bind() {
        Ok(profile) => profile,
        Err(msg) => {
            eprintln!("host-play: server profile: {msg}");
            return ExitCode::FAILURE;
        }
    };
    let template = match SharedClientTemplate::load(profile.clone()) {
        Ok(template) => template,
        Err(msg) => {
            eprintln!("host-play: client assets: {msg}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(msg) = profile.require_bot_operation() {
        eprintln!("host-play: {msg}");
        return ExitCode::FAILURE;
    }

    let Some(pass) = args.pass else {
        eprintln!("host-play: no vault passphrase (set BOT_VAULT_PASS or --vault-pass)");
        return ExitCode::FAILURE;
    };
    let vault_path = profile.vault_path().to_path_buf();
    let mut vault = match open_vault(&vault_path, &pass) {
        Ok(vault) => vault,
        Err(e) => {
            match e {
                VaultError::WrongPassphrase => eprintln!("host-play: wrong passphrase"),
                VaultError::Corrupt(msg) => eprintln!("host-play: corrupt vault: {msg}"),
                VaultError::EmptyPassphrase => eprintln!("host-play: empty passphrase"),
                other => eprintln!("host-play: vault {}: {other}", vault_path.display()),
            }
            return ExitCode::FAILURE;
        }
    };

    let mut profiles = Vec::new();
    for (i, username) in args.users.iter().enumerate() {
        match vault.get(username) {
            Some(existing) => profiles.push(existing.clone()),
            None => {
                let account = Profile {
                    username: username.clone(),
                    password: profile_password_for(username, profile.target()),
                    uid: 274_000_000 + i as i32 + 1,
                    settings: ProfileSettings::default(),
                };
                if vault.upsert(account.clone()).is_err() {
                    eprintln!("host-play: could not write vault {}", vault_path.display());
                    return ExitCode::FAILURE;
                }
                profiles.push(account);
            }
        }
    }

    if !args.lowmem {
        for account in profiles.iter_mut() {
            account.settings.lowmem = false;
        }
    }
    if host_play::debug_enabled() {
        eprintln!(
            "host-play: running {} profile(s) via {}",
            profiles.len(),
            profile.label()
        );
    }
    let play = match run_with_template(
        template,
        args.mainland,
        profiles,
        |_| (None, None),
        |_, _, _| {},
    ) {
        Ok(play) => play,
        Err(msg) => {
            eprintln!("host-play: {msg}");
            return ExitCode::FAILURE;
        }
    };
    play.join();
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::parse_args_from;
    use client::client::ClientRevision;

    fn args(values: &[&str]) -> Result<super::Args, String> {
        parse_args_from(values.iter().map(|value| (*value).to_string()))
    }

    #[test]
    fn shared_profile_parser_keeps_explicit_overrides_order_independent() {
        let parsed = args(&[
            "--cache",
            "/tmp/host-play-explicit-cache",
            "--prod",
            "--user",
            "alice",
        ])
        .unwrap();
        let selection = parsed.profile.resolve(None).unwrap();
        assert_eq!(selection.game_host(), "w1.rs2b2t.com");
        assert_eq!(
            selection.cache_dir(),
            std::path::Path::new("/tmp/host-play-explicit-cache")
        );
        assert_eq!(selection.revision(), ClientRevision::R289);
        assert_eq!(parsed.users, ["alice"]);
    }

    #[test]
    fn shared_profile_parser_rejects_invalid_revision_and_port() {
        assert!(args(&["--revision", "275"]).is_err());
        assert!(args(&["--port", "nope"]).is_err());
    }
}
