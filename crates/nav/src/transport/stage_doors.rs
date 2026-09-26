use super::rs2_syntax::{parse_body, CmpOp, Expr, Stmt};
use super::*;

// ---------------------------------------------------------------------------
// Quest-stage and guild doors, and guarded ladders: named openers evaluated
// per crossing.
// ---------------------------------------------------------------------------
//
// A loc no other producer covered, whose effective `[oploc1]` opener (the
// loc's own named block, else its category's `[oploc1,_<category>]` block
// — the engine's resolution order) moves the player, gets an edge per
// crossing the opener provably makes:
//
// - a straight wall (shape 0) crosses in a direction when the opener, run
//   with the player on that direction's stand tile, reaches one of the
//   engine's open-and-close procs with the matching `$entering` side;
// - a centre-shaped loc (a ladder, shapes 10/11) climbs when the opener,
//   run without a stand (it is climbed from any side), reaches
//   `~climb_ladder` with a landing that does not depend on that side.
//
// Everything the path consults is either computed from the placement (the
// stand tile, `loc_coord`, `loc_angle`, `~check_axis`) or read as a
// requirement the bot can observe:
//
// - `%varp <op> ^const` / `stat(<skill>) <op> n` / `inv_total(inv|worn,
//   <obj>) <op> n` / `map_members <op> ^bool` are evaluated with the
//   player assumed past every threshold (quests complete, levels and
//   items high, a members world), and each comparison the taken path
//   reads records the minimum that keeps its outcome: `>=`/`<` c → c;
//   `>`/`<=`/`=`/`!` c → c + 1. The emitted edge carries those minimums
//   (`varp_req`/`skill_req`/`item_req`/`worn_req`/`members_req`), so any
//   state that meets them takes the same path. Varp minimums then go
//   through [`ObservableGates::admit_edge`]: a transmitted varp stays a
//   raw gate, an untransmitted one becomes its unique completed quest
//   journal row, and a varp with neither proof drops the edge.
// - An `if` whose branches only print (or drop items) is skipped without
//   reading its condition; of two operands that both decide an `|`/`&`,
//   the one with the cheaper requirement is kept.
// - Anything else on the path — a dialog, an NPC lookup, a key taken,
//   a varp written, a queue, a random roll, a `switch`/`while`, an
//   unknown proc — stops the evaluation and no crossing is emitted.
// - After the terminal call the rest of the path may print or talk but
//   must not move the player again or jump anywhere.
//
// The open procs are the engine's `~open_and_close_door`, `_door2`,
// `_door_fast`, `_double_door`, `_double_door2`
// (`scripts/doors/scripts/open_and_close_doors.rs2`,
// `open_and_close_double_doors.rs2`): each `p_teleport`s a player whose
// `$entering` is true onto `loc_coord + ~door_open(loc_angle)` and one
// whose `$entering` is false onto `loc_coord`, so the crossing is real
// only when `$entering` equals "the player stands on the loc's own tile".

/// One proc a crossing may end in: the index of its `$entering` argument,
/// of its replacement-loc argument (single doors) or its `^left`/`^right`
/// side (double doors, which swing `lc_param(loc_type, next_loc_stage)`),
/// the `(loc_type, shape, angle, loc_coord)` arguments a proc takes
/// explicitly instead of reading the active loc, and the normalized
/// teleport sequence its body must carry.
struct OpenProc {
    name: &'static str,
    entering: usize,
    leaf: Option<usize>,
    side: Option<usize>,
    loc_args: Option<[usize; 4]>,
    markers: &'static [&'static str],
}

/// `$x, $z = ~door_open(…)`, then the entering branch and the teleport
/// (`scripts/doors/scripts/open_and_close_doors.rs2:17-35`,
/// `open_and_close_double_doors.rs2:39-50`).
const ACTIVE_LOC_MARKERS: &[&str] = &[
    "$x,$z=~door_open($angle,loc_shape);",
    "if($entering=true){",
    "p_teleport($dest);",
];
/// The same sequence over the explicit shape argument
/// (`open_and_close_double_doors.rs2:118-160`).
const EXPLICIT_LOC_MARKERS: &[&str] = &[
    "$x,$z=~door_open($angle,$shape);",
    "if($entering=true){",
    "p_teleport($dest);",
];

const fn single(name: &'static str) -> OpenProc {
    OpenProc {
        name,
        entering: 1,
        leaf: Some(0),
        side: None,
        loc_args: None,
        markers: ACTIVE_LOC_MARKERS,
    }
}

const OPEN_PROCS: [OpenProc; 10] = [
    single("open_and_close_door"),
    single("open_and_close_door2"),
    single("open_and_close_door3"),
    single("open_and_close_door_fast"),
    single("open_and_close_door_dir"),
    single("open_and_close_metal_gate"),
    single("open_and_close_metal_gate2"),
    OpenProc {
        name: "open_and_close_double_door",
        entering: 0,
        leaf: None,
        side: Some(1),
        loc_args: None,
        markers: ACTIVE_LOC_MARKERS,
    },
    OpenProc {
        name: "open_and_close_double_door2",
        entering: 0,
        leaf: None,
        side: Some(1),
        loc_args: None,
        markers: ACTIVE_LOC_MARKERS,
    },
    OpenProc {
        name: "open_and_close_double_door3",
        entering: 0,
        leaf: None,
        side: Some(5),
        loc_args: Some([1, 2, 3, 4]),
        markers: EXPLICIT_LOC_MARKERS,
    },
];

