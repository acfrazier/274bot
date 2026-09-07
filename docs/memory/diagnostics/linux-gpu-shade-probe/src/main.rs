//! Bounded Linux GPU textured-shade forensic probe.
//! Outside production sources. Does not modify client tests or shaders.
//!
//! Reports: adapter identity, mesh packed shade histogram, scene pixel
//! histograms for boundary shades, and the CPU-side expected block factors.

use client::config::Cache;
use client::core::World;
use client::dash3d::{SceneModel, TerrainOverlayShape};
use client::graphics::{Pix3D, Pix3DDraw, Pix8};
use client::render::backend::GpuBackend;
use client::render::world::GpuVertex;
use client::render::RenderWorld;
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

const GROUND_SHADE: i32 = 200 * 128 + 100;
const TEXTURE_RED: i32 = 7;

fn main() {
    let out_dir = env::var("PROBE_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/work/docs/memory/diagnostics/linux-gpu-shade-out"));
    let _ = fs::create_dir_all(&out_dir);
    let report_path = out_dir.join("probe-report.txt");
    let mut report = fs::File::create(&report_path).expect("create report");

    macro_rules! both {
        ($($t:tt)*) => {{
            let s = format!($($t)*);
            println!("{s}");
            let _ = writeln!(report, "{s}");
        }};
    }

    both!("=== linux-gpu-shade-probe ===");
    both!("time_utc={}", chrono_like_now());
    both!("cwd={:?}", env::current_dir().ok());
    both!("SKIP_GPU={:?}", env::var("SKIP_GPU"));
    both!("BOT_CPU={:?}", env::var("BOT_CPU"));
    both!("WGPU_BACKEND={:?}", env::var("WGPU_BACKEND"));
    both!("DISPLAY={:?}", env::var("DISPLAY"));
    both!("VK_ICD_FILENAMES={:?}", env::var("VK_ICD_FILENAMES"));
    both!("LIBGL_ALWAYS_SOFTWARE={:?}", env::var("LIBGL_ALWAYS_SOFTWARE"));
    both!("XDG_RUNTIME_DIR={:?}", env::var("XDG_RUNTIME_DIR"));

    emit_adapter_info(&mut report);

    // Expected block math (CPU-side mirror of shader).
    both!("--- expected block factors (shader mirror) ---");
    for shade in [0, 15, 16, 17, 31, 32, 33, 47, 48, 49, 63, 64, 65, 79, 80, 95, 96, 111, 112, 127]
    {
        let s = (shade as u32) & 0x7f;
        let block = (s >> 4) & 3;
        let block_factor = [1.0f32, 0.875, 0.75, 0.625][block as usize];
        let factor = block_factor * if (s >> 6) == 1 { 0.5 } else { 1.0 };
        let expected_red = (255.0 * factor).round() as i32;
        both!(
            "shade={shade:3} s={s:3} block={block} factor={factor:.4} expected_red~{expected_red}"
        );
    }

    Pix3D::init_colour_table(0.6);

    let shades: Vec<i32> = env::var("PROBE_SHADES")
        .ok()
        .map(|s| {
            s.split(',')
                .filter_map(|p| p.trim().parse().ok())
                .collect()
        })
        .filter(|v: &Vec<i32>| !v.is_empty())
        .unwrap_or_else(|| {
            vec![0, 15, 16, 17, 31, 32, 33, 47, 48, 49, 63, 64, 65, 80, 96, 112]
        });

    both!("--- mesh shade packing (CPU only) ---");
    for &shade in &shades {
        let mut pix = textured_pix();
        let mesh = build_mesh(shade, &mut pix);
        let verts = mesh.vertices();
        let mut tex_shade_hist: BTreeMap<i32, usize> = BTreeMap::new();
        let mut tex_count = 0usize;
        let mut flat_count = 0usize;
        let mut sample_tex: Vec<(u32, u32, i32)> = Vec::new();
        for v in &verts {
            let tex = v.uv_tex & 0xffff;
            let sh = (v.abhsl & 0xffff) as i32;
            if tex != 0 {
                tex_count += 1;
                *tex_shade_hist.entry(sh).or_default() += 1;
                if sample_tex.len() < 6 {
                    sample_tex.push((tex, v.uv_tex >> 16, sh));
                }
            } else {
                flat_count += 1;
            }
        }
        both!(
            "input_shade={shade} verts={nverts} textured={tex_count} flat={flat_count} tex_shade_hist={tex_shade_hist:?} samples(tex,u,shade)={sample_tex:?}",
            nverts = verts.len(),
        );
    }

    both!("--- GPU render histograms ---");
    let Ok(mut backend) = GpuBackend::try_new() else {
        both!("GpuBackend::try_new FAILED (no adapter or SKIP_GPU=1) — GPU section skipped");
        both!("report_path={}", report_path.display());
        return;
    };
    both!("GpuBackend::try_new OK");

    let mut summary_rows = Vec::new();
    for &shade in &shades {
        let mut pix = textured_pix();
        // Mesh hist from a sibling build (same inputs); render uses its own mesh.
        let mut mesh_tex_shades: BTreeMap<i32, usize> = BTreeMap::new();
        {
            let mut pix_m = textured_pix();
            let mesh_m = build_mesh(shade, &mut pix_m);
            for v in mesh_m.vertices() {
                if v.uv_tex & 0xffff != 0 {
                    *mesh_tex_shades
                        .entry((v.abhsl & 0xffff) as i32)
                        .or_default() += 1;
                }
            }
        }
        let mesh = build_mesh(shade, &mut pix);
        let scene = backend.render_scene_for_test(mesh, &pix);
        let (w, h) = (512usize, 334usize);
        assert_eq!(scene.len(), w * h, "scene size");

        let mut red_dom_hist: BTreeMap<i32, usize> = BTreeMap::new();
        let mut all_r_hist: BTreeMap<i32, usize> = BTreeMap::new();
        let mut max_red = 0i32;
        let mut max_red_rgb = 0i32;
        let mut red_dom = 0usize;
        let mut nonzero = 0usize;
        for &rgb in &scene {
            let r = (rgb >> 16) & 0xff;
            let g = (rgb >> 8) & 0xff;
            let b = rgb & 0xff;
            if rgb != 0 {
                nonzero += 1;
            }
            *all_r_hist.entry(r).or_default() += 1;
            if r > g + 40 && r > b + 40 {
                red_dom += 1;
                *red_dom_hist.entry(r).or_default() += 1;
                if r > max_red {
                    max_red = r;
                    max_red_rgb = rgb;
                }
            }
        }

        let s = (shade as u32) & 0x7f;
        let block = (s >> 4) & 3;
        let block_factor = [1.0f32, 0.875, 0.75, 0.625][block as usize];
        let factor = block_factor * if (s >> 6) == 1 { 0.5 } else { 1.0 };
        let expected = (255.0 * factor).round() as i32;
        let delta = max_red - expected;
        let pass = (max_red - expected).abs() <= 8;

        // Top red-dom bins
        let mut top: Vec<(i32, usize)> = red_dom_hist.iter().map(|(&k, &v)| (k, v)).collect();
        top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        top.truncate(12);

        both!(
            "shade={shade:3} expected~{expected:3} max_red={max_red:3} delta={delta:+4} pass={pass} red_dom_px={red_dom} nonzero={nonzero} mesh_tex_shades={:?} top_red_dom_bins={:?} max_red_rgb=0x{max_red_rgb:06x}",
            mesh_tex_shades,
            top
        );
        summary_rows.push((shade, expected, max_red, delta, pass, red_dom, mesh_tex_shades, top));

        // Write a compact per-shade hist file
        let hist_path = out_dir.join(format!("shade-{shade:03}-red-dom-hist.txt"));
        if let Ok(mut f) = fs::File::create(&hist_path) {
            let _ = writeln!(f, "shade={shade} expected={expected} max_red={max_red}");
            for (r, c) in &red_dom_hist {
                let _ = writeln!(f, "{r} {c}");
            }
        }
    }

    both!("--- summary table ---");
    both!("shade expected max_red delta pass red_dom_px");
    for (shade, expected, max_red, delta, pass, red_dom, _, _) in &summary_rows {
        both!("{shade:5} {expected:8} {max_red:7} {delta:+5} {pass:5} {red_dom}");
    }

    // Relationship check: if mesh always carries exact input shade and
    // max_red stays at 255 for block>=1, truncation/factor path is implicated
    // at FS, not mesh packing.
    let mesh_ok = summary_rows
        .iter()
        .all(|(sh, _, _, _, _, _, hist, _)| hist.len() == 1 && hist.get(sh).is_some());
    let block1_fail = summary_rows
        .iter()
        .filter(|(sh, _, _, _, _, _, _, _)| (*sh >> 4) & 3 == 1 && *sh < 64)
        .any(|(_, _, max_red, _, pass, _, _, _)| !*pass && *max_red >= 247);
    both!("mesh_exact_input_shade_for_all_cases={mesh_ok}");
    both!("block1_under64_max_red_near_full_brightness_fail={block1_fail}");
    if mesh_ok && block1_fail {
        both!("INFERENCE_HINT=mesh packs correct shade; FS brightness still full → u32(in.hsl) block decode or factor path not taking effect on this adapter/run (or max_red selects non-representative pixels — see hist)");
    }

    both!("report_path={}", report_path.display());
    both!("out_dir={}", out_dir.display());
}

