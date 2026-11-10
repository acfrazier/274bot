use std::collections::BTreeMap;
use rs2vm::{Coord,ScriptId,VarpId};
use rs2vm::machine::{Effect,End,Entry,Limits,Machine,PlayerPos,Run,View};
use rs2vm::profile::EngineProfile;
use rs2vm::program::{Instruction,Operand,Program,Script,Signature,Varp};
use rs2vm::world::{Alloc,PlacedId,PlacedLoc,Tri,World,ZoneKey};

const DEST:Coord=Coord::new(3201,0,3200);
struct EmptyWorld;
impl World for EmptyWorld {
    fn zone_locs(&self,_:ZoneKey)->&[PlacedLoc] {&[]}
    fn placed_loc(&self,_:PlacedId)->Option<PlacedLoc> {None}
    fn zone_alloc(&self,_:Coord)->Alloc {Alloc {on_members:Tri::Yes,on_free:Tri::Yes}}
    fn npc_spare_capacity(&self)->Tri {Tri::Yes}
}
struct Asm {program:Program,code:Vec<Instruction>,labels:BTreeMap<&'static str,usize>,fixups:Vec<(usize,&'static str)>}
impl Asm {
    fn new()->Self {Self {program:Program::default(),code:Vec::new(),labels:BTreeMap::new(),fixups:Vec::new()}}
    fn op(&mut self,name:&str,operand:i32) {let opcode=self.program.opcodes.iter().find(|(_,n)|n.as_ref()==name).map(|(id,_)|*id).unwrap_or_else(||{let id=self.program.opcodes.len() as u16;self.program.opcodes.insert(id,name.into());id});self.code.push(Instruction {opcode,operand:Operand::Int(operand)});}
    fn label(&mut self,name:&'static str) {assert!(self.labels.insert(name,self.code.len()).is_none());}
    fn branch(&mut self,name:&str,target:&'static str) {self.fixups.push((self.code.len(),target));self.op(name,0);}
    fn push(&mut self,n:i32) {self.op("PUSH_CONSTANT_INT",n);}
    fn search(&mut self) {self.push(Coord::new(3200,0,3200).0);self.push(1);self.push(5);self.push(0);self.op("NPC_FINDALL",0);}
    fn finish(mut self,locals:u16)->Program {for (pc,target) in self.fixups {self.code[pc].operand=Operand::Int(self.labels[target] as i32-pc as i32-1);}self.program.scripts.insert(ScriptId(0),Script {id:ScriptId(0),int_locals:locals,code:self.code,..Script::default()});self.program.signatures.insert("cam_reset".into(),Signature::default());self.program.packs.varps.insert(VarpId(0),Varp {string:false,protect:true,transmit:true,name:"counter".into()});self.program}
}
fn loop_program(body:impl FnOnce(&mut Asm),after:impl FnOnce(&mut Asm))->Program {
    let mut a=Asm::new();a.search();a.label("step");a.op("NPC_FINDNEXT",0);a.push(0);a.branch("BRANCH_EQUALS","exit");body(&mut a);a.branch("BRANCH","step");a.label("exit");after(&mut a);a.push(DEST.0);a.op("P_TELEPORT",0);a.op("RETURN",0);a.finish(0)
}
fn mode(a:&mut Asm) {a.push(0);a.op("NPC_SETMODE",0);}
fn run(program:&Program,view:View)->Run {let world=EmptyWorld;Machine::new(program,EngineProfile::R289,&world).run(Entry::new(ScriptId(0)),view,Limits {forks_per_site:8,..Limits::default()}).expect("bounded synthetic loop entry")}
fn landing(path:&rs2vm::machine::PathResult)->bool {matches!(path.end,End::Finished)&&path.player==PlayerPos::Tile(DEST)}

#[test]
fn stable_population_summary_keeps_the_full_pass_allowance() {
    let p=loop_program(mode,|_|{});let r=run(&p,View::NAV);
    assert_eq!(r.paths.len(),1,"both empty and nonempty iterations have the same final outcome");
    assert!(landing(&r.paths[0]));assert!(r.covered(0,landing));
    // push 0, compare, push mode, setmode, backward branch, next step: six instructions.
    assert_eq!(r.paths[0].ops.max-r.paths[0].ops.min,6*EngineProfile::R289.npc_capacity);
    assert_eq!(r.transfers.loop_summaries,1);assert_eq!(r.transfers.loop_allowance_before_exit,1);
}

#[test]
fn long_live_loop_is_undecided_before_any_exit_continuation() {
    let p=loop_program(|a|{for _ in 0..40 {a.op("CAM_RESET",0);}},|_|{});let r=run(&p,View::NAV);
    assert!(r.transfers.loop_allowance_before_exit>0);
    assert!(r.paths.iter().all(|p|matches!(&p.end,End::Unsupported(f) if f.reason.as_ref()=="InstructionLimitUndecided"&&f.at.script==ScriptId(0)&&f.at.pc==6)));
    assert!(r.paths.iter().all(|p|!p.effects.iter().any(|e|matches!(e.effect,Effect::Move{..}))));
    assert!(!r.paths.iter().enumerate().any(|(i,p)|landing(p)&&r.covered(i,landing)));
}

#[test]
fn incrementing_compared_state_hits_the_fork_bound_not_a_summary() {
    let p=loop_program(|a|{a.op("PUSH_VARP",0);a.push(1);a.op("ADD",0);a.op("POP_VARP",0);},|_|{});let r=run(&p,View::NAV);
    assert_eq!(r.transfers.loop_summaries,0);
    assert!(r.paths.iter().any(|p|matches!(&p.end,End::BoundReached(f) if f.at.script==ScriptId(0)&&f.at.pc==5)));
    assert!(!r.paths.iter().enumerate().any(|(i,p)|landing(p)&&r.covered(i,landing)));
}

#[test]
fn selected_npc_effects_prevent_a_summary_under_all_view() {
    let p=loop_program(mode,|_|{});let r=run(&p,View::ALL);
    assert_eq!(r.transfers.loop_summaries,0);assert!(r.paths.iter().any(|p|matches!(&p.end,End::BoundReached(f) if f.at.script==ScriptId(0)&&f.at.pc==5)));
}

#[test]
fn population_mutation_does_not_use_the_slot_count_as_a_pass_bound() {
    let p=loop_program(|a|a.op("NPC_DEL",0),|_|{});let r=run(&p,View::NAV);
    assert_eq!(r.transfers.loop_summaries,0);assert_eq!(r.transfers.loop_allowance_before_exit,0);
    assert!(r.paths.iter().any(|p|matches!(&p.end,End::BoundReached(f) if f.at.script==ScriptId(0)&&f.at.pc==5)));
}

#[test]
fn resetting_the_iterator_does_not_close_a_pass() {
    let p=loop_program(|a|a.search(),|_|{});let r=run(&p,View::NAV);
    assert_eq!(r.transfers.loop_summaries,0);assert_eq!(r.transfers.loop_allowance_before_exit,0);
    assert!(r.paths.iter().any(|p|matches!(&p.end,End::BoundReached(f) if f.at.script==ScriptId(0)&&f.at.pc==5)));
}

#[test]
fn suspension_refuses_the_fresh_iterator_summary_and_keeps_the_error_rival() {
    let p=loop_program(|a|{a.push(0);a.op("P_DELAY",0);},|_|{});let r=run(&p,View::NAV);
    assert_eq!(r.transfers.loop_summaries,0);assert_eq!(r.transfers.loop_allowance_before_exit,0);
    assert!(r.paths.iter().any(|p|matches!(&p.end,End::EngineError(f) if f.at.script==ScriptId(0)&&f.at.pc==5)));
    assert!(!r.paths.iter().enumerate().any(|(i,p)|landing(p)&&r.covered(i,landing)));
}

#[test]
fn empty_and_nonempty_exits_keep_their_active_npc_distinction() {
    let p=loop_program(mode,|a|{a.op("NPC_TYPE",0);a.op("POP_INT_DISCARD",0);});let r=run(&p,View::NAV);
    assert_eq!(r.transfers.loop_summaries,1);
    assert!(r.paths.iter().any(|p|matches!(&p.end,End::EngineError(f) if f.reason.as_ref()=="no active NPC")));
    assert!(r.paths.iter().any(landing));
    assert!(!r.paths.iter().enumerate().any(|(i,p)|landing(p)&&r.covered(i,landing)),"the initial empty exit can throw before the teleport");
}

#[test]
fn a_called_body_has_no_local_closed_pass_allowance() {
    let mut a=Asm::new();
    a.search();a.label("step");a.op("NPC_FINDNEXT",0);a.push(0);a.branch("BRANCH_EQUALS","exit");
    a.op("GOSUB_WITH_PARAMS",1);a.branch("BRANCH","step");a.label("exit");
    a.push(DEST.0);a.op("P_TELEPORT",0);a.op("RETURN",0);
    let callee_start=a.code.len();mode(&mut a);a.op("RETURN",0);
    let callee=a.code.split_off(callee_start);
    let mut p=a.finish(0);
    p.scripts.insert(ScriptId(1),Script {id:ScriptId(1),code:callee,..Script::default()});
    let r=run(&p,View::NAV);
    assert_eq!(r.transfers.loop_summaries,0);assert_eq!(r.transfers.loop_allowance_before_exit,0);
    assert!(r.paths.iter().any(|p|matches!(&p.end,End::BoundReached(f) if f.at.script==ScriptId(0)&&f.at.pc==5)));
    assert!(!r.paths.iter().enumerate().any(|(i,p)|landing(p)&&r.covered(i,landing)));
}

#[test]
fn a_second_cycle_has_no_acyclic_pass_allowance() {
    let mut a=Asm::new();
    a.search();a.label("step");a.op("NPC_FINDNEXT",0);a.push(0);a.branch("BRANCH_EQUALS","exit");
    a.push(0);a.op("POP_INT_LOCAL",0);a.label("inner");
    a.op("PUSH_INT_LOCAL",0);a.push(1);a.op("ADD",0);a.op("POP_INT_LOCAL",0);
    a.op("PUSH_INT_LOCAL",0);a.push(2);a.branch("BRANCH_LESS_THAN","inner");
    a.branch("BRANCH","step");a.label("exit");a.push(DEST.0);a.op("P_TELEPORT",0);a.op("RETURN",0);
    let p=a.finish(1);let r=run(&p,View::NAV);
    assert_eq!(r.transfers.loop_summaries,0);assert_eq!(r.transfers.loop_allowance_before_exit,0);
    assert!(r.paths.iter().any(|p|matches!(&p.end,End::BoundReached(f) if f.at.script==ScriptId(0)&&f.at.pc==5)));
    assert!(!r.paths.iter().enumerate().any(|(i,p)|landing(p)&&r.covered(i,landing)));
}
