//! Generated Host JS types (`host-js/index.d.ts`) from our verb tables.
//! Snapshot fields, [`crate::shim::InteractReq`] ops, and nav
//! [`crate::FindOptions`] — not rs2b0t names. NativeTick Load is 0.2.5.

use std::path::{Path, PathBuf};

struct TsField {
    name: &'static str,
    ty: &'static str,
    optional: bool,
    doc: Option<&'static str>,
}

struct TsInterface {
    name: &'static str,
    doc: Option<&'static str>,
    fields: &'static [TsField],
}

struct InteractVariant {
    op: &'static str,
    fields: &'static [TsField],
}

/// `crates/script/host-js/index.d.ts`
pub fn host_js_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("host-js/index.d.ts")
}

pub fn write_host_js_dts() -> Result<(), String> {
    let path = host_js_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let src = render_host_js_dts();
    std::fs::write(&path, src).map_err(|e| format!("write {}: {e}", path.display()))
}

pub fn write_host_js_dts_to(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let src = render_host_js_dts();
    std::fs::write(path, src).map_err(|e| format!("write {}: {e}", path.display()))
}

/// Render the living Host JS declaration file from the host verb tables.
pub fn render_host_js_dts() -> String {
    let mut out = String::from(
        "// Generated from host verb tables — do not edit by hand.\n\
         // Regen: cargo test -p script --test host_js regen_host_js -- --ignored\n\
         // NativeTick Load is 0.2.5. Not a clone of rs2b0t-api.\n\n",
    );

    for iface in SUPPORTING_INTERFACES {
        render_interface(&mut out, iface);
        out.push('\n');
    }

    render_find_options(&mut out);
    out.push('\n');

    render_interface(
        &mut out,
        &TsInterface {
            name: "Camera",
            doc: Some(
                "Orbit camera read from the posted snapshot (`camera_yaw` / `camera_pitch`).",
            ),
            fields: &[
                TsField {
                    name: "yaw",
                    ty: "number",
                    optional: false,
                    doc: Some("Follow-camera yaw."),
                },
                TsField {
                    name: "pitch",
                    ty: "number",
                    optional: false,
                    doc: Some("Follow-camera pitch."),
                },
                TsField {
                    name: "orbit_yaw",
                    ty: "number",
                    optional: false,
                    doc: Some("Orbit target yaw (`CameraView::orbit_yaw`)."),
                },
            ],
        },
    );
    out.push('\n');

    render_interface(
        &mut out,
        &TsInterface {
            name: "ShopStockRow",
            doc: Some("One shop stock row when `shop_open` is true."),
            fields: &[
                TsField {
                    name: "name",
                    ty: "string",
                    optional: false,
                    doc: None,
                },
                TsField {
                    name: "count",
                    ty: "number",
                    optional: false,
                    doc: None,
                },
            ],
        },
    );
    out.push('\n');

    render_interface(
        &mut out,
        &TsInterface {
            name: "Snapshot",
            doc: Some(
                "The PLAYER_INFO snapshot posted into an isolate. Delta posts omit unchanged fields.",
            ),
            fields: SNAPSHOT_FIELDS,
        },
    );
    out.push('\n');

    render_interface(
        &mut out,
        &TsInterface {
            name: "HostHandle",
            doc: Some("The per-tick host handle (`__rs2b0t_host`) Compat scripts queue onto."),
            fields: &[
                TsField {
                    name: "tick",
                    ty: "number",
                    optional: false,
                    doc: None,
                },
                TsField {
                    name: "snapshot",
                    ty: "Snapshot",
                    optional: false,
                    doc: None,
                },
                TsField {
                    name: "interact",
                    ty: "InteractReq[]",
                    optional: false,
                    doc: Some("Interact queue drained by the host after each tick."),
                },
                TsField {
                    name: "hold",
                    ty: "boolean",
                    optional: false,
                    doc: Some("Guardian hold gate (read-only)."),
                },
                TsField {
                    name: "ours",
                    ty: "boolean",
                    optional: false,
                    doc: Some("Guardian claim (read-only)."),
                },
            ],
        },
    );
    out.push('\n');

    render_interact_union(&mut out);

    out
}

