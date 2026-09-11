use std::sync::{Arc, Mutex};
use host_play::{parse_profile_args, SharedClientTemplate};
use host_play::progress::{ProfileProgressObserver, ProfileProgressStage};

fn main() {
    if let Err(error) = run() {
        eprintln!("FAIL: nav_origin_probe: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let (options, rest) = parse_profile_args(std::env::args().skip(1))?;
    if !rest.is_empty() { return Err(format!("unknown args {rest:?}")); }
    let events = Arc::new(Mutex::new(Vec::new()));
    let recording = events.clone();
    let observer = ProfileProgressObserver::new(move |p| {
        recording.lock().unwrap().push(p);
        if p.completed == 0 || p.completed == p.total {
            println!("{}", serde_json::json!({"phase":"progress","stage":p.stage.description(),"completed":p.completed,"total":p.total}));
        }
    });
    let selected = options.resolve(None)?;
    let profile = selected.bind_with_progress(&observer)?;
    let world = profile.world().ok_or("no selected world")?;
    let counts = profile.nav_load_counters();
    let expected = std::env::var("PROBE_EXPECT_ORIGIN").map_err(|_| "PROBE_EXPECT_ORIGIN required")?;
    let bundled = profile.nav_origin().is_bundled();
    if bundled != (expected == "bundled") { return Err(format!("unexpected origin {:?}", profile.nav_origin())); }
    if counts.pack_reads != 1 || counts.pack_decodes != 1 || counts.pack_hashes != u32::from(!bundled) {
        return Err(format!("bad counts {counts:?}"));
    }
    let template = SharedClientTemplate::load_with_progress(profile.clone(), &observer)?;
    let same_world = Arc::ptr_eq(&world, &template.world().ok_or("no template world")?);
    if !same_world { return Err("template copied/replaced the bound world".into()); }
    template.validate_for_play_with_progress(&observer)?;
    if profile.nav_load_counters() != counts { return Err("validation changed nav counters".into()); }
    let mut preserved_after_edit = None;
    let mut new_bind_refusal = None;
    if std::env::var_os("PROBE_MUTATE_OWNED_PACK").is_some() {
        let path = profile.nav_pack();
        if !path.to_string_lossy().contains("/.superpowers/navigation-proof/") {
            return Err("mutation is limited to owned navigation-proof copies".into());
        }
        let original = std::fs::read(path).map_err(|e| e.to_string())?;
        let mut changed = original.clone();
        changed[0] ^= 0xff;
        std::fs::write(path, &changed).map_err(|e| e.to_string())?;
        let existing = template.validate_for_play();
        let next = selected.bind();
        std::fs::write(path, &original).map_err(|e| format!("restore owned pack: {e}"))?;
        existing?;
        let reason = next.err().ok_or("new bind accepted a corrupt owned pack")?;
        preserved_after_edit = Some(Arc::ptr_eq(&world, &template.world().ok_or("lost world")?));
        new_bind_refusal = Some(reason);
    }
    let events = events.lock().unwrap();
    let hash_starts = events.iter().filter(|p| p.stage == ProfileProgressStage::CheckingNavigationFiles && p.completed == 0).count();
    if hash_starts != usize::from(!bundled) { return Err(format!("unexpected hash-stage starts {hash_starts}")); }
    println!("PASS: nav_origin_probe: {}", serde_json::json!({
        "revision":profile.revision().as_i32(),"cache_id":profile.cache_id(),
        "origin":expected,"pack":profile.nav_pack(),"pack_reads":counts.pack_reads,
        "pack_hashes":counts.pack_hashes,"pack_decodes":counts.pack_decodes,
        "hash_stage_starts":hash_starts,"same_world_arc":same_world,
        "preserved_after_disk_edit":preserved_after_edit,"new_bind_refusal":new_bind_refusal
    }));
    Ok(())
}