/// Pure builtins an opener may bind or print (strings and arithmetic);
/// their value is not modelled.
const PURE_OPAQUE: [&str; 14] = [
    "lowercase",
    "uppercase",
    "tostring",
    "append",
    "text_gender",
    "displayname",
    "loc_name",
    "nc_name",
    "string_length",
    "add",
    "sub",
    "multiply",
    "divide",
    "modulo",
];

/// `[proc,check_axis]` as `door_procs.rs2` defines it (normalized): true
/// when the coord shares the loc's z (north/south walls) or x (east/west).
const CHECK_AXIS_BODY: &str = "switch_int($angle){case^loc_north,^loc_south:if(coordz($coord)=coordz($loc_coord)){return(true);}case^loc_west,^loc_east:if(coordx($coord)=coordx($loc_coord)){return(true);}}return(false);";
/// `[proc,check_axis_locactive]`: the same test against the active loc.
const CHECK_AXIS_LOCACTIVE_BODY: &str = "switch_int(loc_angle){case^loc_north,^loc_south:if(coordz($coord)=coordz(loc_coord)){return(true);}case^loc_west,^loc_east:if(coordx($coord)=coordx(loc_coord)){return(true);}}return(false);";

/// Statements an opener may run before the crossing without changing
/// where the player ends up or waiting on input.
const INERT_CALLS: [&str; 10] = [
    "mes",
    "obj_add",
    "sound_synth",
    "say",
    "p_delay",
    "p_arrivedelay",
    "facesquare",
    "anim",
    "spotanim_pl",
    "if_close",
];

/// Calls that move the player or hand control elsewhere; none may follow
/// the crossing.
const MOVES: [&str; 9] = [
    "p_teleport",
    "p_telejump",
    "p_exactmove",
    "p_walk",
    "~forcemove",
    "~agility_exactmove",
    "queue",
    "longqueue",
    "weakqueue",
];

/// Engine `PlayerStat` order (`engine/src/engine/entity/PlayerStat.ts`):
/// the `stat(<name>)` argument → skill id.
const STATS: [&str; 21] = [
    "attack",
    "defence",
    "strength",
    "hitpoints",
    "ranged",
    "prayer",
    "magic",
    "cooking",
    "woodcutting",
    "fletching",
    "fishing",
    "firemaking",
    "crafting",
    "smithing",
    "mining",
    "herblore",
    "agility",
    "thieving",
    "stat18",
    "stat19",
    "runecraft",
];

/// Jumps followed before an opener is refused (label cycles).
const MAX_JUMPS: usize = 16;

/// One `[kind,name]` block: its declared parameters (labels) and body.
#[derive(Debug, Clone)]
struct Block {
    params: Vec<String>,
    body: Vec<Stmt>,
}

/// The script facts an opener evaluation reads.
struct Sources {
    oploc1: HashMap<String, Vec<Option<Block>>>,
    labels: HashMap<String, Vec<Option<Block>>>,
    procs: HashMap<String, Vec<String>>,
    constants: HashMap<String, i32>,
    varps: HashMap<String, i32>,
    objs: HashMap<String, i32>,
    ids: HashMap<String, i32>,
}

impl Sources {
    fn read(content_root: &Path, ids: &HashMap<String, i32>) -> Self {
        let mut oploc1: HashMap<String, Vec<Option<Block>>> = HashMap::new();
        let mut labels: HashMap<String, Vec<Option<Block>>> = HashMap::new();
        let mut procs: HashMap<String, Vec<String>> = HashMap::new();
        visit_rs2(&content_root.join("scripts"), &mut |text| {
            for (kind, name, params, body) in header_blocks(text) {
                match kind.as_str() {
                    "oploc1" => oploc1
                        .entry(name)
                        .or_default()
                        .push(parse_body(&body).map(|body| Block { params, body })),
                    "label" => labels
                        .entry(name)
                        .or_default()
                        .push(parse_body(&body).map(|body| Block { params, body })),
                    "proc" => procs.entry(name).or_default().push(normalized_body(&body)),
                    _ => {}
                }
            }
        });
        Self {
            oploc1,
            labels,
            procs,
            constants: script_constants(content_root),
            varps: varp_ids_by_name(content_root),
            objs: obj_ids_by_name(content_root),
            ids: ids.clone(),
        }
    }

    /// The door procs the crossing model depends on, each defined once
    /// with the modelled body; a drifted proc is not supported.
    fn supported_open_procs(&self) -> HashSet<&'static str> {
        let one = |name: &str| match self.procs.get(name).map(Vec::as_slice) {
            Some([body]) => Some(body.as_str()),
            _ => None,
        };
        if one("check_axis") != Some(CHECK_AXIS_BODY)
            || one("check_axis_locactive") != Some(CHECK_AXIS_LOCACTIVE_BODY)
        {
            return HashSet::new();
        }
        OPEN_PROCS
            .iter()
            .filter(|p| one(p.name).is_some_and(|b| p.markers.iter().all(|m| b.contains(m))))
            .map(|p| p.name)
            .collect()
    }

    /// `[proc,climb_ladder]` defined once and ending in the `p_telejump`
    /// to its `$coord` (`ladders+stairs/scripts/ladders.rs2:154-161`).
    fn climb_supported(&self) -> bool {
        matches!(
            self.procs.get("climb_ladder").map(Vec::as_slice),
            Some([body]) if body.ends_with("p_telejump($coord);")
        )
    }

    fn one<'s>(map: &'s HashMap<String, Vec<Option<Block>>>, name: &str) -> Lookup<'s> {
        match map.get(name).map(Vec::as_slice) {
            None => Lookup::Missing,
            Some([Some(block)]) => Lookup::Found(block),
            Some(_) => Lookup::Unusable,
        }
    }
}

enum Lookup<'s> {
    Missing,
    Found(&'s Block),
    Unusable,
}

