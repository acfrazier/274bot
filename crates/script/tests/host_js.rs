// Host JS living index.d.ts — our verbs, not rs2b0t names.
use script::host_js::{host_js_path, render_host_js_dts, write_host_js_dts};

#[test]
fn host_js_dts_is_fresh() {
    let path = host_js_path();
    let on_disk = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let rendered = render_host_js_dts();
    assert_eq!(
        on_disk, rendered,
        "host-js/index.d.ts is stale; run: cargo test -p script --test host_js regen_host_js -- --ignored"
    );
}

#[test]
fn host_js_dts_includes_required_interfaces() {
    let src = render_host_js_dts();
    assert!(src.contains("export interface FindOptions"));
    assert!(src.contains("allow_wilderness"));
    assert!(src.contains("wilderness"), "FindOptions JSDoc must mention wilderness exclusion");
    assert!(src.contains("export interface Camera"));
    assert!(src.contains("orbit_yaw"));
    assert!(src.contains("export interface ShopStockRow"));
    assert!(!src.contains("Game.teleport"), "must not export Game.teleport");
    assert!(!src.contains("teleport("), "must not export teleport string-table helper");
}

/// Writes `host-js/index.d.ts` from the host verb tables.
#[test]
#[ignore]
fn regen_host_js() {
    write_host_js_dts().expect("write host-js/index.d.ts");
    eprintln!("wrote {}", host_js_path().display());
}
