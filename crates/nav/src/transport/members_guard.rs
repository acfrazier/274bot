//! WORLD membership (`map_members`) gates read from the content handler
//! each packed edge runs.
//!
//! Every loc, NPC and held-obj edge names the op it uses (`kind`, `loc_id`,
//! `option`). The engine runs the first of `[<op>,<name>]`,
//! `[<op>,_<category>]` and the global `[<op>,_]` that exists
//! (`ScriptProvider.getByTrigger`: type, category, global), and that
//! handler is the edge's source. Its *leading path* is its first statement,
//! followed through the label when that statement is an unconditional
//! `@label` jump, and so on. [`apply_members_guards`] sets `members_req` on
//! every edge whose leading path is the F2P refusal
//! `if (map_members = ^false …) { …; return; }`. A refusal anywhere else (a
//! `@multiN` option, a queue, a label inside a conditional, a later
//! statement) does not gate every use of the op, so the pass leaves it to
//! the check. Producers whose gate has another shape (a positive
//! `map_members = ^true` arm, a stage-door path, the spell table's
//! `members` column) set the flag themselves.
//!
//! [`require_members_guards`] is the bake's two-way check. A free edge
//! whose source (with every label, choice and queue it may continue into)
//! reads `map_members` fails, unless that exact read is listed in
//! [`NON_GATE_READS`]. A members edge whose leading path does not refuse
//! fails too, unless its source path carries a read listed in
//! [`MEMBERS_ARMS`].

use super::*;

/// `map_members` reads that do not gate the transport an edge takes:
/// `(op, handler, normalized statement, why)`. The statement is pinned, so
/// a content change to it fails the bake instead of silently passing.
const NON_GATE_READS: &[(&str, &str, &str, &str)] = &[
    (
        "opnpc1",
        "_sailor",
        "if(map_members=^true&npc_type=captain_tobias){",
        "Captain Tobias' clue-scroll reward; the Karamja sail dialogue follows on every world",
    ),
    (
        "oploc1",
        "whistledoor",
        "if(map_members=^true&inv_total(inv,holy_table_napkin)>0&inv_total(inv,magic_whistle)<2){",
        "spawns magic whistles; the door opens on every world",
    ),
    (
        "oploc1",
        "guidordoor",
        "if(map_members=^false|%biohazard<^biohazard_spoken_chemist){",
        "refuses entering only; leaving returns through the door before this check",
    ),
];

/// Members gates that are not their edge's leading refusal, set by the
/// producer that reads them: `(op, handler, normalized statement, why)`. A
/// members edge whose leading path does not refuse passes the check only
/// when one of these exact reads lies on its source path.
const MEMBERS_ARMS: &[(&str, &str, &str, &str)] = &[
    (
        "opnpc1",
        "gnomepilot",
        "if(%grandtree=^grandtree_complete&map_members=^true){",
        "only this members arm opens the glider menu (gliders.rs)",
    ),
    (
        "oploc1",
        "zanarisdoor",
        "if(inv_total(worn,dramen_staff)>0&map_members=^true){",
        "only this members arm teleports into Zanaris (zanaris.rs)",
    ),
    (
        "label",
        "spirit_tree_tele",
        "if(map_members=^false){",
        "every spirit-tree ride jumps here after its quest check and menu (spirit_trees.rs)",
    ),
];

/// Calls whose first argument names the `[queue,<name>]` block they run.
const QUEUE_CALLS: [&str; 4] = ["queue(", "longqueue(", "strongqueue(", "weakqueue("];