fn render_find_options(out: &mut String) {
    out.push_str("/**\n");
    out.push_str(" * Walk/nav opt-ins for packed nav (`Traveller` / `ScriptWalkArm`).\n");
    out.push_str(" * All default off.\n");
    out.push_str(" */\n");
    out.push_str("export interface FindOptions {\n");
    out.push_str("  /** Allow packed-nav teleports (default off). */\n");
    out.push_str("  allow_teleports?: boolean;\n");
    out.push_str("  /**\n");
    out.push_str("   * Allow routes that enter or land in the wilderness zone.\n");
    out.push_str("   * Default off — nav refuses wilderness tiles without this opt-in.\n");
    out.push_str("   */\n");
    out.push_str("  allow_wilderness?: boolean;\n");
    out.push_str("  /** Latch a host BankBudget session when true. */\n");
    out.push_str("  allow_bank_fetch?: boolean;\n");
    out.push_str("}\n");
}

fn render_interface(out: &mut String, iface: &TsInterface) {
    if let Some(doc) = iface.doc {
        out.push_str("/** ");
        out.push_str(doc);
        out.push_str(" */\n");
    }
    out.push_str("export interface ");
    out.push_str(iface.name);
    out.push_str(" {\n");
    for field in iface.fields {
        if let Some(doc) = field.doc {
            out.push_str("  /** ");
            out.push_str(doc);
            out.push_str(" */\n");
        }
        out.push_str("  ");
        out.push_str(field.name);
        if field.optional {
            out.push('?');
        }
        out.push_str(": ");
        out.push_str(field.ty);
        out.push_str(";\n");
    }
    out.push_str("}\n");
}

fn render_interact_union(out: &mut String) {
    out.push_str(
        "/** One interact queued on the host handle; dispatched through the slot Driver. */\n",
    );
    out.push_str("export type InteractReq =\n");
    for (i, variant) in INTERACT_VARIANTS.iter().enumerate() {
        out.push_str("  | { op: '");
        out.push_str(variant.op);
        out.push('\'');
        for field in variant.fields {
            out.push_str("; ");
            out.push_str(field.name);
            if field.optional {
                out.push('?');
            }
            out.push_str(": ");
            out.push_str(field.ty);
        }
        out.push('}');
        if i + 1 < INTERACT_VARIANTS.len() {
            out.push('\n');
        }
    }
    out.push_str(";\n");
}

