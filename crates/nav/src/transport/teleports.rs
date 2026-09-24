use super::*;

/// Teleport edges (the any-tile layer): the seven spell teleports from
/// `skill_magic/configs/magic_spells.dbrow` plus the jewellery rub
/// teleports from `general/scripts/enchanted_jewellry/*.rs2`, all into
/// [`TransportGraph::teleports`] — never `edges`/`at`, so the default
/// [`crate::router::find`] never sees them.
pub(super) fn teleport_edges(
    content_root: &Path,
    graph: &mut TransportGraph,
    skipped: &mut HashMap<&'static str, usize>,
) {
    if let Ok(rules) = load_wilderness_rules(content_root) {
        graph.wilderness = rules;
    }
    let objs = obj_ids_by_name(content_root);
    let spell_cap = spell_teleport_cap(content_root).ok().flatten();
    spell_teleports(content_root, &objs, graph, skipped, spell_cap);
    jewellery_teleports(content_root, &objs, graph, skipped);
}

/// Spell teleports from `skill_magic/configs/magic_spells.dbrow`: each
/// `[magic_spell_teleport_*]` block declares `data=levelrequired,N`,
/// `data=runesrequired,<rune>,<count>[,<rune>,<count>]` (rune names
/// resolved through `pack/obj.pack`), and `data=tele_coord,<coord>`
/// (absolute). Requirement = the magic level (`skill_req`) plus the runes
/// (`item_req`); the members flag declares no gate this model carries.
/// Ticks = [`SPELL_TELEPORT_TICKS`].
pub(super) fn spell_teleports(
    content_root: &Path,
    objs: &HashMap<String, i32>,
    graph: &mut TransportGraph,
    skipped: &mut HashMap<&'static str, usize>,
    wildy_cap: Option<i32>,
) {
    let path = content_root
        .join("scripts")
        .join("skill_magic")
        .join("configs")
        .join("magic_spells.dbrow");
    let Ok(text) = fs::read_to_string(&path) else {
        return;
    };
    // (levelrequired, rune pairs, tele_coord) of the current teleport block.
    type SpellTeleportBlock = (Option<i32>, Vec<(String, i32)>, Option<String>);
    let mut cur: Option<SpellTeleportBlock> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(name) = dbrow_block(line) {
            if let Some(block) = cur.take() {
                push_spell_teleport(objs, graph, skipped, block, wildy_cap);
            }
            cur = name
                .starts_with("magic_spell_teleport_")
                .then(|| (None, Vec::new(), None));
            continue;
        }
        let Some((level, runes, coord)) = &mut cur else {
            continue;
        };
        if let Some(rest) = line.strip_prefix("data=levelrequired,") {
            *level = rest.trim().parse().ok();
        } else if let Some(rest) = line.strip_prefix("data=runesrequired,") {
            *runes = rune_pairs(rest);
        } else if let Some(rest) = line.strip_prefix("data=tele_coord,") {
            *coord = Some(rest.trim().to_string());
        }
    }
    if let Some(block) = cur.take() {
        push_spell_teleport(objs, graph, skipped, block, wildy_cap);
    }
}

/// `[<name>]` dbrow section header → the block name.
pub(super) fn dbrow_block(line: &str) -> Option<&str> {
    let name = line.strip_prefix('[')?.strip_suffix(']')?;
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return None;
    }
    Some(name)
}

/// `firerune,1,airrune,3,lawrune,1[,null,null]` → rune/count pairs; the
/// trailing `null,null` slot padding is dropped.
pub(super) fn rune_pairs(rest: &str) -> Vec<(String, i32)> {
    let toks: Vec<&str> = rest.split(',').map(|t| t.trim()).collect();
    let mut out = Vec::new();
    for pair in toks.chunks(2) {
        if pair.len() < 2 || pair[0] == "null" {
            continue;
        }
        if let Ok(count) = pair[1].parse::<i32>() {
            out.push((pair[0].to_string(), count));
        }
    }
    out
}