/// The `map_members` facts of one handler block.
#[derive(Debug, Default)]
struct Handler {
    /// The first statement refuses every F2P player and returns.
    refuses_f2p: bool,
    /// The first statement is the unconditional jump `@<label>…;`.
    lead_jump: Option<String>,
    /// Normalized statements that read `map_members`.
    reads: Vec<String>,
    /// Every `[label,…]` / `[queue,…]` block this body may continue into.
    jumps: Vec<(&'static str, String)>,
}

/// Every content handler, with the names and categories the engine resolves
/// an edge's op through.
pub(super) struct MembersGuards {
    handlers: HashMap<(String, String), Handler>,
    locs: HashMap<i32, String>,
    npcs: HashMap<i32, String>,
    objs: HashMap<i32, String>,
    categories: HashMap<(&'static str, String), String>,
}

impl MembersGuards {
    pub(super) fn from_content(content_root: &Path) -> Self {
        let mut handlers: HashMap<(String, String), Handler> = HashMap::new();
        visit_rs2(&content_root.join("scripts"), &mut |text| {
            for (op, name, body) in handler_blocks(text) {
                let facts = handler_facts(&body);
                let slot = handlers.entry((op, name)).or_default();
                slot.refuses_f2p |= facts.refuses_f2p;
                slot.lead_jump = slot.lead_jump.take().or(facts.lead_jump);
                slot.reads.extend(facts.reads);
                slot.jumps.extend(facts.jumps);
            }
        });
        let invert = |file: &str| -> HashMap<i32, String> {
            pack_ids_by_name(content_root, file)
                .into_iter()
                .map(|(name, id)| (id, name))
                .collect()
        };
        let mut categories = HashMap::new();
        for ext in ["loc", "npc", "obj"] {
            visit_configs(&content_root.join("scripts"), ext, &mut |text| {
                let mut cur: Option<&str> = None;
                for line in text.lines() {
                    if let Some(name) = config_header(line.trim()) {
                        cur = Some(name);
                    } else if let (Some(name), Some(cat)) =
                        (cur, line.trim().strip_prefix("category="))
                    {
                        categories
                            .entry((ext, name.to_string()))
                            .or_insert_with(|| cat.trim().to_string());
                    }
                }
            });
        }
        Self {
            handlers,
            locs: invert("loc.pack"),
            npcs: invert("npc.pack"),
            objs: invert("obj.pack"),
            categories,
        }
    }

    /// The handler keys the engine tries for `edge`'s op, as `(op, name)`,
    /// in `ScriptProvider.getByTrigger` order: the type (its pack name,
    /// then the `loc_N`/`npc_N`/`obj_N` spelling of the same id), the
    /// category `_<category>`, then the global `_`. `None` for a spell
    /// teleport (the spell table, not a handler) and the never-packed
    /// essence exit.
    fn candidates(&self, edge: &TransportEdge) -> Option<Vec<(String, String)>> {
        let (config, op, id_names, alias) = match edge.kind {
            TransportKind::Boat | TransportKind::Npc | TransportKind::Glider => (
                "npc",
                format!("opnpc{}", edge.option),
                &self.npcs,
                format!("npc_{}", edge.loc_id),
            ),
            TransportKind::Teleport if edge.loc_id == 0 => return None,
            TransportKind::Teleport => (
                "obj",
                format!("opheld{}", edge.option),
                &self.objs,
                format!("obj_{}", edge.loc_id),
            ),
            TransportKind::EssenceExit => return None,
            _ => (
                "loc",
                if edge.option == 0 {
                    "oplocu".to_string()
                } else {
                    format!("oploc{}", edge.option)
                },
                &self.locs,
                format!("loc_{}", edge.loc_id),
            ),
        };
        let names: Vec<&str> = id_names
            .get(&edge.loc_id)
            .map(String::as_str)
            .into_iter()
            .chain([alias.as_str()])
            .collect();
        let category = names
            .iter()
            .find_map(|name| self.categories.get(&(config, name.to_string())))
            .map(|cat| format!("_{cat}"));
        Some(
            names
                .into_iter()
                .map(str::to_string)
                .chain(category)
                .chain(["_".to_string()])
                .map(|name| (op.clone(), name))
                .collect(),
        )
    }

    /// The first of `candidates` that exists: the handler the engine runs.
    fn resolve(&self, candidates: &[(String, String)]) -> Option<(String, String)> {
        candidates
            .iter()
            .find(|key| self.handlers.contains_key(*key))
            .cloned()
    }

    /// The handler and every label/queue block it may continue into, on
    /// any branch: the completeness scan's reach.
    fn reached(&self, source: (String, String)) -> Vec<(&(String, String), &Handler)> {
        let mut seen = HashSet::new();
        let mut pending = vec![source];
        let mut out = Vec::new();
        while let Some(key) = pending.pop() {
            let Some((key, handler)) = self.handlers.get_key_value(&key) else {
                continue;
            };
            if !seen.insert(key) {
                continue;
            }
            pending.extend(
                handler
                    .jumps
                    .iter()
                    .map(|(op, name)| (op.to_string(), name.clone())),
            );
            out.push((key, handler));
        }
        out
    }

    /// Whether the leading path from `source` (its first statement, through
    /// a chain of unconditional leading `@label` jumps) is the F2P refusal.
    fn leading_refusal(&self, source: &(String, String)) -> bool {
        let mut seen = HashSet::new();
        let mut key = source.clone();
        while let Some(handler) = self.handlers.get(&key) {
            if handler.refuses_f2p {
                return true;
            }
            let Some(label) = &handler.lead_jump else {
                return false;
            };
            if !seen.insert(key) {
                return false;
            }
            key = ("label".to_string(), label.clone());
        }
        false
    }

