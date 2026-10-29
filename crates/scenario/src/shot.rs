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

/// Existing capture/recording product envelope: four KiB of UTF-8 per path.
/// This is a portable representation limit, not a measured fairness budget.
pub const OUTPUT_PATH_BYTES: usize = 4096;

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

/// The same normalized prefix without first copying an arbitrarily long label.
fn bounded_safe_label(label: &str) -> String {
    let src = if label.is_empty() { "shot" } else { label };
    let mut out = String::with_capacity(src.len().min(80));
    let mut pending_underscore = false;
    for ch in src.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            if pending_underscore {
                out.push('_');
                pending_underscore = false;
            }
            if out.len() < 80 {
                out.push(ch);
            }
        } else {
            pending_underscore = true;
        }
        if out.len() == 80 {
            return out;
        }
    }
    if pending_underscore {
        out.push('_');
    }
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
        _ => match client::operator_home() {
            Ok(home) => PathBuf::from(format!("{home}/.274bot/smoke")),
            Err(_) => PathBuf::from(".274bot/smoke"),
        },
    }
}

/// A per-run shot dir `<root>/<stamp>_<pid>` (377 `createShotRunDir`),
/// created once per panel run.
pub fn run_dir_path() -> PathBuf {
    let id = format!("{}_{}", stamp_utc(SystemTime::now()), std::process::id());
    default_shot_root().join(id)
}

/// Resolve a per-run directory under an explicit capture metadata allowance.
/// Environment lookup is configuration input; no owned path is copied until
/// its complete UTF-8 length has passed the supplied bound.
pub fn run_dir_path_with_limit(max_path_bytes: usize) -> io::Result<PathBuf> {
    let id = format!("{}_{}", stamp_utc(SystemTime::now()), std::process::id());
    match std::env::var(SHOT_ROOT_ENV) {
        Ok(root) if !root.is_empty() => bounded_child_path(Path::new(&root), &id, max_path_bytes),
        _ => match client::operator_home() {
            Ok(home) => {
                let root = bounded_child_path(Path::new(&home), ".274bot/smoke", max_path_bytes)?;
                bounded_child_path(&root, &id, max_path_bytes)
            }
            Err(_) => bounded_child_path(Path::new(".274bot/smoke"), &id, max_path_bytes),
        },
    }
}

/// Create and return the path for one headed run.
pub fn create_run_dir() -> io::Result<PathBuf> {
    let dir = run_dir_path();
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Resolve the PNG and JSON paths for one shot without touching the file system.
pub fn shot_paths_at(dir: &Path, label: &str, now: SystemTime) -> (PathBuf, PathBuf) {
    let stamp = stamp_utc(now);
    let label = safe_label(label);
    shot_paths_from_names(dir, &stamp, &label)
}

/// Resolve bounded shot paths after checking their UTF-8 byte lengths.
pub fn shot_paths_at_with_limit(
    dir: &Path,
    label: &str,
    now: SystemTime,
    max_path_bytes: usize,
) -> io::Result<(PathBuf, PathBuf)> {
    let stamp = stamp_utc(now);
    let label = bounded_safe_label(label);
    let base_bytes = stamp
        .len()
        .checked_add(1)
        .and_then(|bytes| bytes.checked_add(label.len()))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "shot path length overflow"))?;
    let png_bytes = bounded_path_bytes(dir, base_bytes + 4, max_path_bytes)?;
    let sidecar_bytes = bounded_path_bytes(dir, base_bytes + 5, max_path_bytes)?;
    Ok((
        copy_sized_path(dir, &format!("{stamp}_{label}.png"), png_bytes)?,
        copy_sized_path(dir, &format!("{stamp}_{label}.json"), sidecar_bytes)?,
    ))
}

/// Copy one child path only after both the caller allowance and product
/// representation limit admit its complete owned capacity.
pub fn bounded_child_path(
    dir: &Path,
    filename: &str,
    max_path_bytes: usize,
) -> io::Result<PathBuf> {
    let bytes = bounded_path_bytes(dir, filename.len(), max_path_bytes)?;
    copy_sized_path(dir, filename, bytes)
}

fn bounded_path_bytes(
    dir: &Path,
    filename_bytes: usize,
    max_path_bytes: usize,
) -> io::Result<usize> {
    let max_path_bytes = max_path_bytes.min(OUTPUT_PATH_BYTES);
    let directory = dir.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "shot output directory is not valid UTF-8",
        )
    })?;
    let separator_bytes = usize::from(
        !directory.is_empty()
            && !directory.ends_with(std::path::MAIN_SEPARATOR)
            && !(cfg!(windows) && directory.ends_with('/')),
    );
    let path_bytes = directory
        .len()
        .checked_add(separator_bytes)
        .and_then(|bytes| bytes.checked_add(filename_bytes))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "shot path length overflow"))?;
    if path_bytes > max_path_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "shot output path exceeds the supplied UTF-8 byte allowance",
        ));
    }
    Ok(path_bytes)
}