/// `[kind,name]` headers with their body text, including one-line bodies
/// (`[oploc1,herodoor_l] @open_heroes_guild(^left);`) and parameter lists
/// (`[label,open_legends_door](int $side)`), which `script_blocks` cannot
/// see. Returns `(kind, name, parameter names, body)`.
fn header_blocks(text: &str) -> Vec<(String, String, Vec<String>, String)> {
    let mut out = Vec::new();
    let mut cur: Option<(String, String, Vec<String>, String)> = None;
    for line in text.lines() {
        let header = line.strip_prefix('[').and_then(|rest| {
            let (inner, after) = rest.split_once(']')?;
            let (kind, name) = inner.split_once(',')?;
            let word = |s: &str| {
                !s.is_empty() && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
            };
            (word(kind) && word(name)).then(|| (kind.to_string(), name.to_string(), after))
        });
        if let Some((kind, name, after)) = header {
            if let Some(done) = cur.take() {
                out.push(done);
            }
            let (params, rest) = header_params(after);
            cur = Some((kind, name, params, format!("{rest}\n")));
        } else if let Some((_, _, _, body)) = cur.as_mut() {
            body.push_str(line);
            body.push('\n');
        }
    }
    if let Some(done) = cur {
        out.push(done);
    }
    out
}

/// A header's `(type $a, type $b)` parameter list (and a proc's return
/// list) → the parameter names and the text after them.
fn header_params(after: &str) -> (Vec<String>, &str) {
    let mut rest = after.trim_start();
    let mut params = Vec::new();
    let mut first = true;
    while let Some(inner) = rest.strip_prefix('(') {
        let Some((list, tail)) = inner.split_once(')') else {
            break;
        };
        if first {
            params = list
                .split(',')
                .filter_map(|p| p.split_whitespace().nth(1))
                .map(str::to_string)
                .collect();
        }
        first = false;
        rest = tail.trim_start();
    }
    (params, rest)
}

/// Requirements an evaluated path reads (see the module comment).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Needs {
    varps: Vec<(i32, i32)>,
    skills: Vec<(i32, i32)>,
    items: Vec<(i32, i32)>,
    worn: Vec<(i32, i32)>,
    members: bool,
}

impl Needs {
    fn merge(&mut self, other: Needs) {
        self.varps.extend(other.varps);
        self.skills.extend(other.skills);
        self.items.extend(other.items);
        self.worn.extend(other.worn);
        self.members |= other.members;
    }
}

/// Each key's largest minimum, sorted.
fn max_per_key(pairs: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let mut by: HashMap<i32, i32> = HashMap::new();
    for &(k, v) in pairs {
        let e = by.entry(k).or_insert(v);
        *e = (*e).max(v);
    }
    let mut out: Vec<_> = by.into_iter().collect();
    out.sort_unstable();
    out
}

/// A three-valued condition result with the requirements behind it.
struct Cond {
    value: Option<bool>,
    needs: Needs,
}

impl Cond {
    fn known(value: bool) -> Self {
        Cond {
            value: Some(value),
            needs: Needs::default(),
        }
    }

    fn unknown() -> Self {
        Cond {
            value: None,
            needs: Needs::default(),
        }
    }
}

/// A value an opener binds or passes.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Val {
    Bool(bool),
    Int(i32),
    Coord(WorldTile),
    /// A loc id, the loc's own `next_loc_stage`, or the loc itself.
    Loc(LocRef),
    /// A plain script name (synth, obj, category, …) or a string.
    Name(String),
    /// A tile relative to the player, for an opener run without a stand
    /// (a ladder climbed from any side): `movecoord(coord, dx, dy, dz)`.
    Rel(i32, i32, i32),
    /// Pure but not modelled (`loc_shape`, `~door_open(…)`).
    Opaque,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocRef {
    Id(i32),
    NextStage,
    Itself,
}

/// What the terminal call did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Arrival {
    /// An open-and-close proc walked the player through the wall; the
    /// leaf it places.
    Door(LocRef),
    /// `~climb_ladder` jumped the player to this tile.
    Climb(WorldTile),
}

/// Where a path ended.
enum Flow {
    Next,
    Return,
    /// The terminal call ran, and whether the script has already ended
    /// after it (a `return`, or a jumped label's end).
    Crossed(Arrival, bool),
    Refused,
}

/// One evaluation context: the placement and, for a wall crossing, the
/// stand tile (a ladder has none: it is climbed from any side).
struct Eval<'s> {
    src: &'s Sources,
    opens: &'s HashSet<&'static str>,
    at: WorldTile,
    stand: Option<WorldTile>,
    angle: i32,
    needs: Needs,
    jumps: usize,
}

type Env = HashMap<String, Val>;