pub(super) fn push_spell_teleport(
    objs: &HashMap<String, i32>,
    graph: &mut TransportGraph,
    skipped: &mut HashMap<&'static str, usize>,
    (level, runes, coord): (Option<i32>, Vec<(String, i32)>, Option<String>),
    wildy_cap: Option<i32>,
) {
    let Some(level) = level else {
        bump(skipped, SKIP_TELEPORT_BAD_DEST, 1);
        return;
    };
    let Some(coord) = coord.and_then(|c| coord_literal(&c)) else {
        bump(skipped, SKIP_TELEPORT_BAD_DEST, 1);
        return;
    };
    let mut item_req = Vec::with_capacity(runes.len());
    for (rune, count) in &runes {
        let Some(&id) = objs.get(rune) else {
            bump(skipped, SKIP_TELEPORT_UNRESOLVED_RUNE, 1);
            return;
        };
        item_req.push((id, *count));
    }
    graph.teleports.push(TransportEdge {
        kind: TransportKind::Teleport,
        at: TELEPORT_PLACEHOLDER_AT,
        to: WorldTile {
            x: coord.1,
            z: coord.2,
            level: coord.0,
        },
        loc_id: 0, // a spell button, not a loc/obj use
        option: 0,
        ticks: SPELL_TELEPORT_TICKS,
        dir: None,
        open_loc_id: None,
        skill_req: vec![(SKILL_MAGIC, level)],
        item_req,
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap,
    });
}

/// Jewellery rub teleports from `general/scripts/enchanted_jewellry/*.rs2`:
/// `[opheld4,<name>]` blocks whose body — directly or through a forwarded
/// `@label` (the glory rubs share `@amulet_of_glory_interface`) — calls
/// `~player_teleport_normal(<coord>|map_findsquare(<coord>, …))`. The block
/// name is the charged obj's name, or a `_`-prefixed category the obj
/// config `skill_magic/configs/enchanted_jewelry.obj` resolves
/// (`category=…`); obj ids come from `pack/obj.pack`. Requirement = holding
/// the charged item (`item_req`); `option` 4 is the Rub op (`opheld4`).
/// Ticks = [`JEWELLERY_TELEPORT_TICKS`].
pub(super) fn jewellery_teleports(
    content_root: &Path,
    objs: &HashMap<String, i32>,
    graph: &mut TransportGraph,
    skipped: &mut HashMap<&'static str, usize>,
) {
    let cats = jewellery_categories(content_root);
    let dir = content_root
        .join("scripts")
        .join("general")
        .join("scripts")
        .join("enchanted_jewellry");
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    for ent in entries.flatten() {
        let path = ent.path();
        if path.extension().and_then(|s| s.to_str()) != Some("rs2") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let wildy_cap = jewellery_file_cap(&text).ok().flatten();
        for (op, name, body) in jewellery_blocks(&text) {
            if op != "opheld4" {
                continue;
            }
            let dests = block_teleport_dests(&body, &text);
            if dests.is_empty() {
                continue;
            }
            let items: Vec<String> = match name.strip_prefix('_') {
                Some(cat) => cats.get(cat).cloned().unwrap_or_default(),
                None => vec![name.clone()],
            };
            for item in items {
                let Some(&obj_id) = objs.get(&item) else {
                    bump(skipped, SKIP_TELEPORT_UNRESOLVED_ITEM, 1);
                    continue;
                };
                for dest in &dests {
                    graph.teleports.push(TransportEdge {
                        kind: TransportKind::Teleport,
                        at: TELEPORT_PLACEHOLDER_AT,
                        to: *dest,
                        loc_id: obj_id,
                        option: 4, // Rub (opheld4)
                        ticks: JEWELLERY_TELEPORT_TICKS,
                        dir: None,
                        open_loc_id: None,
                        skill_req: vec![],
                        item_req: vec![(obj_id, 1)],
                        quest_req: vec![],
                        varp_req: vec![],
                        worn_req: vec![],
                        members_req: false,
                        wildy_cap,
                    });
                }
            }
        }
    }
}

/// `category=<cat>` → obj block names, from `skill_magic/configs/
/// enchanted_jewelry.obj` (the `_`-prefixed `opheld4` blocks dispatch on
/// these categories).
pub(super) fn jewellery_categories(content_root: &Path) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    let path = content_root
        .join("scripts")
        .join("skill_magic")
        .join("configs")
        .join("enchanted_jewelry.obj");
    let Ok(text) = fs::read_to_string(&path) else {
        return out;
    };
    let mut cur: Option<&str> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            cur = line.strip_prefix('[').and_then(|s| s.strip_suffix(']'));
            continue;
        }
        if let Some(cat) = line.strip_prefix("category=") {
            if let Some(name) = cur {
                out.entry(cat.trim().to_string())
                    .or_default()
                    .push(name.to_string());
            }
        }
    }
    out
}