    /// Whether the edge's source refuses every F2P player before it moves.
    fn refuses_f2p(&self, edge: &TransportEdge) -> bool {
        self.candidates(edge)
            .and_then(|candidates| self.resolve(&candidates))
            .is_some_and(|source| self.leading_refusal(&source))
    }
}

/// Set `members_req` on every edge (and held-obj teleport) whose source
/// handler's leading path is the F2P refusal.
pub(super) fn apply_members_guards(guards: &MembersGuards, graph: &mut TransportGraph) {
    for edge in graph.edges.iter_mut().chain(graph.teleports.iter_mut()) {
        if !edge.members_req && guards.refuses_f2p(edge) {
            edge.members_req = true;
        }
    }
}

/// Bake-time check of every packed edge's `members_req` against its
/// source. A free edge whose source handler (or any label/choice/queue it
/// may continue into) reads `map_members` fails, unless the read is a
/// pinned [`NON_GATE_READS`] entry. A members edge whose leading path does
/// not refuse F2P fails, unless a pinned [`MEMBERS_ARMS`] read lies on its
/// source path. Every members spell in `magic_spells.dbrow` must pack
/// `members_req`, and every loc, NPC and held-obj edge must resolve to a
/// handler.
pub(crate) fn require_members_guards(
    content_root: &Path,
    graph: &TransportGraph,
) -> Result<(), String> {
    let guards = MembersGuards::from_content(content_root);
    let mut errors = Vec::new();
    for edge in graph.edges.iter().chain(&graph.teleports) {
        let Some(candidates) = guards.candidates(edge) else {
            continue;
        };
        let Some(source) = guards.resolve(&candidates) else {
            let tried: Vec<String> = candidates
                .iter()
                .map(|(op, name)| format!("[{op},{name}]"))
                .collect();
            errors.push(format!(
                "{:?} {} op{} at {:?} has no source handler (tried {})",
                edge.kind,
                edge.loc_id,
                edge.option,
                edge.at,
                tried.join(", ")
            ));
            continue;
        };
        let reached = guards.reached(source.clone());
        if edge.members_req {
            let pinned_arm = reached.iter().any(|((op, name), handler)| {
                handler.reads.iter().any(|read| {
                    MEMBERS_ARMS
                        .iter()
                        .any(|(o, n, stmt, _)| o == op && n == name && stmt == read)
                })
            });
            if !pinned_arm && !guards.leading_refusal(&source) {
                errors.push(format!(
                    "{:?} {} op{} at {:?} packs members_req=true but the leading path of [{},{}] does not refuse F2P",
                    edge.kind, edge.loc_id, edge.option, edge.at, source.0, source.1
                ));
            }
            continue;
        }
        for ((op, name), handler) in reached {
            let gate = handler.refuses_f2p.then_some("an F2P refusal");
            let read = handler.reads.iter().find(|read| {
                !NON_GATE_READS
                    .iter()
                    .any(|(o, n, stmt, _)| o == op && n == name && stmt == read)
            });
            if let Some(what) = gate.or(read.map(String::as_str)) {
                errors.push(format!(
                    "{:?} {} op{} at {:?} packs members_req=false but [{},{}] (via [{},{}]) reads map_members: {what}",
                    edge.kind, edge.loc_id, edge.option, edge.at, op, name, source.0, source.1
                ));
            }
        }
    }
    for to in members_spell_landings(content_root) {
        if graph
            .teleports
            .iter()
            .any(|e| e.loc_id == 0 && e.to == to && !e.members_req)
        {
            errors.push(format!(
                "members spell teleport to {to:?} packs members_req=false"
            ));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} transport edge(s) ignore their source map_members guard:\n  {}",
            errors.len(),
            errors.join("\n  ")
        ))
    }
}

/// Landings of the `magic_spell_teleport_*` rows declaring
/// `data=members,true` (the column `check_spell_requirements` refuses on an
/// F2P world).
pub(super) fn members_spell_landings(content_root: &Path) -> Vec<WorldTile> {
    let Ok(text) = fs::read_to_string(content_root.join(MAGIC_SPELLS_DBROW)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut cur: Option<(bool, Option<WorldTile>)> = None;
    let mut flush = |cur: Option<(bool, Option<WorldTile>)>| {
        if let Some((true, Some(to))) = cur {
            out.push(to);
        }
    };
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(name) = dbrow_block(line) {
            flush(cur.take());
            cur = name
                .starts_with("magic_spell_teleport_")
                .then_some((false, None));
            continue;
        }
        let Some((members, to)) = &mut cur else {
            continue;
        };
        if let Some(rest) = line.strip_prefix("data=members,") {
            *members = rest.trim() == "true";
        } else if let Some(rest) = line.strip_prefix("data=tele_coord,") {
            *to = coord_literal(rest.trim()).map(|(level, x, z)| WorldTile { x, z, level });
        }
    }
    flush(cur);
    out
}