fn emit_adapter_info(report: &mut fs::File) {
    macro_rules! both {
        ($($t:tt)*) => {{
            let s = format!($($t)*);
            println!("{s}");
            let _ = writeln!(report, "{s}");
        }};
    }
    both!("--- wgpu adapters ---");
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()));
    both!("enumerate_count={}", adapters.len());
    for (i, a) in adapters.into_iter().enumerate() {
        let info = a.get_info();
        both!(
            "adapter[{i}] name={name:?} vendor=0x{vendor:x} device=0x{device:x} device_type={device_type:?} backend={backend:?} driver={driver:?} driver_info={driver_info:?}",
            name = info.name,
            vendor = info.vendor,
            device = info.device,
            device_type = info.device_type,
            backend = info.backend,
            driver = info.driver,
            driver_info = info.driver_info
        );
    }
    match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    })) {
        Ok(a) => {
            let info = a.get_info();
            both!(
                "selected_like_client name={name:?} backend={backend:?} driver={driver:?} driver_info={driver_info:?} device_type={device_type:?} vendor=0x{vendor:x} device=0x{device:x}",
                name = info.name,
                backend = info.backend,
                driver = info.driver,
                driver_info = info.driver_info,
                device_type = info.device_type,
                vendor = info.vendor,
                device = info.device
            );
        }
        Err(e) => both!("selected_like_client ERR {e}"),
    }
}