impl Eval<'_> {
    fn run(&mut self, stmts: &[Stmt], env: &mut Env) -> Flow {
        for (k, stmt) in stmts.iter().enumerate() {
            match self.stmt(stmt, env) {
                Flow::Next => {}
                Flow::Crossed(leaf, true) => return Flow::Crossed(leaf, true),
                // The rest of this block still runs after the crossing: it
                // may print or talk, never move the player or jump. A
                // `return` ends the script there.
                Flow::Crossed(leaf, false) => {
                    for rest in &stmts[k + 1..] {
                        if *rest == Stmt::Return {
                            return Flow::Crossed(leaf, true);
                        }
                        if moves_or_jumps(rest) {
                            return Flow::Refused;
                        }
                    }
                    return Flow::Crossed(leaf, false);
                }
                other => return other,
            }
        }
        Flow::Next
    }

    fn stmt(&mut self, stmt: &Stmt, env: &mut Env) -> Flow {
        match stmt {
            Stmt::Block(body) => self.run(body, env),
            Stmt::Return => Flow::Return,
            // A branch that only prints or drops items cannot change the
            // crossing, so its condition is not a requirement.
            Stmt::If(arms, other)
                if arms.iter().all(|(_, body)| inert(body))
                    && other.as_deref().is_none_or(inert) =>
            {
                Flow::Next
            }
            Stmt::If(arms, other) => {
                for (cond, body) in arms {
                    let c = self.cond(cond, env);
                    let Some(value) = c.value else {
                        return Flow::Refused;
                    };
                    self.needs.merge(c.needs);
                    if value {
                        return self.run(body, env);
                    }
                }
                match other {
                    Some(body) => self.run(body, env),
                    None => Flow::Next,
                }
            }
            Stmt::Jump(name, args) => self.jump(name, args, env),
            Stmt::Def(var, value) => {
                let v = match value {
                    None => Some(Val::Opaque),
                    Some(e) => self.value(e, env),
                };
                match v {
                    Some(v) => {
                        env.insert(var.clone(), v);
                        Flow::Next
                    }
                    None => Flow::Refused,
                }
            }
            // A local rebinding keeps the path deterministic; a varp or
            // any other write changes game state and is refused.
            Stmt::Assign(lhs, value) => {
                if !lhs.iter().all(|w| w.starts_with('$')) {
                    return Flow::Refused;
                }
                let v = match lhs.as_slice() {
                    [_] => self.value(value, env),
                    _ => self.value(value, env).map(|_| Val::Opaque),
                };
                match v {
                    Some(v) => {
                        for w in lhs {
                            env.insert(w.clone(), v.clone());
                        }
                        Flow::Next
                    }
                    None => Flow::Refused,
                }
            }
            Stmt::Call(name, args) => {
                if let Some(proc) = name.strip_prefix('~') {
                    return self.open(proc, args, env);
                }
                if INERT_CALLS.contains(&name.as_str())
                    && args.iter().all(|a| self.value(a, env).is_some())
                {
                    Flow::Next
                } else {
                    Flow::Refused
                }
            }
            Stmt::Other(_) => Flow::Refused,
        }
    }

    fn jump(&mut self, name: &str, args: &[Expr], env: &Env) -> Flow {
        self.jumps += 1;
        if self.jumps > MAX_JUMPS {
            return Flow::Refused;
        }
        let Lookup::Found(block) = Sources::one(&self.src.labels, name) else {
            return Flow::Refused;
        };
        if block.params.len() != args.len() {
            return Flow::Refused;
        }
        let mut inner = Env::new();
        for (param, arg) in block.params.iter().zip(args) {
            let Some(v) = self.value(arg, env) else {
                return Flow::Refused;
            };
            inner.insert(param.clone(), v);
        }
        // A jump never returns to its caller: the label's end ends the
        // script.
        match self.run(&block.body, &mut inner) {
            Flow::Next => Flow::Return,
            Flow::Crossed(leaf, _) => Flow::Crossed(leaf, true),
            other => other,
        }
    }

    /// A `~proc(…)` statement: one of the supported crossing procs with
    /// the entering flag this stand needs, or a refusal.
    fn open(&mut self, proc: &str, args: &[Expr], env: &Env) -> Flow {
        if proc == "climb_ladder" {
            return self.climb(args, env);
        }
        let Some(spec) = OPEN_PROCS.iter().find(|p| p.name == proc) else {
            return Flow::Refused;
        };
        if !self.opens.contains(spec.name) {
            return Flow::Refused;
        }
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            let Some(v) = self.value(a, env) else {
                return Flow::Refused;
            };
            vals.push(v);
        }
        let Some(stand) = self.stand else {
            return Flow::Refused;
        };
        if vals.get(spec.entering) != Some(&Val::Bool(stand == self.at)) {
            return Flow::Refused;
        }
        // A proc handed the loc explicitly must be handed this loc.
        if let Some([ty, _shape, angle, coord]) = spec.loc_args {
            if vals.get(ty) != Some(&Val::Loc(LocRef::Itself))
                || vals.get(angle) != Some(&Val::Int(self.angle))
                || vals.get(coord) != Some(&Val::Coord(self.at))
            {
                return Flow::Refused;
            }
        }
        if let Some(side) = spec.side {
            let left = self.src.constants.get("left").copied();
            let right = self.src.constants.get("right").copied();
            let sided = matches!(vals.get(side), Some(Val::Int(s)) if Some(*s) == left || Some(*s) == right);
            return if sided {
                Flow::Crossed(Arrival::Door(LocRef::NextStage), false)
            } else {
                Flow::Refused
            };
        }
        match spec.leaf.and_then(|k| vals.get(k)) {
            Some(Val::Loc(leaf)) => Flow::Crossed(Arrival::Door(*leaf), false),
            Some(Val::Name(name)) => match loc_pack_id(name, &self.src.ids) {
                Some(id) => Flow::Crossed(Arrival::Door(LocRef::Id(id)), false),
                None => Flow::Refused,
            },
            _ => Flow::Refused,
        }
    }

    /// `~climb_ladder(<dest>, <up>)` on a ladder (no stand): the proc
    /// `p_telejump`s to `<dest>`. A player-relative dest bakes from the
    /// loc tile the way [`landing_tile`] does for `ladders.rs2` — a pure
    /// level change or a cellar `z ± 6400` shift; any horizontal shift
    /// depends on the side the player climbed from and is refused.
    fn climb(&mut self, args: &[Expr], env: &Env) -> Flow {
        if self.stand.is_some() || !self.src.climb_supported() {
            return Flow::Refused;
        }
        let [dest, up] = args else {
            return Flow::Refused;
        };
        if !matches!(self.value(up, env), Some(Val::Bool(_))) {
            return Flow::Refused;
        }
        let to = match self.value(dest, env) {
            Some(Val::Coord(to)) => to,
            Some(Val::Rel(0, dy, 0)) => WorldTile {
                level: self.at.level + dy,
                ..self.at
            },
            Some(Val::Rel(0, 0, dz)) if dz.abs() == CELLAR_SHIFT => WorldTile {
                z: self.at.z + dz,
                ..self.at
            },
            _ => return Flow::Refused,
        };
        Flow::Crossed(Arrival::Climb(to), false)
    }

    /// A pure expression's value, or `None` for anything with an effect
    /// or an unbound local.
    fn value(&self, e: &Expr, env: &Env) -> Option<Val> {
        match e {
            Expr::Num(n) => Some(Val::Int(*n)),
            Expr::Str => Some(Val::Name(String::new())),
            Expr::Word(w) => self.word(w, env),
            Expr::Call(name, args) => self.call_value(name, args, env),
            Expr::Cmp(..) | Expr::And(..) | Expr::Or(..) | Expr::Other => None,
        }
    }

    fn word(&self, w: &str, env: &Env) -> Option<Val> {
        Some(match w {
            "true" => Val::Bool(true),
            "false" => Val::Bool(false),
            "coord" => match self.stand {
                Some(stand) => Val::Coord(stand),
                None => Val::Rel(0, 0, 0),
            },
            "loc_coord" => Val::Coord(self.at),
            "loc_angle" => Val::Int(self.angle),
            "loc_type" => Val::Loc(LocRef::Itself),
            "loc_shape" | "null" => Val::Opaque,
            _ if w.starts_with('$') => env.get(w)?.clone(),
            _ if w.starts_with('^') => Val::Int(*self.src.constants.get(&w[1..])?),
            _ if w.starts_with(['%', '~', '@', '.']) => return None,
            _ => match coord_literal(w) {
                Some((level, x, z)) => Val::Coord(WorldTile { x, z, level }),
                None => Val::Name(w.to_string()),
            },
        })
    }

    fn call_value(&self, name: &str, args: &[Expr], env: &Env) -> Option<Val> {
        let vals: Option<Vec<Val>> = args.iter().map(|a| self.value(a, env)).collect();
        let vals = vals?;
        match (name, vals.as_slice()) {
            ("~check_axis", [Val::Coord(c), Val::Coord(l), Val::Int(angle)]) => {
                Some(Val::Bool(check_axis(*c, *l, *angle)))
            }
            ("~check_axis_locactive", [Val::Coord(c)]) => {
                Some(Val::Bool(check_axis(*c, self.at, self.angle)))
            }
            ("coordx", [Val::Coord(c)]) => Some(Val::Int(c.x)),
            ("coordz", [Val::Coord(c)]) => Some(Val::Int(c.z)),
            ("coordy", [Val::Coord(c)]) => Some(Val::Int(c.level)),
            ("movecoord", [Val::Coord(c), Val::Int(dx), Val::Int(dy), Val::Int(dz)]) => {
                Some(Val::Coord(WorldTile {
                    x: c.x + dx,
                    z: c.z + dz,
                    level: c.level + dy,
                }))
            }
            ("movecoord", [Val::Rel(x, y, z), Val::Int(dx), Val::Int(dy), Val::Int(dz)]) => {
                Some(Val::Rel(x + dx, y + dy, z + dz))
            }
            ("loc_param", [Val::Name(p)]) if p == "next_loc_stage" => {
                Some(Val::Loc(LocRef::NextStage))
            }
            ("lc_param", [Val::Loc(LocRef::Itself), Val::Name(p)]) if p == "next_loc_stage" => {
                Some(Val::Loc(LocRef::NextStage))
            }
            ("~door_open", [_, _]) | ("loc_param", [_]) => Some(Val::Opaque),
            _ if PURE_OPAQUE.contains(&name) => Some(Val::Opaque),
            _ => None,
        }
    }

    /// A condition under the "past every threshold" assumption.
    fn cond(&self, e: &Expr, env: &Env) -> Cond {
        match e {
            // Either true disjunct decides; the cheaper proof is kept.
            Expr::Or(a, b) => {
                let (a, b) = (self.cond(a, env), self.cond(b, env));
                match (a.value, b.value) {
                    (Some(true), Some(true)) => cheaper(a, b),
                    (Some(true), _) => a,
                    (_, Some(true)) => b,
                    (Some(false), Some(false)) => both(false, a, b),
                    _ => Cond::unknown(),
                }
            }
            // Either false conjunct decides; the cheaper proof is kept.
            Expr::And(a, b) => {
                let (a, b) = (self.cond(a, env), self.cond(b, env));
                match (a.value, b.value) {
                    (Some(false), Some(false)) => cheaper(a, b),
                    (Some(false), _) => a,
                    (_, Some(false)) => b,
                    (Some(true), Some(true)) => both(true, a, b),
                    _ => Cond::unknown(),
                }
            }
            Expr::Cmp(op, lhs, rhs) => self.compare(*op, lhs, rhs, env),
            _ => Cond::unknown(),
        }
    }

    fn compare(&self, op: CmpOp, lhs: &Expr, rhs: &Expr, env: &Env) -> Cond {
        let Some(rhs_val) = self.value(rhs, env) else {
            return Cond::unknown();
        };
        match lhs {
            Expr::Word(w) if w.starts_with('%') => {
                let (Some(&id), Val::Int(c)) = (self.src.varps.get(&w[1..]), rhs_val) else {
                    return Cond::unknown();
                };
                let (value, min) = past_threshold(op, c);
                Cond {
                    value: Some(value),
                    needs: Needs {
                        varps: vec![(id, min)],
                        ..Needs::default()
                    },
                }
            }
            Expr::Word(w) if w == "map_members" => {
                // A members world: `map_members` reads `^true`.
                let (Some(&t), Val::Int(c)) = (self.src.constants.get("true"), rhs_val) else {
                    return Cond::unknown();
                };
                let value = match op {
                    CmpOp::Eq => c == t,
                    CmpOp::Ne => c != t,
                    _ => return Cond::unknown(),
                };
                Cond {
                    value: Some(value),
                    needs: Needs {
                        members: true,
                        ..Needs::default()
                    },
                }
            }
            Expr::Call(f, args) if f == "stat" => {
                let (Some(skill), Val::Int(c)) = (stat_id(args), rhs_val) else {
                    return Cond::unknown();
                };
                let (value, min) = past_threshold(op, c);
                Cond {
                    value: Some(value),
                    needs: Needs {
                        skills: vec![(skill, min)],
                        ..Needs::default()
                    },
                }
            }
            Expr::Call(f, args) if f == "inv_total" => {
                let ([Expr::Word(inv), Expr::Word(obj)], Val::Int(c)) = (args.as_slice(), rhs_val)
                else {
                    return Cond::unknown();
                };
                let Some(&obj) = self.src.objs.get(obj) else {
                    return Cond::unknown();
                };
                let (value, min) = past_threshold(op, c);
                let needs = match inv.as_str() {
                    "inv" => Needs {
                        items: vec![(obj, min)],
                        ..Needs::default()
                    },
                    // One worn obj of a kind is all a slot holds.
                    "worn" if min <= 1 => Needs {
                        worn: vec![(obj, min)],
                        ..Needs::default()
                    },
                    _ => return Cond::unknown(),
                };
                Cond {
                    value: Some(value),
                    needs,
                }
            }
            _ => {
                let Some(lhs_val) = self.value(lhs, env) else {
                    return Cond::unknown();
                };
                match (lhs_val, rhs_val) {
                    (Val::Int(a), Val::Int(b)) => Cond::known(match op {
                        CmpOp::Eq => a == b,
                        CmpOp::Ne => a != b,
                        CmpOp::Lt => a < b,
                        CmpOp::Le => a <= b,
                        CmpOp::Gt => a > b,
                        CmpOp::Ge => a >= b,
                    }),
                    (Val::Bool(a), Val::Bool(b)) => match op {
                        CmpOp::Eq => Cond::known(a == b),
                        CmpOp::Ne => Cond::known(a != b),
                        _ => Cond::unknown(),
                    },
                    (Val::Coord(a), Val::Coord(b)) => match op {
                        CmpOp::Eq => Cond::known(a == b),
                        CmpOp::Ne => Cond::known(a != b),
                        _ => Cond::unknown(),
                    },
                    _ => Cond::unknown(),
                }
            }
        }
    }
}