/// `[<op>,<name>]` blocks of a script text → `(op, name, body)`. Unlike
/// [`script_blocks`], a header may carry a same-line body
/// (`[opheld4,amulet_of_glory_4] @amulet_of_glory_interface(…);`) or a
/// parameter list (`[proc,name](int $x)`), and every header ends the
/// previous block.
fn handler_blocks(text: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let mut cur: Option<(String, String, String)> = None;
    for raw in text.lines() {
        let line = raw.trim();
        let header = line
            .strip_prefix('[')
            .and_then(|rest| rest.split_once(']'))
            .and_then(|(inner, rest)| Some((inner.split_once(',')?, rest)))
            .filter(|((op, name), _)| is_word(op) && is_word(name));
        if let Some(((op, name), rest)) = header {
            out.extend(cur.take());
            let inline = if rest.trim_start().starts_with('(') {
                ""
            } else {
                rest
            };
            cur = Some((op.to_string(), name.to_string(), format!("{inline}\n")));
        } else if let Some((_, _, body)) = &mut cur {
            body.push_str(line);
            body.push('\n');
        }
    }
    out.extend(cur);
    out
}

fn is_word(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

fn handler_facts(body: &str) -> Handler {
    let code = strip_block_comments(body);
    let statements = top_level_statements(&code);
    let first = statements.first().map(String::as_str);
    let refuses_f2p = first
        .and_then(if_head_and_arm)
        .is_some_and(|(head, arm)| refusal_head(&head) && refusal_arm(&arm));
    let lead_jump = first.and_then(label_jump);
    let reads = code
        .lines()
        .map(|raw| raw.split_once("//").map_or(raw, |(code, _)| code))
        .filter(|line| line.contains("map_members"))
        .map(|line| line.chars().filter(|c| !c.is_whitespace()).collect())
        .collect();
    let calls = strip_strings(&code);
    let mut jumps: Vec<(&'static str, String)> = body_labels(&calls)
        .into_iter()
        .map(|name| ("label", name))
        .collect();
    // `@multi2("…", label_a, "…", label_b)`: each bare label argument is the
    // choice's continuation.
    for (at, _) in calls.match_indices("@multi") {
        let Some(open) = calls[at..].find('(').map(|i| at + i + 1) else {
            continue;
        };
        let args = calls[open..].split(')').next().unwrap_or("");
        jumps.extend(
            args.split(',')
                .map(str::trim)
                .filter(|arg| is_word(arg) && arg.starts_with(|c: char| c.is_ascii_alphabetic()))
                .map(|arg| ("label", arg.to_string())),
        );
    }
    for call in QUEUE_CALLS {
        for (at, _) in calls.match_indices(call) {
            let preceded_by_word = calls[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
            let name: String = calls[at + call.len()..]
                .trim_start()
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !preceded_by_word && !name.is_empty() {
                jumps.push(("queue", name));
            }
        }
    }
    Handler {
        refuses_f2p,
        lead_jump,
        reads,
        jumps,
    }
}

/// `@name;` or `@name(…);` as a whole statement → `name`.
fn label_jump(stmt: &str) -> Option<String> {
    let rest = stmt.strip_prefix('@')?;
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    let (name, args) = rest.split_at(end);
    let args = args.trim_start();
    (!name.is_empty() && (args == ";" || (args.starts_with('(') && args.ends_with(");"))))
        .then(|| name.to_string())
}

/// `map_members = ^false`, alone or OR-joined with other terms: every F2P
/// player takes the arm.
fn refusal_head(head: &str) -> bool {
    let flat: String = head.chars().filter(|c| !c.is_whitespace()).collect();
    let mut depth = 0i32;
    let mut terms = vec![String::new()];
    for c in flat.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            '&' if depth == 0 => return false,
            '|' if depth == 0 => {
                terms.push(String::new());
                continue;
            }
            _ => {}
        }
        terms.last_mut().expect("one term").push(c);
    }
    terms.iter().any(|term| term == "map_members=^false")
}

/// The arm ends the handler: its last statement is `return;`.
fn refusal_arm(arm: &str) -> bool {
    normalized_body(arm).ends_with("return;}")
}

fn strip_block_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        rest = rest[start + 2..]
            .split_once("*/")
            .map_or("", |(_, after)| after);
    }
    out.push_str(rest);
    out
}

/// Code with `"…"` string literals blanked, so colour tags such as
/// `@blu@` are not read as label jumps.
fn strip_strings(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_string = false;
    let mut escaped = false;
    for c in text.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        if c == '"' {
            in_string = true;
        }
        out.push(c);
    }
    out
}