fn chrono_like_now() -> String {
    // Avoid chrono dep; UTC via date command not available guarantee — use system time.
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs} unix_s")
}

fn flat_world() -> World {
    let max_level: i32 = 1;
    let max_tile_x: i32 = 3;
    let max_tile_z: i32 = 3;
    let groundh = vec![
        vec![vec![2000i32; max_tile_z as usize + 1]; max_tile_x as usize + 1];
        max_level as usize
    ];
    let mut world = World::new(groundh, max_tile_z, max_level, max_tile_x);
    world.fill_base_level(0);
    for x in 0..max_tile_x {
        for z in 0..max_tile_z {
            world.set_ground(
                0,
                x,
                z,
                TerrainOverlayShape::PLAIN,
                0,
                -1,
                0,
                0,
                0,
                0,
                GROUND_SHADE,
                GROUND_SHADE,
                GROUND_SHADE,
                GROUND_SHADE,
                GROUND_SHADE,
                GROUND_SHADE,
                GROUND_SHADE,
                GROUND_SHADE,
                0,
                0,
            );
        }
    }
    world
}

fn textured_wall_model(shade: i32) -> client::dash3d::Model {
    let mut model = client::dash3d::Model {
        num_points: 4,
        point_x: Some(vec![-60, 60, 60, -60]),
        point_y: Some(vec![0, 0, -180, -180]),
        point_z: Some(vec![0, 0, 0, 0]),
        num_faces: 2,
        face_vertex_a: Some(vec![0, 0]),
        face_vertex_b: Some(vec![1, 2]),
        face_vertex_c: Some(vec![2, 3]),
        ..Default::default()
    };
    model.face_render_type = Some(vec![2, 2]);
    model.face_colour = Some(vec![TEXTURE_RED, TEXTURE_RED]);
    model.face_colour_a = Some(vec![shade, shade]);
    model.face_colour_b = Some(vec![shade, shade]);
    model.face_colour_c = Some(vec![shade, shade]);
    model.face_texture_p = Some(vec![0, 0]);
    model.face_texture_m = Some(vec![1, 1]);
    model.face_texture_n = Some(vec![2, 2]);
    model.calc_bounding_cylinder();
    model
}

fn solid_texture(rgb: i32) -> Pix8 {
    let mut tex = Pix8::new(64, 64, vec![0, rgb]);
    for p in tex.data.iter_mut() {
        *p = 1;
    }
    tex
}

fn textured_pix() -> Pix3DDraw {
    let mut pix = Pix3DDraw::default();
    pix.set_clipping(512, 334);
    pix.textures[TEXTURE_RED as usize] = Some(solid_texture(0xff0000));
    pix.tex_pal[TEXTURE_RED as usize] = Some(vec![0, 0xff0000]);
    pix
}

fn game_distance_table() -> [i32; 9] {
    let mut distance = [0i32; 9];
    for (x, slot) in distance.iter_mut().enumerate() {
        let angle = x as i32 * 32 + 128 + 15;
        let offset = angle * 3 + 600;
        let sin = Pix3D::sin_table()[angle as usize];
        *slot = (offset * sin) >> 16;
    }
    distance
}

fn build_mesh(shade: i32, pix: &mut Pix3DDraw) -> client::render::world::SceneMesh {
    let mut world = flat_world();
    world.set_wall(0, 1, 2, 2000, 8, 0, 0, 0, 0, 0, 0, 0);
    let mut rw = RenderWorld::new();
    rw.set_wall_model(
        &world,
        0,
        1,
        2,
        Some(SceneModel::Model(textured_wall_model(shade))),
        None,
    );
    rw.reset_vis_calc(&game_distance_table(), 500, 800, 512, 334);
    rw.prepare_scene(&mut world, &Cache::default(), 0, 192, 1950, 192, 3, 0, 128);
    rw.build_scene_mesh(&mut world, &Cache::default(), 0, pix)
}

// silence unused import if GpuVertex not used in some builds
#[allow(dead_code)]
fn _touch_vertex(v: &GpuVertex) -> u32 {
    v.abhsl
}