/// Of two conditions with the same outcome, the one whose requirements
/// are cheaper for the bot to hold: none, then quest progress, then levels
/// and world membership, then items it must carry or wear.
fn cheaper(a: Cond, b: Cond) -> Cond {
    let cost = |n: &Needs| {
        n.varps.len()
            + 2 * n.skills.len()
            + usize::from(n.members) * 2
            + 4 * (n.items.len() + n.worn.len())
    };
    if cost(&b.needs) < cost(&a.needs) {
        b
    } else {
        a
    }
}

fn both(value: bool, a: Cond, b: Cond) -> Cond {
    let mut needs = a.needs;
    needs.merge(b.needs);
    Cond {
        value: Some(value),
        needs,
    }
}

/// `x <op> c` for an `x` past every threshold, and the smallest `x` with
/// the same outcome.
fn past_threshold(op: CmpOp, c: i32) -> (bool, i32) {
    match op {
        CmpOp::Ge => (true, c),
        CmpOp::Lt => (false, c),
        CmpOp::Gt | CmpOp::Ne => (true, c.saturating_add(1)),
        CmpOp::Le | CmpOp::Eq => (false, c.saturating_add(1)),
    }
}

fn stat_id(args: &[Expr]) -> Option<i32> {
    let [Expr::Word(name)] = args else {
        return None;
    };
    STATS.iter().position(|s| s == name).map(|i| i as i32)
}

