//! Symbolic path execution, coverage, and entry-local limits.

use crate::{Coord, LocTypeId, OpCount, ScriptId, Site, Ticks, VarpId};
use crate::profile::EngineProfile;
use crate::program::Program;
use crate::world::{LocObj, World, WorldOverlay, ZoneKey};

mod domains;
mod execute;
mod rows;
mod transfers;

/// An interned expression in one entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ExprId(pub u32);
/// An interned input or observation in one entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct AtomId(pub u32);
/// An integer stack value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Value {
    /// An engine integer.
    Known(i32),
    /// An expression over observations.
    Sym(ExprId),
    /// An unmodelled result, identified by its producing instruction.
    Opaque(Site),
}
/// A string stack value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Text {
    /// A known string.
    Lit(Box<str>),
    /// An unmodelled string.
    Unknown(Site),
}
/// A packed coordinate component.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Axis {
    /// East-west coordinate.
    X,
    /// Height plane.
    Level,
    /// North-south coordinate.
    Z,
}
/// A modelled expression. Arithmetic outside these forms is opaque.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expr {
    /// A direct observation.
    Atom(AtomId),
    /// Engine-truncated addition.
    AddConst { /// Base expression.
        of: ExprId, /// Added integer.
        c: i32 },
    /// Inclusive bit range.
    Bits { /// Base expression.
        of: ExprId, /// First bit.
        lo: u8, /// Last bit.
        hi: u8 },
    /// Masked update of an inclusive bit range.
    WithBits { /// Base expression.
        of: ExprId, /// First bit.
        lo: u8, /// Last bit.
        hi: u8, /// Replacement bits.
        to: Value },
    /// Masked coordinate offsets.
    CoordPlus { /// Base coordinate expression.
        of: ExprId, /// X offset.
        dx: i32, /// Level offset.
        dlevel: i32, /// Z offset.
        dz: i32 },
    /// One component of a coordinate.
    CoordPart { /// Base coordinate expression.
        of: ExprId, /// Selected component.
        axis: Axis },
}
/// A text-layout observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LayoutKind {
    /// Number of pages after initialization.
    Pages,
    /// Number of lines on a proved-valid page.
    Lines(Value),
}
/// An observation's source, independent of its value.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AtomKind {
    /// A whole backing varp.
    Varp(VarpId),
    /// A current or base skill level.
    Stat { /// Skill index.
        id: i32, /// True for the base level.
        base: bool },
    /// An inventory item count.
    InvTotal { /// Inventory id.
        inv: i32, /// Object type.
        obj: i32 },
    /// An inventory category count.
    InvCat { /// Inventory id.
        inv: i32, /// Category id.
        category: i32 },
    /// The object in an inventory slot.
    InvSlot { /// Inventory id.
        inv: i32, /// Slot index.
        slot: i32 },
    /// Whether this world is members-only.
    Members,
    /// An unknown entry tile.
    Stand,
    /// An unknown procedure argument.
    Arg(u8),
    /// The one player's unique id.
    Uid,
    /// A layout result.
    Layout(LayoutKind),
    /// A fresh observation that can change between reads.
    Live(&'static str),
}
/// An observation's stable identity across sibling paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Atom {
    /// Observation source.
    pub kind: AtomKind,
    /// Suspension generation.
    pub epoch: u32,
    /// Earlier executions of the producing instruction on this path.
    pub nth: u32,
    /// Producing instruction.
    pub site: Site,
    /// How the caller can control this observation.
    pub class: CondClass,
    /// Initial inclusive numeric domain.
    pub bounds: (i32, i32),
}
/// Interned entry-local expressions. No value outlives its run.
#[derive(Debug, Default)]
pub struct Arena {
    /// Stable observation records.
    pub atoms: Vec<Atom>,
    /// Stable expression records.
    pub exprs: Vec<Expr>,
    /// Instructions that produced a random nearby coordinate.
    pub near: Vec<Site>,
}
impl Arena {
    /// Read an expression without indexing malformed input.
    pub fn expr(&self, id: ExprId) -> Option<&Expr> { self.exprs.get(id.0 as usize) }
    /// Read an observation without indexing malformed input.
    pub fn atom(&self, id: AtomId) -> Option<&Atom> { self.atoms.get(id.0 as usize) }
    /// Resolve a plain atom value.
    pub fn atom_of(&self, value: Value) -> Option<&Atom> {
        let Value::Sym(id) = value else { return None; };
        let Expr::Atom(atom) = self.expr(id)? else { return None; };
        self.atom(*atom)
    }
}
/// Comparison in engine integer order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CmpOp {
    /// Equal.
    Eq,
    /// Unequal.
    Ne,
    /// Less than.
    Lt,
    /// Less than or equal.
    Le,
    /// Greater than.
    Gt,
    /// Greater than or equal.
    Ge,
}
impl CmpOp {
    /// The complementary relation.
    pub const fn negate(self) -> Self { match self { Self::Eq=>Self::Ne,Self::Ne=>Self::Eq,Self::Lt=>Self::Ge,Self::Le=>Self::Gt,Self::Gt=>Self::Le,Self::Ge=>Self::Lt } }
    /// The relation with operands exchanged.
    pub const fn reverse(self) -> Self { match self {Self::Lt=>Self::Gt,Self::Le=>Self::Ge,Self::Gt=>Self::Lt,Self::Ge=>Self::Le,other=>other} }
    /// Compare concrete engine integers.
    pub const fn test(self,a:i32,b:i32)->bool { match self {Self::Eq=>a==b,Self::Ne=>a!=b,Self::Lt=>a<b,Self::Le=>a<=b,Self::Gt=>a>b,Self::Ge=>a>=b} }
}
/// A complete branch predicate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pred {
    /// A relation between integer values.
    Cmp { /// Left value.
        lhs: Value, /// Relation.
        op: CmpOp, /// Right value.
        rhs: Value },
    /// The keys that share one switch target.
    In { /// Switch key.
        of: Value, /// Distinct keys.
        set: Box<[i32]> },
}
/// How the caller can control a predicate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CondClass {
    /// Observable account requirement.
    Requirement,
    /// Account state unavailable to the planner.
    Hidden,
    /// An entry argument or stand.
    Input,
    /// A changing observation.
    Live,
    /// An unmodelled value.
    Opaque,
}
/// One full branch decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cond {
    /// Unweakened predicate.
    pub pred: Pred,
    /// The path's selected truth value.
    pub truth: bool,
    /// Least controllable input of the predicate.
    pub class: CondClass,
    /// Branch location.
    pub site: Site,
    /// Fork identity.
    pub choice: u32,
}
/// The player's final or current tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerPos {
    /// Concrete packed coordinate.
    Tile(Coord),
    /// Offsets from an unknown entry stand.
    Rel { /// X offset.
        dx: i32, /// Level offset.
        dlevel: i32, /// Z offset.
        dz: i32 },
    /// A random nearby tile, without a proved outcome set.
    Near(Site),
    /// A typed coordinate argument with no concrete or stand-relative value.
    Unknown(Value),
}
/// Whether a script certainly suspended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Suspension {
    /// No suspension occurred.
    No,
    /// A suspension might have occurred.
    Maybe,
    /// A suspension occurred.
    Yes,
}
/// Modal interfaces that can own an input pause.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Modal {
    /// Main interface, if open.
    pub main: Option<i32>,
    /// Chat interface, if open.
    pub chat: Option<i32>,
    /// Side interface, if open.
    pub side: Option<i32>,
}
/// Movement command class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveKind {
    /// Ordinary teleport.
    Teleport,
    /// Jump teleport.
    Telejump,
    /// Exact movement with original engine operands.
    Exact { /// Start coordinate.
        start: Value, /// End coordinate.
        end: Value, /// Start cycle.
        start_cycle: Value, /// End cycle.
        end_cycle: Value, /// Facing direction.
        direction: Value },
}
/// Integer or string varp storage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stored {
    /// An integer value.
    Int(Value),
    /// A string value.
    Str(Text),
}
/// A loc mutation's operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocKind {
    /// An add command, possibly changing a placed object.
    Add,
    /// A type change.
    Change,
    /// A removal.
    Del,
}
/// Deferred work operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeferKind {
    /// Start work that this machine does not run.
    Start,
    /// Cancel deferred work.
    Cancel,
}
/// An observable engine action, in dispatch order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// The player moved.
    Move { /// Command class.
        kind: MoveKind, /// Resulting position.
        to: PlayerPos },
    /// Zone allocation prevented movement.
    NoMove { /// Rejected destination.
        to: Coord },
    /// A script delay.
    Delay { /// Least ticks.
        min: u32, /// Greatest ticks.
        max: u32 },
    /// An update to a backing variable.
    VarWrite { /// Backing varp.
        varp: VarpId, /// Written bit range, or whole value.
        bits: Option<(u8,u8)>, /// Stored value.
        value: Stored },
    /// Inventory mutation.
    Inv { /// Command arguments.
        args: [Value;3] },
    /// Loc mutation.
    Loc { /// Mutation class.
        kind: LocKind, /// Stable object identity.
        obj: LocObj, /// New or removed type.
        ty: LocTypeId, /// Engine duration.
        duration: i32 },
    /// An NPC effect outside the navigation view.
    Npc { /// Command name.
        command: &'static str },
    /// A ground-object effect outside the navigation view.
    Obj { /// Command name.
        command: &'static str },
    /// Deferred work.
    Defer { /// Start or cancel.
        kind: DeferKind, /// Command name.
        command: &'static str },
    /// An interface opened or closed.
    Modal { /// New modal state.
        state: Modal },
    /// The weak queue was cleared by explicit close or cancellation.
    WeakQueueCleared,
    /// A player message.
    Message { /// Displayed text.
        text: Text },
    /// Cosmetic action.
    Cosmetic { /// Command name.
        command: &'static str },
}
/// An effect with its engine location and elapsed ticks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectRecord {
    /// The action.
    pub effect: Effect,
    /// Producing instruction.
    pub site: Site,
    /// Elapsed ticks at the action.
    pub ticks: Ticks,
}
/// Effect classes compared at joins and live-loop summaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct View(pub u16);
impl View {
    /// Movement effects.
    pub const MOVE: Self = Self(1);
    /// Time effects.
    pub const TIME: Self = Self(2);
    /// Varp effects.
    pub const VAR: Self = Self(4);
    /// Inventory effects.
    pub const INV: Self = Self(8);
    /// Loc effects.
    pub const LOC: Self = Self(16);
    /// NPC effects.
    pub const NPC: Self = Self(32);
    /// Ground-object effects.
    pub const OBJ: Self = Self(64);
    /// Deferred work effects.
    pub const DEFER: Self = Self(128);
    /// Modal state effects.
    pub const MODAL: Self = Self(256);
    /// Message effects.
    pub const MESSAGE: Self = Self(512);
    /// Cosmetic effects.
    pub const COSMETIC: Self = Self(1024);
    /// Everything that navigation compares.
    pub const NAV: Self = Self(1|2|4|8|16|128|256);
    /// Every effect class.
    pub const ALL: Self = Self(2047);
    pub(crate) fn holds(self,e:&Effect)->bool {
        let flag=match e {Effect::Move{..}|Effect::NoMove{..}=>Self::MOVE,Effect::Delay{..}=>Self::TIME,Effect::VarWrite{..}=>Self::VAR,Effect::Inv{..}=>Self::INV,Effect::Loc{..}=>Self::LOC,Effect::Npc{..}=>Self::NPC,Effect::Obj{..}=>Self::OBJ,Effect::Defer{..}=>Self::DEFER,Effect::Modal{..}|Effect::WeakQueueCleared=>Self::MODAL,Effect::Message{..}=>Self::MESSAGE,Effect::Cosmetic{..}=>Self::COSMETIC};
        self.0 & flag.0 != 0
    }
}
impl std::ops::BitOr for View { type Output=Self; fn bitor(self,rhs:Self)->Self {Self(self.0|rhs.0)} }
/// A located engine fault, unsupported transfer, or path bound.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{reason} at script {script} instruction {pc}", script=.at.script.0,pc=.at.pc)]
pub struct Fault {
    /// Why execution stopped.
    pub reason: std::borrow::Cow<'static,str>,
    /// Instruction that could not continue.
    pub at: Site,
}
/// A path's terminal state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum End {
    /// The entry returned.
    Finished,
    /// The first input pause, after cancellation.
    AwaitingInput,
    /// The engine can throw at this instruction.
    EngineError(Fault),
    /// The machine cannot prove the transfer.
    Unsupported(Fault),
    /// A path-local analysis cap.
    BoundReached(Fault),
}
/// One complete or fail-closed path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathResult {
    /// Terminal outcome.
    pub end: End,
    /// Last dispatched instruction.
    pub at: Site,
    /// Call sites, outermost first.
    pub chain: Vec<Site>,
    /// Full recorded path conditions.
    pub conds: Vec<Cond>,
    /// Ordered engine effects.
    pub effects: Vec<EffectRecord>,
    /// Final position.
    pub player: PlayerPos,
    /// Elapsed tick range.
    pub ticks: Ticks,
    /// Dispatched instruction range.
    pub ops: OpCount,
    /// Loc reads that depend on entry assumption 5.
    pub world_reads_after_suspend: u32,
    /// Modal state after terminal transfers.
    pub modal: Modal,
    /// Integer stack at the end, useful for procedure oracles.
    pub ints: Vec<Value>,
    /// String stack at the end, useful for procedure oracles.
    pub strings: Vec<Text>,
    pub(crate) trail: Vec<(u32,u16)>,
}
/// Entry execution inputs.
#[derive(Debug)]
pub struct Entry {
    /// First script.
    pub script: ScriptId,
    /// Known procedure arguments. Missing arguments become input atoms.
    pub ints: Vec<Value>,
    /// Known string arguments. Missing arguments become unknown strings.
    pub strings: Vec<Text>,
    /// Primary active loc for an operation trigger.
    pub active_loc: Option<LocObj>,
    /// Player's tile, or an unknown stand.
    pub stand: Option<Coord>,
    /// Protected player access at entry.
    pub protected: bool,
}
impl Entry {
    /// Start an operation or procedure with protected player access.
    pub fn new(script:ScriptId)->Self {Self {script,ints:Vec::new(),strings:Vec::new(),active_loc:None,stand:None,protected:true}}
}
/// Monotonic entry counters, unchanged by joins.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RunStats {
    /// Paths created, including paths later joined.
    pub paths: u64,
    /// Instructions dispatched over all paths.
    pub instructions: u64,
    /// Cumulative entry-owned allocation bytes, including freed snapshots.
    pub bytes: u64,
}
/// Evidence that the actual dispatcher applied the contract transfers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TransferAudit {
    /// Input cancellation transitions applied.
    pub cancellations: u64,
    /// Automatic suspended-script close transitions applied.
    pub automatic_closes: u64,
    /// Certain or possible suspension transfers applied.
    pub suspensions: u64,
    /// Stable-population loop summaries installed.
    pub loop_summaries: u64,
    /// Summaries whose allowance preceded every exit continuation.
    pub loop_allowance_before_exit: u64,
    /// Unsupported prefixes retained as results.
    pub unsupported_prefixes: u64,
}
/// Bounds for one entry. A limit failure discards all entry results.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Maximum paths ever created.
    pub paths: u64,
    /// Maximum total dispatched instructions.
    pub instructions: u64,
    /// Maximum cumulative allocation bytes.
    pub bytes: u64,
    /// Maximum forks at one instruction and call chain on one path.
    pub forks_per_site: u32,
}
impl Default for Limits { fn default()->Self {Self {paths:65_536,instructions:4_000_000,bytes:64*1024*1024,forks_per_site:64}} }
/// Failure of an entire entry, with no partial results.
#[derive(Debug, thiserror::Error)]
#[error("entry {entry:?}: {bound} limit {limit}, count {count}, at {at:?}")]
pub struct EntryFailure {
    /// Entry script.
    pub entry: ScriptId,
    /// Analysis bound name.
    pub bound: &'static str,
    /// Configured limit.
    pub limit: u64,
    /// Required count that crossed the limit.
    pub count: u64,
    /// Crossing instruction.
    pub at: Site,
}
/// All outcomes of an entry and the symbols used by them.
#[derive(Debug)]
pub struct Run {
    /// Every terminal outcome, including unsupported prefixes.
    pub paths: Vec<PathResult>,
    /// Entry-local expressions and observations.
    pub arena: Arena,
    /// Monotonic work and allocation counters.
    pub stats: RunStats,
    /// Applied transfer evidence.
    pub transfers: TransferAudit,
}
impl Run {
    /// Resolve a plain observation.
    pub fn atom_of(&self,value:Value)->Option<&Atom> {self.arena.atom_of(value)}
    /// Paths not proved disjoint from a proposed edge condition.
    pub fn rivals(&self,conds:&[Cond])->Vec<&PathResult> {self.paths.iter().filter(|p|!domains::disjoint(&self.arena,conds,&p.conds)).collect()}
    /// Whether every possible rival has the caller's required outcome.
    pub fn covered(&self,index:usize,same:impl Fn(&PathResult)->bool)->bool {
        self.paths.get(index).is_some_and(|p| self.paths.iter().filter(|q|!domains::disjoint(&self.arena,&p.conds,&q.conds)).all(same))
    }
    /// Remove usable conditions only while every rival retains the required outcome.
    pub fn generalise(&self,index:usize,same:impl Fn(&PathResult)->bool,order:impl Fn(&Cond)->u32)->Vec<Cond> {
        let Some(p)=self.paths.get(index) else {return Vec::new();};
        let mut conds:Vec<_>=p.conds.iter().filter(|c|c.class<=CondClass::Input).cloned().collect();
        let mut priority:Vec<_>=conds.iter().cloned().enumerate().collect();
        priority.sort_by_key(|(i,c)|(order(c),std::cmp::Reverse(*i)));
        for (_,candidate) in priority {
            let Some(i)=conds.iter().position(|c|*c==candidate) else {continue;};
            let trial:Vec<_>=conds.iter().enumerate().filter(|(j,_)|*j!=i).map(|(_,c)|c.clone()).collect();
            if self.paths.iter().filter(|q|!domains::disjoint(&self.arena,&trial,&q.conds)).all(&same) {conds.remove(i);}
        }
        conds
    }
}
/// A program and world bound for offline execution.
pub struct Machine<'a> {
    program: &'a Program,
    profile: EngineProfile,
    world: &'a dyn World,
}
impl<'a> Machine<'a> {
    /// Bind immutable inputs. Construction performs no world copies.
    pub fn new(program:&'a Program,profile:EngineProfile,world:&'a dyn World)->Self {Self {program,profile,world}}
    /// Execute all supported paths, or fail the entire entry at a monotonic cap.
    pub fn run(&self,entry:Entry,view:View,limits:Limits)->Result<Run,EntryFailure> {execute::run(self,entry,view,limits)}
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Frame {script:ScriptId,pc:i32,ints:Vec<Value>,strings:Vec<Text>,called:Option<Site>}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IterAge {Fresh,MaybeStale,Stale}
#[derive(Clone, Debug, PartialEq, Eq)]
struct LocIter {zone:ZoneKey,stamp:u32,items:Vec<LocObj>,next:usize,age:IterAge}
#[derive(Clone, Debug, PartialEq, Eq)]
struct NpcIter {site:Site,age:IterAge,population_epoch:u32,serial:u32}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Layout {generation:u32,lines_per_page:Option<i32>,pages:Option<Value>}
#[derive(Clone, Debug, PartialEq, Eq)]
struct State {
    frames:Vec<Frame>,ints:Vec<Value>,strings:Vec<Text>,player:PlayerPos,protected:bool,
    active_loc:[Option<LocObj>;2],active_npc:[Option<Value>;2],loc_iter:Option<LocIter>,npc_iter:Option<NpcIter>,
    vars:Vec<(VarpId,Stored)>,world:WorldOverlay,modal:Modal,layout:Layout,ticks:Ticks,epoch:u32,suspended:Suspension,
    ops:OpCount,conds:Vec<Cond>,domains:domains::Domains,effects:Vec<EffectRecord>,trail:Vec<(u32,u16)>,
    reads:Vec<(Site,u32)>,forks:Vec<(Vec<Site>,u32)>,world_reads_after_suspend:u32,population_epoch:u32,
    loops:Vec<LoopVisit>,at:Site,
    forced_end:Option<End>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct LoopVisit {site:Site,serial:u32,baseline:Box<State>,cost:OpCount,slot:usize}
struct Choice {arity:u16,depth:usize,conds:Vec<Cond>,domains:domains::Domains,trail:Vec<(u32,u16)>}
const ENGINE_OP_LIMIT:u32=500_000;
