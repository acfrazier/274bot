//! Validate a launch profile and its real shared client without starting slots.
//! Pass --initialize to fetch/validate assets through the bound HTTP endpoint.
use std::sync::Arc;

fn check() -> Result<serde_json::Value, String> {
    let (options, rest) = host_play::parse_profile_args(std::env::args().skip(1))?;
    if rest.iter().any(|arg| arg != "--initialize") {
        return Err("profile_check accepts shared profile flags and optional --initialize".into());
    }
    let profile = options.resolve(None)?.bind()?;
    let template = host_play::SharedClientTemplate::load(Arc::clone(&profile))?;
    let mut client = template.prepare_client(0, false)?;
    let initialize = rest.iter().any(|arg| arg == "--initialize");
    if initialize {
        client.maininit();
        if client.error_loading {
            return Err(format!(
                "asset initialization failed: {}",
                client.last_progress_message
            ));
        }
    }
    Ok(serde_json::json!({
        "profile": profile.label(),
        "revision": client.revision().as_i32(),
        "cache_id": profile.cache_id(),
        "cache_dir": profile.client().cache_dir(),
        "asset_endpoint": format!("{}:{}", profile.client().asset_host(), profile.client().asset_port()),
        "shared_binding": Arc::ptr_eq(client.session_profile().ok_or("missing client binding")?, profile.client()),
        "shared_cache": client.cache_from_shared,
        "navigation": format!("{:?}", profile.nav_availability()),
        "initialized": initialize,
        "gameplay_started": false,
        "bot_operation_error": profile.require_bot_operation().err(),
    }))
}

fn main() {
    match check() {
        Ok(result) => println!("{}", serde_json::to_string_pretty(&result).unwrap()),
        Err(error) => {
            eprintln!("FAIL: {error}");
            std::process::exit(1);
        }
    }
}