/// `door_procs.rs2` `[proc,check_axis]`: the coord shares the loc's z for
/// a north/south wall, its x for a west/east wall.
fn check_axis(coord: WorldTile, loc: WorldTile, angle: i32) -> bool {
    match angle {
        1 | 3 => coord.z == loc.z,
        0 | 2 => coord.x == loc.x,
        _ => false,
    }
}

/// Statements that neither move, wait on input, write state nor bind:
/// inert calls and `if`s made only of them.
fn inert(stmts: &[Stmt]) -> bool {
    stmts.iter().all(|stmt| match stmt {
        Stmt::Call(name, _) => INERT_CALLS.contains(&name.as_str()),
        Stmt::Block(body) => inert(body),
        Stmt::If(arms, other) => {
            arms.iter().all(|(_, body)| inert(body)) && other.as_deref().is_none_or(inert)
        }
        _ => false,
    })
}

fn moves_or_jumps(stmt: &Stmt) -> bool {
    let mut calls = Vec::new();
    stmt.calls(&mut calls);
    calls.iter().any(|c| {
        c.starts_with('@')
            || c == "~climb_ladder"
            || MOVES.contains(&c.as_str())
            || OPEN_PROCS
                .iter()
                .any(|p| c.strip_prefix('~') == Some(p.name))
    })
}