fn copy_sized_path(dir: &Path, filename: &str, path_bytes: usize) -> io::Result<PathBuf> {
    let mut path = PathBuf::new();
    path.try_reserve_exact(path_bytes)
        .map_err(io::Error::other)?;
    if path.capacity() > path_bytes {
        return Err(io::Error::new(
            io::ErrorKind::OutOfMemory,
            "shot path allocation exceeds its admitted capacity",
        ));
    }
    path.push(dir);
    path.push(filename);
    Ok(path)
}

fn shot_paths_from_names(dir: &Path, stamp: &str, label: &str) -> (PathBuf, PathBuf) {
    let base = format!("{stamp}_{label}");
    (
        dir.join(format!("{base}.png")),
        dir.join(format!("{base}.json")),
    )
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
    let (png_path, sidecar_path) = shot_paths_at(dir, label, now);
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
    img.save(&png_path).map_err(io::Error::other)?;
    std::fs::write(sidecar_path, snapshot_json)?;
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
    fn bounded_label_keeps_legacy_names_with_at_most_eighty_capacity_bytes() {
        for label in [
            String::new(),
            "walk courtyard".into(),
            "café".into(),
            "!hello!".into(),
            " ".repeat(4096),
            "x".repeat(4096),
            format!("{}  z", "a".repeat(79)),
            format!("{}  z", "a".repeat(78)),
        ] {
            let bounded = bounded_safe_label(&label);
            assert_eq!(bounded, safe_label(&label));
            assert!(bounded.capacity() <= 80);
        }
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
    fn shot_paths_at_uses_the_shared_stamp_and_safe_label() {
        let now = UNIX_EPOCH + Duration::from_secs(86_400);
        let (png, sidecar) = shot_paths_at(Path::new("shots"), "walk courtyard", now);
        assert_eq!(
            png,
            PathBuf::from("shots/1970-01-02T00-00-00_walk_courtyard.png")
        );
        assert_eq!(
            sidecar,
            PathBuf::from("shots/1970-01-02T00-00-00_walk_courtyard.json")
        );
    }

    #[test]
    fn bounded_shot_paths_accept_exact_limit_and_reject_oversized_prefix() {
        let now = UNIX_EPOCH + Duration::from_secs(86_400);
        let label = "x";
        let suffix_bytes = stamp_utc(now).len() + 1 + safe_label(label).len() + 5;
        let directory_bytes = 4096 - 1 - suffix_bytes;
        let directory = PathBuf::from("d".repeat(directory_bytes));
        let (png, sidecar) =
            shot_paths_at_with_limit(&directory, label, now, 4096).expect("exact limit");
        assert_eq!(png.to_str().expect("UTF-8 PNG path").len(), 4095);
        assert_eq!(sidecar.to_str().expect("UTF-8 sidecar path").len(), 4096);
        assert!(png.capacity() <= 4096);
        assert!(sidecar.capacity() <= 4096);
        assert_eq!((png, sidecar), shot_paths_at(&directory, label, now));

        let oversized = PathBuf::from(format!("d{}", "d".repeat(directory_bytes)));
        let error = shot_paths_at_with_limit(&oversized, label, now, 4096)
            .expect_err("oversized path is rejected");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn bounded_directory_join_preserves_names_without_capacity_growth() {
        for directory in [
            String::new(),
            "shots".into(),
            "shots/".into(),
            "d".repeat(3000),
            "é".repeat(1500),
        ] {
            let directory = Path::new(&directory);
            let expected = directory.join("run-name");
            let maximum = expected.to_str().unwrap().len();
            let actual =
                bounded_child_path(directory, "run-name", maximum).expect("exact allowance");
            assert_eq!(actual, expected);
            assert!(actual.capacity() <= maximum);
            assert_eq!(
                bounded_child_path(directory, "run-name", maximum - 1)
                    .expect_err("one byte below the complete path is refused")
                    .kind(),
                io::ErrorKind::InvalidInput
            );
        }
        assert_eq!(
            run_dir_path_with_limit(0)
                .expect_err("no directory fits a zero-byte allowance")
                .kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[cfg(unix)]
    #[test]
    fn bounded_shot_paths_reject_non_utf8_directory() {
        use std::os::unix::ffi::OsStringExt;

        let directory = PathBuf::from(std::ffi::OsString::from_vec(vec![0xff]));
        let error = shot_paths_at_with_limit(&directory, "shot", UNIX_EPOCH, 4096)
            .expect_err("non-UTF-8 path is rejected");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("not valid UTF-8"));
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