const SUPPORTING_INTERFACES: &[TsInterface] = &[
    TsInterface {
        name: "WorldTile",
        doc: Some("Absolute world tile `{x, z, level}`."),
        fields: &[
            TsField {
                name: "x",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "z",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "level",
                ty: "number",
                optional: false,
                doc: None,
            },
        ],
    },
    TsInterface {
        name: "ItemRow",
        doc: Some("One inventory/bank/equipment/trade row from the posted snapshot."),
        fields: &[
            TsField {
                name: "name",
                ty: "string | null",
                optional: false,
                doc: None,
            },
            TsField {
                name: "count",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "id",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "ops",
                ty: "string[]",
                optional: false,
                doc: None,
            },
            TsField {
                name: "noted",
                ty: "boolean",
                optional: false,
                doc: None,
            },
            TsField {
                name: "cert",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "component_id",
                ty: "number",
                optional: false,
                doc: None,
            },
        ],
    },
    TsInterface {
        name: "StatRow",
        doc: None,
        fields: &[
            TsField {
                name: "index",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "name",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "xp",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "level",
                ty: "number",
                optional: false,
                doc: None,
            },
        ],
    },
    TsInterface {
        name: "SceneEntity",
        doc: Some("One npc/loc/player/ground row from the posted snapshot."),
        fields: &[
            TsField {
                name: "index",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "id",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "name",
                ty: "string | null",
                optional: false,
                doc: None,
            },
            TsField {
                name: "x",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "z",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "level",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "distance",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "health",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "max_health",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "in_combat",
                ty: "boolean",
                optional: false,
                doc: None,
            },
            TsField {
                name: "animating",
                ty: "boolean",
                optional: false,
                doc: None,
            },
            TsField {
                name: "actions",
                ty: "string[]",
                optional: false,
                doc: None,
            },
            TsField {
                name: "reachable",
                ty: "boolean",
                optional: false,
                doc: None,
            },
            TsField {
                name: "reachable_adj",
                ty: "boolean",
                optional: false,
                doc: None,
            },
            TsField {
                name: "combat_level",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "target_kind",
                ty: "number",
                optional: false,
                doc: Some("0 none, 1 npc, 2 player."),
            },
            TsField {
                name: "target_index",
                ty: "number",
                optional: false,
                doc: Some("-1 when not facing anyone."),
            },
        ],
    },
    TsInterface {
        name: "BankStand",
        doc: Some("A packed bank stand (booth loc or teller npc)."),
        fields: &[
            TsField {
                name: "name",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "x",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "z",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "level",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "kind",
                ty: "'booth' | 'npc'",
                optional: false,
                doc: None,
            },
            TsField {
                name: "op",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "choose",
                ty: "string | null",
                optional: false,
                doc: None,
            },
        ],
    },
    TsInterface {
        name: "NearestBooth",
        doc: None,
        fields: &[
            TsField {
                name: "x",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "z",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "level",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "name",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "op",
                ty: "string",
                optional: false,
                doc: None,
            },
        ],
    },
    TsInterface {
        name: "ChatOption",
        doc: None,
        fields: &[TsField {
            name: "text",
            ty: "string",
            optional: false,
            doc: None,
        }],
    },
    TsInterface {
        name: "VarpRow",
        doc: None,
        fields: &[
            TsField {
                name: "index",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "value",
                ty: "number",
                optional: false,
                doc: None,
            },
        ],
    },
    TsInterface {
        name: "CombatStyleButton",
        doc: None,
        fields: &[
            TsField {
                name: "mode",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "label",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "component_id",
                ty: "number",
                optional: false,
                doc: None,
            },
        ],
    },
    TsInterface {
        name: "SideTabIface",
        doc: None,
        fields: &[
            TsField {
                name: "index",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "id",
                ty: "number",
                optional: false,
                doc: None,
            },
        ],
    },
    TsInterface {
        name: "ChatLine",
        doc: None,
        fields: &[
            TsField {
                name: "seq",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "text",
                ty: "string",
                optional: false,
                doc: None,
            },
        ],
    },
    TsInterface {
        name: "MakeButton",
        doc: None,
        fields: &[
            TsField {
                name: "qty",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "com_id",
                ty: "number",
                optional: false,
                doc: None,
            },
        ],
    },
    TsInterface {
        name: "MakeProduct",
        doc: None,
        fields: &[
            TsField {
                name: "object_id",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "name",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "buttons",
                ty: "MakeButton[]",
                optional: false,
                doc: None,
            },
        ],
    },
];