/// `(op, name, body)` blocks in an enchanted_jewellry file. Headers here
/// are `[op,<name>]` possibly with body text on the same line (the glory
/// `opheld4` one-liners) and/or a `(params)` list
/// (`[label,<name>](string $m)`); the strict [`script_header`] rejects
/// both, so these files need their own lenient parse.
pub(super) fn jewellery_blocks(text: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let mut cur: Option<(String, String)> = None;
    let mut body = String::new();
    for raw in text.lines() {
        let line = match raw.find("//") {
            Some(i) => raw[..i].trim(),
            None => raw.trim(),
        };
        if line.is_empty() {
            continue;
        }
        if let Some((header, rest)) = jewellery_header(line) {
            if let Some(prev) = cur.take() {
                out.push((prev.0, prev.1, std::mem::take(&mut body)));
            }
            cur = Some((header.0.to_string(), header.1.to_string()));
            if let Some(rest) = rest {
                body.push_str(rest);
                body.push('\n');
            }
        } else if cur.is_some() {
            body.push_str(line);
            body.push('\n');
        }
    }
    if let Some(prev) = cur.take() {
        out.push((prev.0, prev.1, body));
    }
    out
}

/// `[op,<name>](params)` block header → `((op, name), same-line body)`.
/// A header line carries a trailing body only when there is no `(params)`
/// list after the `]`.
pub(super) fn jewellery_header(line: &str) -> Option<((&str, &str), Option<&str>)> {
    let rest = line.strip_prefix('[')?;
    let close = rest.find(']')?;
    let head = &rest[..close];
    let tail = rest[close + 1..].trim();
    let (a, b) = head.split_once(',')?;
    let word = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_');
    if !word(a) || !word(b) {
        return None;
    }
    if tail.is_empty() || (tail.starts_with('(') && tail.ends_with(')')) {
        Some(((a, b), None))
    } else {
        Some(((a, b), Some(tail)))
    }
}

/// The body of `[label,<name>](…)` in the raw script text, from the header
/// line to the next block header line (params tolerated; the strict
/// [`script_header`] skips these headers).
pub(super) fn label_body_raw(text: &str, name: &str) -> Option<String> {
    let needle = format!("[label,{name}]");
    let mut in_label = false;
    let mut out = String::new();
    for raw in text.lines() {
        let line = match raw.find("//") {
            Some(i) => raw[..i].trim(),
            None => raw.trim(),
        };
        if line.is_empty() {
            continue;
        }
        if in_label {
            if jewellery_header(line).is_some() {
                break;
            }
            out.push_str(line);
            out.push('\n');
        } else if line.starts_with(&needle) {
            in_label = true;
        }
    }
    in_label.then_some(out)
}

/// The destinations an `opheld4` block can take the player to: direct
/// `~player_teleport_normal(...)` calls in the block, plus any such calls
/// in the `@label` bodies the block forwards to.
pub(super) fn block_teleport_dests(body: &str, script_text: &str) -> Vec<WorldTile> {
    let mut out = Vec::new();
    for args in call_args_all(body, "~player_teleport_normal") {
        if let Some(dest) = args.first().and_then(|a| teleport_dest(a)) {
            out.push(dest);
        }
    }
    for label in body_labels(body) {
        if let Some(lb) = label_body_raw(script_text, &label) {
            out.extend(block_teleport_dests(&lb, script_text));
        }
    }
    out
}

/// The landing of one `~player_teleport_normal(...)` arg: a 5-part coord
/// literal, or `map_findsquare(<coord>, …)` (the search square is the
/// literal's own tile).
pub(super) fn teleport_dest(arg: &str) -> Option<WorldTile> {
    let arg = arg.trim();
    let coord = if let Some(inner) = arg.strip_prefix("map_findsquare(") {
        inner.split(',').next()?.trim()
    } else {
        arg
    };
    coord_literal(coord).map(|(level, x, z)| WorldTile { x, z, level })
}