/// A loc's `category=` and `param=next_loc_stage` from every `.loc`
/// config (the `_unpack` trees included — the pack tool compiles them). A
/// loc declared twice with different values keeps neither.
fn loc_config_facts(
    content_root: &Path,
    ids: &HashMap<String, i32>,
) -> HashMap<i32, (Option<String>, Option<i32>)> {
    let mut out: HashMap<i32, (Option<String>, Option<i32>)> = HashMap::new();
    let mut conflicted: HashSet<i32> = HashSet::new();
    visit_loc_configs(&content_root.join("scripts"), &mut |text| {
        let mut cur: Option<(i32, Option<String>, Option<i32>)> = None;
        let mut flush = |cur: &mut Option<(i32, Option<String>, Option<i32>)>| {
            if let Some((id, category, stage)) = cur.take() {
                match out.get(&id) {
                    Some(prev) if *prev != (category.clone(), stage) => {
                        out.remove(&id);
                        conflicted.insert(id);
                    }
                    Some(_) => {}
                    None if conflicted.contains(&id) => {}
                    None => {
                        out.insert(id, (category, stage));
                    }
                }
            }
        };
        for raw in text.lines() {
            let line = raw.trim();
            if let Some(name) = config_header(line) {
                flush(&mut cur);
                cur = loc_pack_id(name, ids).map(|id| (id, None, None));
            } else if let Some((_, category, stage)) = cur.as_mut() {
                if let Some(value) = line.strip_prefix("category=") {
                    *category = Some(value.trim().to_string());
                } else if let Some(value) = line.strip_prefix("param=next_loc_stage,") {
                    *stage = stage_open_loc_id(value.trim(), ids);
                }
            }
        }
        flush(&mut cur);
    });
    out
}

/// Every block reachable from `block` through `@label` jumps (bounded),
/// and whether any names a terminal call — the loc is an opener at all.
fn names_terminal(src: &Sources, block: &Block) -> bool {
    let mut pending = vec![block];
    let mut seen: HashSet<*const Block> = HashSet::new();
    while let Some(b) = pending.pop() {
        if !seen.insert(b as *const Block) || seen.len() > MAX_JUMPS {
            continue;
        }
        let mut calls = Vec::new();
        b.body.iter().for_each(|s| s.calls(&mut calls));
        for c in &calls {
            if let Some(label) = c.strip_prefix('@') {
                if let Lookup::Found(next) = Sources::one(&src.labels, label) {
                    pending.push(next);
                }
            } else if c == "~climb_ladder"
                || OPEN_PROCS
                    .iter()
                    .any(|p| c.strip_prefix('~') == Some(p.name))
            {
                return true;
            }
        }
    }
    false
}