const SNAPSHOT_FIELDS: &[TsField] = &[
    TsField {
        name: "tick",
        ty: "number",
        optional: false,
        doc: Some("Always carried; other fields are delta-posted."),
    },
    TsField {
        name: "here",
        ty: "WorldTile | null",
        optional: false,
        doc: None,
    },
    TsField {
        name: "ingame",
        ty: "boolean",
        optional: false,
        doc: None,
    },
    TsField {
        name: "inv",
        ty: "ItemRow[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "inv_size",
        ty: "number",
        optional: false,
        doc: None,
    },
    TsField {
        name: "stats",
        ty: "StatRow[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "booths",
        ty: "WorldTile[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "nearest_booth",
        ty: "NearestBooth | null",
        optional: false,
        doc: None,
    },
    TsField {
        name: "banks",
        ty: "BankStand[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "bank",
        ty: "ItemRow[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "bank_side",
        ty: "ItemRow[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "bank_open",
        ty: "boolean",
        optional: false,
        doc: None,
    },
    TsField {
        name: "bank_loaded",
        ty: "boolean",
        optional: false,
        doc: None,
    },
    TsField {
        name: "bank_note_on",
        ty: "number",
        optional: false,
        doc: None,
    },
    TsField {
        name: "bank_note_off",
        ty: "number",
        optional: false,
        doc: None,
    },
    TsField {
        name: "scene_state",
        ty: "number",
        optional: false,
        doc: Some("2 = 3D ready."),
    },
    TsField {
        name: "weight",
        ty: "number",
        optional: false,
        doc: None,
    },
    TsField {
        name: "camera_yaw",
        ty: "number",
        optional: false,
        doc: Some("Orbit camera yaw."),
    },
    TsField {
        name: "camera_pitch",
        ty: "number",
        optional: false,
        doc: Some("Orbit camera pitch."),
    },
    TsField {
        name: "teleports_enabled",
        ty: "boolean",
        optional: false,
        doc: Some("Whether packed nav last armed with `allow_teleports`."),
    },
    TsField {
        name: "self_slot",
        ty: "number",
        optional: false,
        doc: None,
    },
    TsField {
        name: "trade_offer_open",
        ty: "boolean",
        optional: false,
        doc: None,
    },
    TsField {
        name: "trade_confirm_open",
        ty: "boolean",
        optional: false,
        doc: None,
    },
    TsField {
        name: "trade_partner",
        ty: "string | null",
        optional: false,
        doc: None,
    },
    TsField {
        name: "trade_mine",
        ty: "ItemRow[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "trade_theirs",
        ty: "ItemRow[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "trade_side",
        ty: "ItemRow[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "trade_accept_id",
        ty: "number",
        optional: false,
        doc: None,
    },
    TsField {
        name: "trade_decline_id",
        ty: "number",
        optional: false,
        doc: None,
    },
    TsField {
        name: "shop_open",
        ty: "boolean",
        optional: false,
        doc: None,
    },
    TsField {
        name: "shop_stock",
        ty: "ShopStockRow[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "hold",
        ty: "boolean",
        optional: false,
        doc: None,
    },
    TsField {
        name: "ours",
        ty: "boolean",
        optional: false,
        doc: None,
    },
    TsField {
        name: "npcs",
        ty: "SceneEntity[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "locs",
        ty: "SceneEntity[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "players",
        ty: "SceneEntity[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "ground",
        ty: "SceneEntity[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "equipment",
        ty: "ItemRow[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "chat_open",
        ty: "boolean",
        optional: false,
        doc: None,
    },
    TsField {
        name: "chat_continue",
        ty: "boolean",
        optional: false,
        doc: None,
    },
    TsField {
        name: "chat_text",
        ty: "string | null",
        optional: false,
        doc: None,
    },
    TsField {
        name: "chat_options",
        ty: "ChatOption[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "side_tab",
        ty: "number",
        optional: false,
        doc: None,
    },
    TsField {
        name: "varps",
        ty: "VarpRow[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "combat_styles",
        ty: "CombatStyleButton[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "run_energy",
        ty: "number",
        optional: false,
        doc: None,
    },
    TsField {
        name: "run_enabled",
        ty: "boolean",
        optional: false,
        doc: None,
    },
    TsField {
        name: "retaliate_enabled",
        ty: "boolean",
        optional: false,
        doc: None,
    },
    TsField {
        name: "my_name",
        ty: "string | null",
        optional: false,
        doc: None,
    },
    TsField {
        name: "in_combat",
        ty: "boolean",
        optional: false,
        doc: None,
    },
    TsField {
        name: "animating",
        ty: "boolean",
        optional: false,
        doc: None,
    },
    TsField {
        name: "main_modal_id",
        ty: "number",
        optional: false,
        doc: None,
    },
    TsField {
        name: "chat_modal_id",
        ty: "number",
        optional: false,
        doc: None,
    },
    TsField {
        name: "make_products",
        ty: "MakeProduct[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "side_tab_ifaces",
        ty: "SideTabIface[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "spell_buttons",
        ty: "CombatStyleButton[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "chat_lines",
        ty: "ChatLine[]",
        optional: false,
        doc: None,
    },
];

