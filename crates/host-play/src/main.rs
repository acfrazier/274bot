//! `host-play` CLI: resolve and bind one immutable server profile, load its
//! shared client template, then unlock the selected vault and run its accounts.

use std::env;
use std::process::ExitCode;

use host_play::passphrase::{self, Purpose};
use host_play::{
    open_vault, parse_profile_args, profile_password, run_with_template, set_debug, ProfileOptions,
};
use vault::{Profile, ProfileSettings, VaultError};

#[derive(Debug)]
struct Args {
    profile: ProfileOptions,
    /// `--vault-pass-stdin`: read the passphrase from a piped stdin. The
    /// passphrase itself is never an argument or an environment variable.
    pass_stdin: bool,
    users: Vec<String>,
    lowmem: bool,
    mainland: bool,
}

fn usage() -> ! {
    eprintln!(
        "usage: host-play [--profile NAME|--rs2b2t] \
         [--revision 274|289] [--host HOST] [--port PORT] \
         [--asset-host HOST] [--http-port PORT] [--engine PATH] \
         [--cache DIR] [--unpack DIR] [--nav-pack PATH] [--nav-flags PATH] \
         [--content DIR] [--vault PATH] [--catalog DIR] [--cache-manifest PATH] \
         [--world-members true|false] \
         [--vault-pass-stdin] [--lowmem|--highmem] [--mainland] [--debug] \
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
        pass_stdin: false,
        users: Vec::new(),
        lowmem: true,
        mainland: env::var("BOT_MAINLAND").as_deref() == Ok("1"),
    };
    let mut it = remaining.into_iter();
    while let Some(arg) = it.next() {
        if passphrase::is_removed_flag(&arg) {
            return Err(passphrase::removed_flag_error("host-play"));
        }
        let mut value = || {
            it.next()
                .ok_or_else(|| format!("host-play: missing value for {arg}"))
        };
        match arg.as_str() {
            "--vault-pass-stdin" => parsed.pass_stdin = true,
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
    passphrase::warn_legacy_env("host-play");
    // Before any slot starts: macOS defaults the soft limit to 256 open files.
    if let Some(line) = host_play::fd_limit::describe(&host_play::fd_limit::raise_open_file_limit())
    {
        api::host_log!(stderr; api::hostlog::Category::Lifecycle, api::hostlog::Level::Info, "{line}");
    }

    // Resolve, prepare and decode before opening or creating a vault. A 289
    // profile remains constructible, but the host boundary refuses gameplay
    // before any credential or filesystem mutation.
    let selection = match args.profile.resolve(None) {
        Ok(selection) => selection,
        Err(msg) => {
            eprintln!("host-play: server profile: {msg}");
            return ExitCode::FAILURE;
        }
    };
    let template = match selection.prepare_template() {
        Ok(template) => template,
        Err(msg) => {
            eprintln!("host-play: server profile: {msg}");
            return ExitCode::FAILURE;
        }
    };
    let profile = template.profile().clone();
    if let Err(msg) = profile.require_bot_operation() {
        eprintln!("host-play: {msg}");
        return ExitCode::FAILURE;
    }

    let vault_path = profile.vault_path().to_path_buf();
    let pass = match passphrase::obtain(
        "host-play",
        args.pass_stdin,
        Purpose::for_vault(vault_path.is_file()),
    ) {
        Ok(pass) => pass,
        Err(msg) => {
            eprintln!("host-play: {msg}");
            return ExitCode::FAILURE;
        }
    };
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
    // The passphrase has done its job; do not keep it through the long run.
    drop(pass);

    let mut profiles = Vec::new();
    for (i, username) in args.users.iter().enumerate() {
        match vault.get(username) {
            Some(existing) => profiles.push(existing.clone()),
            None => {
                let account = Profile {
                    username: username.clone(),
                    password: profile_password(username).into(),
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
    // The vault is only needed to seed the profiles: release its derived key
    // before the long run.
    drop(vault);

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
        let home =
            std::env::temp_dir().join(format!("274bot-host-play-parser-{}", std::process::id()));
        if home.exists() {
            std::fs::remove_dir_all(&home).unwrap();
        }
        let cache = home.join("host-play-explicit-cache");
        let cache_arg = cache.to_string_lossy().into_owned();
        let parsed = args(&["--cache", &cache_arg, "--rs2b2t", "--user", "alice"]).unwrap();
        let selection = parsed
            .profile
            .resolve_with_env(
                None,
                &host_play::profile::ProfileEnvironment {
                    home: Some(home.clone()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(selection.game_host(), "w1.rs2b2t.com");
        assert_eq!(selection.cache_dir(), cache);
        assert_eq!(selection.revision(), ClientRevision::R289);
        assert_eq!(parsed.users, ["alice"]);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn shared_profile_parser_rejects_invalid_revision_and_port() {
        assert!(args(&["--revision", "275"]).is_err());
        assert!(args(&["--port", "nope"]).is_err());
    }

    #[test]
    fn the_passphrase_is_never_an_argument_and_is_never_echoed_back() {
        for spelling in [
            &["--vault-pass", "hunter2-hunter2"][..],
            &["--vault-pass=hunter2-hunter2"][..],
            &["--user", "alice", "--vault-pass", "hunter2-hunter2"][..],
        ] {
            let error = args(spelling).unwrap_err();
            assert!(error.contains("--vault-pass-stdin"), "{error}");
            assert!(
                !error.contains("hunter2"),
                "the value must not be echoed: {error}"
            );
        }
    }

    #[test]
    fn only_the_stdin_flag_selects_the_pipe_channel() {
        assert!(!args(&["--user", "alice"]).unwrap().pass_stdin);
        assert!(args(&["--vault-pass-stdin"]).unwrap().pass_stdin);
    }
}