/// Stage-door crossings and guarded ladder climbs for every loc no earlier
/// producer covered (see the module comment): straight-wall placements
/// get a Door edge per proven crossing, centre-shaped ones a Ladder edge
/// per proven climb. The loc's own named `[oploc1]` block wins over its
/// category's; a loc whose opener never names a crossing proc or a climb
/// is not an opener and is not counted. A handler declared twice, a loc
/// whose names carry more than one block, or an opener that crosses
/// nowhere is counted and left out.
#[allow(clippy::too_many_arguments)]
pub(super) fn stage_door_edges(
    content_root: &Path,
    ids: &HashMap<String, i32>,
    positions: &HashMap<i32, Vec<Placement>>,
    graph: &mut TransportGraph,
    collision: &WorldCollision,
    skipped: &mut HashMap<&'static str, usize>,
    observable: &ObservableGates,
    audit: &mut VarpGateAudit,
) {
    let src = Sources::read(content_root, ids);
    let opens = src.supported_open_procs();
    if opens.is_empty() {
        bump(skipped, SKIP_STAGE_DOOR_PROCS, 1);
        return;
    }
    let facts = loc_config_facts(content_root, ids);
    // Loc-backed kinds only: an NPC hop's `loc_id` is an npc.pack id.
    let covered: HashSet<i32> = graph
        .edges
        .iter()
        .filter(|e| {
            matches!(
                e.kind,
                TransportKind::Door
                    | TransportKind::Ladder
                    | TransportKind::Stairs
                    | TransportKind::AgilityShortcut
                    | TransportKind::SpiritTree
            )
        })
        .map(|e| e.loc_id)
        .collect();
    // `ladders.rs2`/`stairs.rs2` blocks belong to the ladder/stairs
    // producer, which prices them per loc name (an unpriced one stays out).
    let vertical = vertical_rule_names(content_root);
    let mut names: HashMap<i32, Vec<&str>> = HashMap::new();
    for (name, &id) in ids {
        names.entry(id).or_default().push(name.as_str());
    }
    let mut loc_ids: Vec<i32> = positions.keys().copied().collect();
    loc_ids.sort_unstable();
    for id in loc_ids {
        if covered.contains(&id)
            || names
                .get(&id)
                .is_some_and(|ns| ns.iter().any(|n| vertical.contains(*n)))
        {
            continue;
        }
        let placements = &positions[&id];
        if !placements.iter().any(|p| placement_role(p).is_some()) {
            continue;
        }
        let (category, stage) = facts.get(&id).cloned().unwrap_or((None, None));
        let own: Vec<Lookup> = names
            .get(&id)
            .into_iter()
            .flatten()
            .map(|n| Sources::one(&src.oploc1, n))
            .filter(|l| !matches!(l, Lookup::Missing))
            .collect();
        let handler = match own.as_slice() {
            [Lookup::Found(block)] => *block,
            [] => match category.map(|c| Sources::one(&src.oploc1, &format!("_{c}"))) {
                Some(Lookup::Found(block)) => block,
                Some(Lookup::Unusable) => {
                    bump(skipped, SKIP_STAGE_DOOR_CONFLICT, 1);
                    continue;
                }
                _ => continue,
            },
            _ => {
                bump(skipped, SKIP_STAGE_DOOR_CONFLICT, 1);
                continue;
            }
        };
        if !names_terminal(&src, handler) {
            continue;
        }
        let emitted = graph.edges.len();
        let mut proven = false;
        for p in placements {
            let at = WorldTile {
                x: p.x,
                z: p.z,
                level: p.level,
            };
            let eval = |stand: Option<WorldTile>| {
                let mut eval = Eval {
                    src: &src,
                    opens: &opens,
                    at,
                    stand,
                    angle: p.angle,
                    needs: Needs::default(),
                    jumps: 0,
                };
                match eval.run(&handler.body, &mut Env::new()) {
                    Flow::Crossed(arrival, _) => Some((arrival, eval.needs)),
                    _ => None,
                }
            };
            match placement_role(p) {
                Some(Role::Wall(angle_dir)) => {
                    for dir in [angle_dir, opposite(angle_dir)] {
                        // The stand for the crossing along the angle is the
                        // loc's own tile; the reverse starts across the wall.
                        let stand = if dir == angle_dir {
                            at
                        } else {
                            let (dx, dz) = dir_delta(angle_dir);
                            WorldTile {
                                x: at.x + dx,
                                z: at.z + dz,
                                level: at.level,
                            }
                        };
                        let Some((Arrival::Door(leaf), needs)) = eval(Some(stand)) else {
                            continue;
                        };
                        let leaf = match leaf {
                            LocRef::Id(leaf) => Some(leaf),
                            LocRef::NextStage => stage,
                            LocRef::Itself => None,
                        }
                        .filter(|&leaf| leaf != id);
                        let Some(to) = door_far_side(at, dir, collision) else {
                            continue;
                        };
                        let Some(mut edge) = gated_edge(TransportKind::Door, id, at, to, needs)
                        else {
                            continue;
                        };
                        edge.dir = Some(dir);
                        edge.open_loc_id = leaf;
                        proven = true;
                        observable.admit_edge(graph, edge, audit);
                    }
                }
                Some(Role::Ladder) => {
                    let Some((Arrival::Climb(to), needs)) = eval(None) else {
                        continue;
                    };
                    // `~climb_ladder`'s price is the measured ladders.rs2
                    // one: `ladder_cellar` runs the same
                    // `p_arrivedelay; ~climb_ladder(movecoord(coord(), 0, 0, 6400), …)`.
                    let Some(extra) = extra_ticks("ladder_cellar") else {
                        continue;
                    };
                    if !in_world_box(&to) {
                        continue;
                    }
                    let Some(mut edge) = gated_edge(TransportKind::Ladder, id, at, to, needs)
                    else {
                        continue;
                    };
                    edge.ticks = 1 + extra;
                    proven = true;
                    observable.admit_edge(graph, edge, audit);
                }
                None => {}
            }
        }
        if !proven {
            bump(skipped, SKIP_STAGE_DOOR_UNPROVEN, 1);
        } else if graph.edges.len() == emitted {
            bump(skipped, SKIP_STAGE_DOOR_UNOBSERVABLE, 1);
        }
    }
}

/// Loc names with an `[oploc…]` block in `ladders+stairs/scripts/
/// {ladders,stairs}.rs2` (the files [`ladder_stair_edges`] reads).
fn vertical_rule_names(content_root: &Path) -> HashSet<String> {
    let dir = content_root
        .join("scripts")
        .join("ladders+stairs")
        .join("scripts");
    let mut out = HashSet::new();
    for file in ["ladders.rs2", "stairs.rs2"] {
        let Ok(text) = fs::read_to_string(dir.join(file)) else {
            continue;
        };
        for (kind, name, _, _) in header_blocks(&text) {
            if oploc_option(&kind).is_some() {
                out.insert(name);
            }
        }
    }
    out
}

/// How a placement is used: a straight wall crossed along its angle, or a
/// centre-shaped loc (a ladder) climbed from any side.
enum Role {
    Wall(DoorDir),
    Ladder,
}

fn placement_role(p: &Placement) -> Option<Role> {
    match p.shape {
        LocShape::WALL_STRAIGHT => door_dir(p.angle).map(Role::Wall),
        LocShape::CENTREPIECE_STRAIGHT | LocShape::CENTREPIECE_DIAGONAL => Some(Role::Ladder),
        _ => None,
    }
}

/// An edge carrying the evaluated requirements (option 1, one tick; the
/// caller fills `dir`/`open_loc_id`/`ticks`), or `None` when its worn gate
/// needs two different objs at once (`worn_req` is any-one-of).
fn gated_edge(
    kind: TransportKind,
    id: i32,
    at: WorldTile,
    to: WorldTile,
    needs: Needs,
) -> Option<TransportEdge> {
    let worn = max_per_key(&needs.worn);
    if worn.len() > 1 {
        return None;
    }
    Some(TransportEdge {
        kind,
        at,
        to,
        loc_id: id,
        option: 1,
        ticks: 1,
        dir: None,
        open_loc_id: None,
        skill_req: max_per_key(&needs.skills),
        item_req: max_per_key(&needs.items),
        quest_req: vec![],
        varp_req: max_per_key(&needs.varps),
        worn_req: worn.into_iter().map(|(obj, _)| obj).collect(),
        members_req: needs.members,
        wildy_cap: None,
    })
}