const INTERACT_VARIANTS: &[InteractVariant] = &[
    InteractVariant {
        op: "open-booth",
        fields: &[
            TsField {
                name: "x",
                ty: "number",
                optional: true,
                doc: None,
            },
            TsField {
                name: "z",
                ty: "number",
                optional: true,
                doc: None,
            },
            TsField {
                name: "level",
                ty: "number",
                optional: true,
                doc: None,
            },
        ],
    },
    InteractVariant {
        op: "open-stand",
        fields: &[
            TsField {
                name: "x",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "z",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "level",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "kind",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "name",
                ty: "string | null",
                optional: true,
                doc: None,
            },
            TsField {
                name: "stand_op",
                ty: "number | null",
                optional: true,
                doc: None,
            },
            TsField {
                name: "choose",
                ty: "string | null",
                optional: true,
                doc: None,
            },
        ],
    },
    InteractVariant {
        op: "walk",
        fields: &[
            TsField {
                name: "x",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "z",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "level",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "allow_teleports",
                ty: "boolean",
                optional: true,
                doc: None,
            },
        ],
    },
    InteractVariant {
        op: "walk-to",
        fields: &[
            TsField {
                name: "x",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "z",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "level",
                ty: "number",
                optional: false,
                doc: None,
            },
        ],
    },
    InteractVariant {
        op: "deposit",
        fields: &[TsField {
            name: "name",
            ty: "string",
            optional: false,
            doc: None,
        }],
    },
    InteractVariant {
        op: "withdraw",
        fields: &[
            TsField {
                name: "name",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "action",
                ty: "string",
                optional: false,
                doc: None,
            },
        ],
    },
    InteractVariant {
        op: "held",
        fields: &[
            TsField {
                name: "name",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "action",
                ty: "string",
                optional: false,
                doc: None,
            },
        ],
    },
    InteractVariant {
        op: "close",
        fields: &[],
    },
    InteractVariant {
        op: "npc",
        fields: &[
            TsField {
                name: "name",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "action",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "index",
                ty: "number | null",
                optional: true,
                doc: None,
            },
        ],
    },
    InteractVariant {
        op: "loc",
        fields: &[
            TsField {
                name: "x",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "z",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "level",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "action",
                ty: "string",
                optional: false,
                doc: None,
            },
        ],
    },
    InteractVariant {
        op: "obj",
        fields: &[
            TsField {
                name: "x",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "z",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "level",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "name",
                ty: "string | null",
                optional: true,
                doc: None,
            },
            TsField {
                name: "action",
                ty: "string",
                optional: false,
                doc: None,
            },
        ],
    },
    InteractVariant {
        op: "player",
        fields: &[
            TsField {
                name: "name",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "action",
                ty: "string",
                optional: false,
                doc: None,
            },
        ],
    },
    InteractVariant {
        op: "use-on",
        fields: &[
            TsField {
                name: "name",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "kind",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "target_name",
                ty: "string | null",
                optional: true,
                doc: None,
            },
            TsField {
                name: "x",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "z",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "level",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "index",
                ty: "number | null",
                optional: true,
                doc: None,
            },
        ],
    },
    InteractVariant {
        op: "use-widget-on",
        fields: &[
            TsField {
                name: "component_id",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "kind",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "target_name",
                ty: "string | null",
                optional: true,
                doc: None,
            },
            TsField {
                name: "x",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "z",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "level",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "index",
                ty: "number | null",
                optional: true,
                doc: None,
            },
        ],
    },
    InteractVariant {
        op: "continue",
        fields: &[],
    },
    InteractVariant {
        op: "answer",
        fields: &[TsField {
            name: "option",
            ty: "number",
            optional: false,
            doc: None,
        }],
    },
    InteractVariant {
        op: "answer-count",
        fields: &[TsField {
            name: "value",
            ty: "number",
            optional: false,
            doc: None,
        }],
    },
    InteractVariant {
        op: "if-button",
        fields: &[TsField {
            name: "component_id",
            ty: "number",
            optional: false,
            doc: None,
        }],
    },
    InteractVariant {
        op: "close-modal",
        fields: &[],
    },
    InteractVariant {
        op: "side-tab",
        fields: &[TsField {
            name: "tab",
            ty: "number",
            optional: false,
            doc: None,
        }],
    },
    InteractVariant {
        op: "wear",
        fields: &[TsField {
            name: "name",
            ty: "string",
            optional: false,
            doc: None,
        }],
    },
    InteractVariant {
        op: "set-run",
        fields: &[TsField {
            name: "on",
            ty: "boolean",
            optional: false,
            doc: None,
        }],
    },
    InteractVariant {
        op: "set-retaliate",
        fields: &[TsField {
            name: "on",
            ty: "boolean",
            optional: false,
            doc: None,
        }],
    },
    InteractVariant {
        op: "set-note-mode",
        fields: &[TsField {
            name: "on",
            ty: "boolean",
            optional: false,
            doc: None,
        }],
    },
    InteractVariant {
        op: "set-camera-yaw",
        fields: &[TsField {
            name: "yaw",
            ty: "number",
            optional: false,
            doc: None,
        }],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_includes_find_options_wilderness_jsdoc() {
        let src = render_host_js_dts();
        assert!(src.contains("allow_wilderness"));
        assert!(src.contains("wilderness zone"));
    }

    #[test]
    fn render_excludes_game_teleport() {
        let src = render_host_js_dts();
        assert!(!src.contains("Game.teleport"));
        assert!(!src.contains("teleport("));
    }
}
