//! Whole-window shot files (the 377 harness pattern): per-run dir under
//! `~/.274bot/smoke/<runId>/`, files `<stamp>_<safeLabel>.png` +
//! `<stamp>_<safeLabel>.json`. Pure and shared so both runners (headed
//! panel, headless e2e) and the unit tests use the same naming.

use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Env override for the shot root (mirrors the 377 `DEFAULT_SHOT_DIR`
/// env pattern).
pub const SHOT_ROOT_ENV: &str = "274BOT_SMOKE_DIR";

/// 377 `safeLabel`: collapse every maximal run of chars outside
/// `[a-zA-Z0-9._-]` to a single `_`, then cap at 80. The sanitized
/// output is ASCII by construction, so `truncate(80)` cannot split a
/// char.
pub fn safe_label(label: &str) -> String {
    let src = if label.is_empty() { "shot" } else { label };
    let mut out = String::with_capacity(src.len());
    let mut pending_underscore = false;
    for ch in src.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            if pending_underscore {
                out.push('_');
                pending_underscore = false;
            }
            out.push(ch);
        } else {
            pending_underscore = true;
        }
    }
    if pending_underscore {
        out.push('_');
    }
    out.truncate(80);
    out
}

/// UTC `YYYY-MM-DDTHH-MM-SS`: the 377 shot stamp
/// (`toISOString().replace(/[:.]/g, '-').slice(0, 19)`).
pub fn stamp_utc(now: SystemTime) -> String {
    let secs = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
    let days = secs.div_euclid(86_400);
    let secs_of_day = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let (hh, mm, ss) = (secs_of_day / 3600, secs_of_day / 60 % 60, secs_of_day % 60);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}-{mm:02}-{ss:02}")
}

/// Days since epoch → `(year, month, day)` (Howard Hinnant's
/// `civil_from_days`; no time crate in the tree).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

/// The shot root: `$274BOT_SMOKE_DIR` when set, else `~/.274bot/smoke`.
pub fn default_shot_root() -> PathBuf {
    match std::env::var(SHOT_ROOT_ENV) {
        Ok(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => match std::env::var("HOME") {
            Ok(home) => PathBuf::from(format!("{home}/.274bot/smoke")),
            Err(_) => PathBuf::from(".274bot/smoke"),
        },
    }
}

/// A per-run shot dir `<root>/<stamp>_<pid>` (377 `createShotRunDir`),
/// created once per panel run.
pub fn create_run_dir() -> io::Result<PathBuf> {
    let id = format!("{}_{}", stamp_utc(SystemTime::now()), std::process::id());
    let dir = default_shot_root().join(id);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Write one shot into `dir`: `<stamp>_<safeLabel>.png` from the RGBA8
/// buffer plus the `.json` sidecar, stamped at `now` (injected so tests
/// see exact names). Returns the PNG path (the 377 `screenshotPage`
/// contract).
pub fn write_shot_at(
    dir: &Path,
    label: &str,
    rgba: &[u8],
    width: u32,
    height: u32,
    snapshot_json: &str,
    now: SystemTime,
) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let base = format!("{}_{}", stamp_utc(now), safe_label(label));
    let png_path = dir.join(format!("{base}.png"));
    let img = image::RgbaImage::from_raw(width, height, rgba.to_vec()).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "shot {label}: rgba buffer is {} bytes, need {}",
                rgba.len(),
                4 * width * height
            ),
        )
    })?;
    img.save(&png_path)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    std::fs::write(dir.join(format!("{base}.json")), snapshot_json)?;
    Ok(png_path)
}

/// [`write_shot_at`] with the current time.
pub fn write_shot(
    dir: &Path,
    label: &str,
    rgba: &[u8],
    width: u32,
    height: u32,
    snapshot_json: &str,
) -> io::Result<PathBuf> {
    write_shot_at(
        dir,
        label,
        rgba,
        width,
        height,
        snapshot_json,
        SystemTime::now(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "274bot-shot-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn safe_label_sanitizes_like_377() {
        assert_eq!(safe_label("walk courtyard"), "walk_courtyard");
        assert_eq!(safe_label("chef/door.stall_2"), "chef_door.stall_2");
        assert_eq!(safe_label("!hello!"), "_hello_");
        assert_eq!(safe_label("a b c"), "a_b_c");
        assert_eq!(safe_label("  "), "_");
        assert_eq!(safe_label(""), "shot");
        assert_eq!(safe_label("café"), "caf_");
    }

    #[test]
    fn safe_label_caps_at_80() {
        assert_eq!(safe_label(&"x".repeat(120)).len(), 80);
        let already = "y".repeat(80);
        assert_eq!(safe_label(&already), already);
        // A truncation lands on the dash boundary, not mid-char.
        assert_eq!(
            safe_label(&format!("{}-b", "a".repeat(79))),
            format!("{}-", "a".repeat(79))
        );
    }

    #[test]
    fn stamp_utc_is_the_377_stamp() {
        // 1970-01-02T00:00:00Z
        assert_eq!(
            stamp_utc(UNIX_EPOCH + Duration::from_secs(86_400)),
            "1970-01-02T00-00-00"
        );
        // 2026-08-25T00:00:00Z (day 20690)
        assert_eq!(
            stamp_utc(UNIX_EPOCH + Duration::from_secs(1_787_616_000)),
            "2026-08-25T00-00-00"
        );
        // The seconds part survives the colon/dot → dash stamping.
        assert_eq!(
            stamp_utc(UNIX_EPOCH + Duration::from_secs(1_787_616_000 + 80_501)),
            "2026-08-25T22-21-41"
        );
    }

    #[test]
    fn default_shot_root_prefers_the_env_override() {
        std::env::set_var(SHOT_ROOT_ENV, "/tmp/274bot-shots-test");
        assert_eq!(default_shot_root(), PathBuf::from("/tmp/274bot-shots-test"));
        std::env::remove_var(SHOT_ROOT_ENV);
        // Home fallback: `~/.274bot/smoke` (the 377 DEFAULT_SHOT_DIR).
        assert!(default_shot_root().ends_with(".274bot/smoke"));
    }

    #[test]
    fn write_shot_at_writes_png_and_json_sidecar() {
        let dir = temp_dir("write");
        let rgba = vec![
            0, 255, 0, 255, // 2x2 RGBA
            255, 0, 0, 255, //
            0, 0, 255, 255, //
            255, 255, 0, 255, //
        ];
        let json = r#"{"tile":[3220,3220,0]}"#;
        let png = write_shot_at(
            &dir,
            "walk courtyard",
            &rgba,
            2,
            2,
            json,
            UNIX_EPOCH + Duration::from_secs(1_787_616_000),
        )
        .expect("shot writes");
        assert_eq!(png, dir.join("2026-08-25T00-00-00_walk_courtyard.png"));
        let header = std::fs::read(&png).unwrap();
        assert_eq!(&header[..8], b"\x89PNG\r\n\x1a\n");
        let sidecar =
            std::fs::read_to_string(dir.join("2026-08-25T00-00-00_walk_courtyard.json")).unwrap();
        assert_eq!(sidecar, json);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_shot_rejects_a_mismatched_rgba_buffer() {
        let dir = temp_dir("bad");
        let err = write_shot_at(&dir, "bad", &[0u8; 3], 2, 2, "{}", SystemTime::now());
        assert!(
            err.is_err(),
            "a 3-byte buffer for a 2x2 RGBA shot is invalid"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
