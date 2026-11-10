use super::*;
use crate::program::Operand;
pub(super) use crate::world::buffer_growth as growth;

pub(super) enum Step {Next,Returned,End(End),Split(Vec<State>)}
pub(super) enum Stop {Path(End),Entry(EntryFailure)}
impl From<End> for Stop {fn from(end:End)->Self {Self::Path(end)}}
impl From<EntryFailure> for Stop {fn from(e:EntryFailure)->Self {Self::Entry(e)}}
pub(super) type ExecResult<T> = Result<T,Stop>;
pub(super) struct Exec<'a,'b> {
    pub machine:&'a Machine<'b>,pub view:View,pub limits:Limits,pub entry:ScriptId,
    pub arena:Arena,pub stats:RunStats,pub audit:TransferAudit,choices:Vec<Option<Choice>>,
}
pub(super) fn unsupported(site:Site,reason:&'static str)->End {End::Unsupported(Fault {reason:reason.into(),at:site})}
pub(super) fn engine(site:Site,reason:&'static str)->End {End::EngineError(Fault {reason:reason.into(),at:site})}
fn result(st:State,end:End)->PathResult {PathResult {end,at:st.at,chain:st.frames.iter().filter_map(|f|f.called).collect(),conds:st.conds,effects:st.effects,player:st.player,ticks:st.ticks,ops:st.ops,world_reads_after_suspend:st.world_reads_after_suspend,modal:st.modal,ints:st.ints,strings:st.strings,trail:st.trail}}
pub(super) fn run(machine:&Machine<'_>,entry:Entry,view:View,limits:Limits)->Result<Run,EntryFailure> {
    let site=Site {script:entry.script,pc:0};
    let mut e=Exec {machine,view,limits,entry:entry.script,arena:Arena::default(),stats:RunStats {paths:1,..RunStats::default()},audit:TransferAudit::default(),choices:Vec::new()};
    if limits.paths<1 {return Err(e.failure("paths",limits.paths,1,site));}
    let player=entry.stand.map_or(PlayerPos::Rel {dx:0,dlevel:0,dz:0},PlayerPos::Tile);
    let state=match e.initial(entry,site) {
        Ok(state)=>state,
        Err(Stop::Entry(failure))=>return Err(failure),
        Err(Stop::Path(end))=>{
            if matches!(end,End::Unsupported(_)) {e.audit.unsupported_prefixes+=1;}
            e.charge(std::mem::size_of::<PathResult>(),site)?;
            return Ok(Run {paths:vec![PathResult {end,at:site,chain:Vec::new(),conds:Vec::new(),effects:Vec::new(),player,ticks:Ticks::default(),ops:OpCount::default(),world_reads_after_suspend:0,modal:Modal::default(),ints:Vec::new(),strings:Vec::new(),trail:Vec::new()}],arena:e.arena,stats:e.stats,transfers:e.audit});
        }
    };
    e.charge(std::mem::size_of::<State>(),site)?;
    let mut pending=vec![state];let mut ended=Vec::new();
    while !pending.is_empty() {
        let mut parked=Vec::new();
        while let Some(mut st)=pending.pop() {
            loop {
                match e.dispatch(&mut st) {
                    Ok(Step::Next)=>{},
                    Ok(Step::Returned)=>{e.reserve(&mut parked,1,st.at)?;parked.push(st);break;},
                    Ok(Step::End(end))|Err(Stop::Path(end))=>{
                        if matches!(end,End::Unsupported(_)) {e.audit.unsupported_prefixes+=1;}
                        e.reserve(&mut ended,1,st.at)?;
                        e.charge(st.frames.iter().filter(|f|f.called.is_some()).count()*std::mem::size_of::<Site>(),st.at)?;
                        ended.push(result(st,end));break;
                    }
                    Ok(Step::Split(mut states))=>{e.reserve(&mut pending,states.len(),st.at)?;pending.append(&mut states);break;},
                    Err(Stop::Entry(failure))=>return Err(failure),
                }
            }
        }
        e.erase(&mut parked,&mut ended);
        pending=parked;
    }
    e.erase(&mut Vec::new(),&mut ended);
    Ok(Run {paths:ended,arena:e.arena,stats:e.stats,transfers:e.audit})
}
impl Exec<'_, '_> {
    pub fn failure(&self,bound:&'static str,limit:u64,count:u64,at:Site)->EntryFailure {EntryFailure {entry:self.entry,bound,limit,count,at}}
    pub fn charge(&mut self,bytes:usize,site:Site)->Result<(),EntryFailure> {
        let count=self.stats.bytes.saturating_add(bytes as u64);
        if count>self.limits.bytes {return Err(self.failure("bytes",self.limits.bytes,count,site));}
        self.stats.bytes=count;Ok(())
    }
    pub(super) fn reserve<T>(&mut self,values:&mut Vec<T>,additional:usize,at:Site)->Result<(),EntryFailure> {
        if let Some((additional,bytes))=growth(values,additional) {self.charge(bytes,at)?;values.reserve_exact(additional);}Ok(())
    }
    pub(super) fn overlay<T>(&mut self,st:&mut State,operation:impl FnOnce(&mut WorldOverlay,&dyn World,&mut crate::world::ByteBudget)->Result<T,crate::world::WorldError>)->ExecResult<T> {
        let mut budget=crate::world::ByteBudget {limit:self.limits.bytes,used:self.stats.bytes};
        let value=operation(&mut st.world,self.machine.world,&mut budget);
        self.stats.bytes=budget.used;
        match value {
            Ok(value)=>Ok(value),
            Err(crate::world::WorldError::ByteLimit {limit,count})=>Err(self.failure("bytes",limit,count,st.at).into()),
            Err(error)=>Err(super::transfers::world_end(st.at,error).into()),
        }
    }
    pub fn copy(&mut self,st:&State)->Result<State,EntryFailure> {
        let upper=self.stats.bytes.saturating_add(st.owned_bytes() as u64);
        if upper>self.limits.bytes {return Err(self.failure("bytes",self.limits.bytes,upper,st.at));}
        let copied=st.clone();self.stats.bytes=self.stats.bytes.saturating_add(copied.owned_bytes() as u64);Ok(copied)
    }
    pub(super) fn copy_without_loops(&mut self,st:&mut State)->Result<State,EntryFailure> {
        let loops=std::mem::take(&mut st.loops);let copied=self.copy(st);st.loops=loops;copied
    }
    fn copy_with_domains(&mut self,st:&mut State,domains:domains::Domains)->Result<State,EntryFailure> {
        let old=std::mem::take(&mut st.domains);
        let copied=self.copy(st);st.domains=old;
        copied.map(|mut child|{child.domains=domains;child})
    }
    fn initial(&mut self,entry:Entry,site:Site)->ExecResult<State> {
        let Some(script)=self.machine.program.script(entry.script) else {return Err(engine(site,"entry script does not exist").into());};
        self.charge(usize::from(script.int_locals)*std::mem::size_of::<Value>()+usize::from(script.string_locals)*std::mem::size_of::<Text>()+std::mem::size_of::<Frame>(),site)?;
        let mut ints=vec![Value::Known(0);script.int_locals as usize];
        for n in 0..usize::from(script.int_args) {let value=if let Some(v)=entry.ints.get(n) {*v} else {let index=u8::try_from(n).map_err(|_|unsupported(site,"entry argument index exceeds the bytecode header"))?;self.atom(AtomKind::Arg(index),0,0,site,CondClass::Input,(i32::MIN,i32::MAX))?};let Some(local)=ints.get_mut(n) else {return Err(engine(site,"argument exceeds local count").into());};*local=value;}
        let mut strings=vec![Text::Lit("".into());script.string_locals as usize];
        let mut provided=entry.strings.into_iter();
        for n in 0..usize::from(script.string_args) {let value=provided.next().unwrap_or(Text::Unknown(site));self.charge(value.bytes(),site)?;let Some(local)=strings.get_mut(n) else {return Err(engine(site,"string argument exceeds local count").into());};*local=value;}
        Ok(State {frames:vec![Frame {script:entry.script,pc:-1,ints,strings,called:None}],ints:Vec::new(),strings:Vec::new(),player:entry.stand.map_or(PlayerPos::Rel {dx:0,dlevel:0,dz:0},PlayerPos::Tile),protected:entry.protected,active_loc:[entry.active_loc,None],active_npc:[None,None],loc_iter:None,npc_iter:None,vars:Vec::new(),world:WorldOverlay::with_profile(self.machine.profile),modal:Modal::default(),layout:Layout::default(),ticks:Ticks::default(),epoch:0,suspended:Suspension::No,ops:OpCount::default(),conds:Vec::new(),domains:domains::Domains::default(),effects:Vec::new(),trail:Vec::new(),reads:Vec::new(),forks:Vec::new(),world_reads_after_suspend:0,population_epoch:0,loops:Vec::new(),at:site,forced_end:None})
    }
    pub fn expr(&mut self,expr:Expr,site:Site)->Result<Value,EntryFailure> {
        if let Some(i)=self.arena.exprs.iter().position(|e|*e==expr) {return Ok(Value::Sym(ExprId(i as u32)));}
        if let Some((additional,bytes))=growth(&self.arena.exprs,1) {self.charge(bytes,site)?;self.arena.exprs.reserve_exact(additional);}
        let id=ExprId(self.arena.exprs.len() as u32);self.arena.exprs.push(expr);Ok(Value::Sym(id))
    }
    pub fn atom(&mut self,kind:AtomKind,epoch:u32,nth:u32,site:Site,class:CondClass,bounds:(i32,i32))->Result<Value,EntryFailure> {
        let atom=Atom {kind,epoch,nth,site,class,bounds};
        let id=if let Some(i)=self.arena.atoms.iter().position(|a|*a==atom) {AtomId(i as u32)} else {
            if let Some((additional,bytes))=growth(&self.arena.atoms,1) {self.charge(bytes,site)?;self.arena.atoms.reserve_exact(additional);}
            let id=AtomId(self.arena.atoms.len() as u32);self.arena.atoms.push(atom);id
        };
        self.expr(Expr::Atom(id),site)
    }
    pub fn observe(&mut self,st:&mut State,kind:AtomKind,class:CondClass,bounds:(i32,i32),fresh:bool)->Result<Value,EntryFailure> {
        let nth=if fresh {if let Some((_,n))=st.reads.iter_mut().find(|(s,_)|*s==st.at) {*n+=1;*n} else {self.reserve(&mut st.reads,1,st.at)?;st.reads.push((st.at,0));0}} else {0};
        let site=if fresh {st.at} else {Site::default()};
        let stable=matches!(kind,AtomKind::Members|AtomKind::Uid)||matches!(kind,AtomKind::Varp(id) if self.machine.program.packs.varps.get(&id).is_some_and(|v|v.protect));
        self.atom(kind,if stable {0}else{st.epoch},nth,site,class,bounds)
    }
    pub fn push(&mut self,st:&mut State,value:Value)->ExecResult<()> {self.reserve(&mut st.ints,1,st.at)?;st.ints.push(value);Ok(())}
    pub fn push_text(&mut self,st:&mut State,value:Text)->ExecResult<()> {self.reserve(&mut st.strings,1,st.at)?;st.strings.push(value);Ok(())}
    pub fn effect(&mut self,st:&mut State,effect:Effect)->ExecResult<()> {self.reserve(&mut st.effects,1,st.at)?;st.effects.push(EffectRecord {effect,site:st.at,ticks:st.ticks});Ok(())}
    pub(super) fn clone_text(&mut self,text:&Text,at:Site)->Result<Text,EntryFailure> {self.charge(text.bytes(),at)?;Ok(text.clone())}
    fn narrow(&mut self,st:&State,pred:&Pred,truth:bool)->ExecResult<Option<domains::Domains>> {
        self.charge(st.domains.clone_bytes(),st.at)?;
        let mut narrowed=st.domains.clone();
        let mut budget=crate::world::ByteBudget {limit:self.limits.bytes,used:self.stats.bytes};
        let outcome=domains::assume_bounded(&self.arena,&mut narrowed,pred,truth,&mut budget);
        self.stats.bytes=budget.used;
        match outcome {
            Ok(true)=>Ok(Some(narrowed)),Ok(false)=>Ok(None),
            Err(crate::world::WorldError::ByteLimit {limit,count})=>Err(self.failure("bytes",limit,count,st.at).into()),
            Err(error)=>Err(super::transfers::world_end(st.at,error).into()),
        }
    }
    pub fn split(&mut self,st:&mut State,pred:Pred)->ExecResult<Vec<State>> {
        let mut arms=[None,None];let mut len=0;
        for truth in [false,true] {
            if domains::recorded(&st.conds,&pred).is_some_and(|known|known!=truth) {continue;}
            if let Some(narrowed)=self.narrow(st,&pred,truth)? {arms[len]=Some((truth,narrowed));len+=1;}
        }
        let choice=if len>1 {Some(self.new_choice(st,len)?)}else{None};
        self.charge(len*std::mem::size_of::<State>(),st.at)?;
        let mut states=Vec::with_capacity(len);
        for (n,(truth,narrowed)) in arms.into_iter().flatten().enumerate() {
            let mut child=self.copy_with_domains(st,narrowed)?;
            if let Some((id,chain))=&choice {
                self.charge(pred.bytes(),st.at)?;
                self.mark_choice(&mut child,pred.clone(),truth,*id,n as u16,chain)?;
            }
            self.push(&mut child,Value::Known(i32::from(truth)))?;states.push(child);
        }
        Ok(states)
    }
    fn new_choice(&mut self,st:&State,arity:usize)->ExecResult<(u32,Vec<Site>)> {
        let count=self.stats.paths.saturating_add(arity as u64-1);
        if count>self.limits.paths {return Err(self.failure("paths",self.limits.paths,count,st.at).into());}
        if arity>usize::from(u16::MAX) {return Err(unsupported(st.at,"choice has too many successors").into());}
        let old=st.forks.iter().find(|(c,_)|c.iter().copied().eq(st.frames.iter().filter_map(|f|f.called).chain(std::iter::once(st.at)))).map_or(0,|(_,n)|*n);
        if old>=self.limits.forks_per_site {return Err(End::BoundReached(Fault {reason:"fork-site limit".into(),at:st.at}).into());}
        let chain_len=st.frames.iter().filter(|f|f.called.is_some()).count()+1;
        self.charge(chain_len*std::mem::size_of::<Site>(),st.at)?;
        let mut chain=Vec::with_capacity(chain_len);
        chain.extend(st.frames.iter().filter_map(|f|f.called));chain.push(st.at);
        let growth=growth(&self.choices,1);
        self.charge(growth.map_or(0,|(_,bytes)|bytes)+st.conds.len()*std::mem::size_of::<Cond>()+st.conds.iter().map(|c|c.pred.bytes()).sum::<usize>()+st.domains.clone_bytes()+st.trail.len()*std::mem::size_of::<(u32,u16)>(),st.at)?;
        self.stats.paths=count;let id=self.choices.len() as u32;
        if let Some((additional,_))=growth {self.choices.reserve_exact(additional);}
        self.choices.push(Some(Choice {arity:arity as u16,depth:st.frames.len(),conds:st.conds.clone(),domains:st.domains.clone(),trail:st.trail.clone()}));
        Ok((id,chain))
    }
    fn mark_choice(&mut self,child:&mut State,pred:Pred,truth:bool,id:u32,n:u16,chain:&[Site])->ExecResult<()> {
        self.reserve(&mut child.conds,1,child.at)?;self.reserve(&mut child.trail,1,child.at)?;
        child.conds.push(Cond {class:domains::pred_class(&self.arena,&pred),pred,truth,site:child.at,choice:id});child.trail.push((id,n));
        if let Some((_,count))=child.forks.iter_mut().find(|(c,_)|c.as_slice()==chain) {*count+=1;}
        else {self.reserve(&mut child.forks,1,child.at)?;self.charge(std::mem::size_of_val(chain),child.at)?;child.forks.push((chain.to_vec(),1));}
        Ok(())
    }
    fn dispatch(&mut self,st:&mut State)->ExecResult<Step> {
        if let Some(end)=st.forced_end.take() {return Ok(Step::End(end));}
        let Some(frame)=st.frames.last_mut() else {return Ok(Step::End(End::Finished));};
        let next=frame.pc.checked_add(1).ok_or_else(||engine(st.at,"instruction index overflow"))?;
        st.at=Site {script:frame.script,pc:next as u32};
        if st.ops.min>ENGINE_OP_LIMIT {return Ok(Step::End(engine(st.at,"engine instruction limit")));}
        if st.ops.max>ENGINE_OP_LIMIT {return Ok(Step::End(unsupported(st.at,"InstructionLimitUndecided")));}
        let count=self.stats.instructions+1;if count>self.limits.instructions {return Err(self.failure("instructions",self.limits.instructions,count,st.at).into());}
        self.stats.instructions=count;st.ops.min+=1;st.ops.max+=1;
        let script=self.machine.program.script(frame.script).ok_or_else(||engine(st.at,"script missing"))?;
        let instruction=script.code.get(next as usize).ok_or_else(||engine(st.at,"instruction outside script"))?;
        frame.pc=next;
        let name=self.machine.program.opcode(instruction.opcode).ok_or_else(||unsupported(st.at,"unnamed opcode"))?;
        let operand=match &instruction.operand {Operand::Int(i)=>*i,Operand::Text(_)=>0};
        match name {
            "PUSH_CONSTANT_INT"=>self.push(st,Value::Known(operand))?,
            "PUSH_CONSTANT_STRING"=>{let Operand::Text(text)=&instruction.operand else {return Err(engine(st.at,"string operand missing").into());};self.charge(text.len(),st.at)?;self.push_text(st,Text::Lit(text.clone()))?;}
            "PUSH_INT_LOCAL"=>{let v=st.frames.last().and_then(|f|f.ints.get(operand as usize)).copied().ok_or_else(||unsupported(st.at,"int local outside frame"))?;self.push(st,v)?;}
            "POP_INT_LOCAL"=>{let v=pop(st)?;let at=st.at;let dst=st.frames.last_mut().and_then(|f|f.ints.get_mut(operand as usize)).ok_or_else(||unsupported(at,"int local outside frame"))?;*dst=v;}
            "PUSH_STRING_LOCAL"=>{let v=st.frames.last().and_then(|f|f.strings.get(operand as usize)).ok_or_else(||unsupported(st.at,"string local outside frame"))?;let v=self.clone_text(v,st.at)?;self.push_text(st,v)?;}
            "POP_STRING_LOCAL"=>{let v=pop_text(st)?;let at=st.at;let dst=st.frames.last_mut().and_then(|f|f.strings.get_mut(operand as usize)).ok_or_else(||unsupported(at,"string local outside frame"))?;*dst=v;}
            "POP_INT_DISCARD"=>{pop(st)?;}
            "POP_STRING_DISCARD"=>{pop_text(st)?;}
            "BRANCH"=>jump(st,operand)?,
            "BRANCH_NOT"|"BRANCH_EQUALS"|"BRANCH_LESS_THAN"|"BRANCH_GREATER_THAN"|"BRANCH_LESS_THAN_OR_EQUALS"|"BRANCH_GREATER_THAN_OR_EQUALS"=>{
                let rhs=pop(st)?;let lhs=pop(st)?;let op=match name {"BRANCH_NOT"=>CmpOp::Ne,"BRANCH_EQUALS"=>CmpOp::Eq,"BRANCH_LESS_THAN"=>CmpOp::Lt,"BRANCH_GREATER_THAN"=>CmpOp::Gt,"BRANCH_LESS_THAN_OR_EQUALS"=>CmpOp::Le,_=>CmpOp::Ge};
                if let (Some(a),Some(b))=(known(st,&self.arena,lhs),known(st,&self.arena,rhs)) {if op.test(a,b) {jump(st,operand)?;}return Ok(Step::Next);}
                let pred=Pred::Cmp {lhs,op,rhs};
                if let Some(truth)=domains::recorded(&st.conds,&pred) {if truth {jump(st,operand)?;}return Ok(Step::Next);}
                let mut children=self.split(st,pred)?;
                for child in &mut children {if pop(child)?==Value::Known(1) {jump(child,operand)?;}}
                return Ok(Step::Split(children));
            }
            "SWITCH"=>return self.switch(st,operand),
            "RETURN"=>{
                if st.frames.len()>1 {st.frames.pop();return Ok(Step::Returned);}
                self.finish(st)?;return Ok(Step::End(End::Finished));
            }
            "GOSUB_WITH_PARAMS"|"JUMP_WITH_PARAMS"|"GOSUB"|"JUMP"=>{
                let target=if name=="GOSUB"||name=="JUMP" {let v=pop(st)?;known(st,&self.arena,v).ok_or_else(||unsupported(st.at,"unknown stack call target"))?} else {operand};
                self.call(st,ScriptId(target as u32),name.starts_with("JUMP"))?;
            }
            "JOIN_STRING"=>{
                if operand<0||operand as usize>st.strings.len() {return Err(unsupported(st.at,"string stack underflow").into());}
                let start=st.strings.len()-operand as usize;
                let parts=&st.strings[start..];
                let text=if parts.iter().all(|p|matches!(p,Text::Lit(_))) {
                    let len=parts.iter().map(Text::bytes).sum();self.charge(len,st.at)?;
                    let mut joined=String::with_capacity(len);
                    for part in parts {if let Text::Lit(text)=part {joined.push_str(text);}}
                    Text::Lit(joined.into_boxed_str())
                } else {Text::Unknown(st.at)};
                st.strings.truncate(start);self.push_text(st,text)?;
            }
            "PUSH_VARP"|"POP_VARP"|"PUSH_VARBIT"|"POP_VARBIT"|"PUSH_VARN"|"PUSH_VARS"=>self.var(st,name,operand)?,
            "DEFINE_ARRAY"|"PUSH_ARRAY_INT"|"POP_ARRAY_INT"=>return Ok(Step::End(engine(st.at,"engine array opcode throws"))),
            other=>return self.command(st,other,operand),
        }
        Ok(Step::Next)
    }
    fn call(&mut self,st:&mut State,target:ScriptId,jump:bool)->ExecResult<()> {
        // CoreOps.ts:225-261; ScriptState.ts:334-376. fp counts callers.
        if !jump&&st.frames.len()>50 {return Err(engine(st.at,"engine frame limit").into());}
        let script=self.machine.program.script(target).ok_or_else(||engine(st.at,"call target missing"))?;
        if st.ints.len()<script.int_args as usize||st.strings.len()<script.string_args as usize {return Err(unsupported(st.at,"call argument stack underflow").into());}
        if script.int_args>script.int_locals||script.string_args>script.string_locals {return Err(engine(st.at,"callee argument exceeds locals").into());}
        self.charge(script.int_locals as usize*std::mem::size_of::<Value>()+script.string_locals as usize*std::mem::size_of::<Text>(),st.at)?;
        let mut ints=vec![Value::Known(0);script.int_locals as usize];
        let start=st.ints.len()-script.int_args as usize;
        ints[..script.int_args as usize].copy_from_slice(&st.ints[start..]);st.ints.truncate(start);
        let mut strings=vec![Text::Lit("".into());script.string_locals as usize];
        for index in (0..usize::from(script.string_args)).rev() {strings[index]=pop_text(st)?;}
        let called=if jump {None}else{Some(st.at)};if jump {st.frames.clear();}
        self.reserve(&mut st.frames,1,st.at)?;
        st.frames.push(Frame {script:target,pc:-1,ints,strings,called});Ok(())
    }
    fn switch(&mut self,st:&mut State,index:i32)->ExecResult<Step> {
        let value=pop(st)?;let Some(table)=self.machine.program.script(st.at.script).and_then(|s|s.switches.get(index as usize)) else {return Ok(Step::Next);};
        if let Some(value)=known(st,&self.arena,value) {if let Some(offset)=table.get(&value) {jump(st,*offset)?;}return Ok(Step::Next);}
        let count=table.values().filter(|offset|**offset!=0).count();
        if count==0 {return Ok(Step::Next);}
        self.charge(count*std::mem::size_of::<(i32,i32)>(),st.at)?;
        let mut targets=Vec::with_capacity(count);
        for (&key,&offset) in table {if offset!=0 {targets.push((offset,key));}}
        targets.sort_unstable();
        let groups=targets.iter().enumerate().filter(|(i,(offset,_))|*i==0||targets[*i-1].0!=*offset).count();
        self.charge(count*4+(groups+1)*std::mem::size_of::<(Pred,bool,i32)>(),st.at)?;
        let mut all=Vec::with_capacity(count);all.extend(targets.iter().map(|(_,key)|*key));
        let mut arms=Vec::with_capacity(groups+1);let mut start=0;
        while start<targets.len() {
            let offset=targets[start].0;
            let end=start+targets[start..].iter().take_while(|(other,_)|*other==offset).count();
            self.charge((end-start)*4,st.at)?;
            let mut keys=Vec::with_capacity(end-start);keys.extend(targets[start..end].iter().map(|(_,key)|*key));
            arms.push((Pred::In {of:value,set:keys.into_boxed_slice()},true,offset));start=end;
        }
        arms.push((Pred::In {of:value,set:all.into_boxed_slice()},false,0));
        self.charge(arms.len()*std::mem::size_of::<(Pred,bool,i32,domains::Domains)>(),st.at)?;
        let mut possible=Vec::with_capacity(arms.len());
        for (pred,truth,offset) in arms {
            if domains::recorded(&st.conds,&pred).is_some_and(|known|known!=truth) {continue;}
            if let Some(narrowed)=self.narrow(st,&pred,truth)? {possible.push((pred,truth,offset,narrowed));}
        }
        let choice=if possible.len()>1 {Some(self.new_choice(st,possible.len())?)}else{None};
        self.charge(possible.len()*std::mem::size_of::<State>(),st.at)?;
        let mut children=Vec::with_capacity(possible.len());
        for (n,(pred,truth,offset,narrowed)) in possible.into_iter().enumerate() {
            let mut child=self.copy_with_domains(st,narrowed)?;
            if let Some((id,chain))=&choice {self.mark_choice(&mut child,pred,truth,*id,n as u16,chain)?;}
            jump(&mut child,offset)?;children.push(child);
        }
        Ok(Step::Split(children))
    }
    fn erase(&mut self,live:&mut Vec<State>,ended:&mut Vec<PathResult>) {
        for id in (0..self.choices.len()).rev() {
            let Some(choice)=self.choices[id].as_ref() else {continue;};
            let has=|trail:&[(u32,u16)]|trail.iter().any(|(c,_)|*c==id as u32);
            let members=live.iter().filter(|s|has(&s.trail));
            let results=ended.iter().filter(|s|has(&s.trail));
            if members.clone().count()==usize::from(choice.arity)&&results.clone().next().is_none()
                &&members.clone().all(|s|s.frames.len()<choice.depth)
                &&unique_successors(members.clone().map(|s|&s.trail),id as u32,choice.arity) {
                let Some(first)=live.iter().position(|s|has(&s.trail)) else {continue;};
                if !members.clone().all(|s|state_eq(&live[first],s,self.view,None)) {continue;}
                let min=members.clone().map(|s|s.ops.min).min().unwrap_or(0);
                let max=members.map(|s|s.ops.max).max().unwrap_or(0);
                let Some(choice)=self.choices[id].take() else {continue;};
                let state=&mut live[first];
                state.conds=choice.conds;state.domains=choice.domains;state.trail=choice.trail;state.ops=OpCount {min,max};
                for i in (first+1..live.len()).rev() {if has(&live[i].trail) {live.remove(i);}}
            } else if members.clone().next().is_none()&&results.clone().count()==usize::from(choice.arity)
                &&unique_successors(results.clone().map(|s|&s.trail),id as u32,choice.arity) {
                let Some(first)=ended.iter().position(|s|has(&s.trail)) else {continue;};
                if !results.clone().all(|s|result_eq(&ended[first],s,self.view)) {continue;}
                let min=results.clone().map(|s|s.ops.min).min().unwrap_or(0);
                let max=results.map(|s|s.ops.max).max().unwrap_or(0);
                let Some(choice)=self.choices[id].take() else {continue;};
                let state=&mut ended[first];
                state.conds=choice.conds;state.trail=choice.trail;state.ops=OpCount {min,max};
                for i in (first+1..ended.len()).rev() {if has(&ended[i].trail) {ended.remove(i);}}
            }
        }
    }
}
fn unique_successors<'a>(trails:impl Iterator<Item=&'a Vec<(u32,u16)>>+Clone,id:u32,arity:u16)->bool {
    (0..arity).all(|n|trails.clone().filter(|trail|trail.iter().any(|(c,k)|*c==id&&*k==n)).count()==1)
}
pub(super) fn pop(st:&mut State)->Result<Value,End> {st.ints.pop().ok_or_else(||unsupported(st.at,"int stack underflow"))}
pub(super) fn pop_text(st:&mut State)->Result<Text,End> {st.strings.pop().ok_or_else(||unsupported(st.at,"string stack underflow"))}
// PROJANIM_MAP (ServerOps.ts:203-211) is the largest fixed command: nine integers.
pub(super) struct Args {values:[Value;9],len:usize}
impl std::ops::Deref for Args {type Target=[Value];fn deref(&self)->&Self::Target {&self.values[..self.len]}}
pub(super) fn pop_args(st:&mut State,n:usize)->Result<Args,End> {
    if n>9 {return Err(unsupported(st.at,"command integer arity exceeds the closed inventory"));}
    let start=st.ints.len().checked_sub(n).ok_or_else(||unsupported(st.at,"int stack underflow"))?;
    let mut args=Args {values:[Value::Known(0);9],len:n};args.values[..n].copy_from_slice(&st.ints[start..]);st.ints.truncate(start);Ok(args)
}
pub(super) fn pop_array<const N:usize>(st:&mut State)->Result<[Value;N],End> {
    let start=st.ints.len().checked_sub(N).ok_or_else(||unsupported(st.at,"int stack underflow"))?;
    let mut args=[Value::Known(0);N];args.copy_from_slice(&st.ints[start..]);st.ints.truncate(start);Ok(args)
}
pub(super) fn known(st:&State,arena:&Arena,value:Value)->Option<i32> {domains::known(arena,&st.domains,value)}
pub(super) fn jump(st:&mut State,offset:i32)->Result<(),End> {let at=st.at;let frame=st.frames.last_mut().ok_or_else(||engine(at,"branch without frame"))?;frame.pc=frame.pc.checked_add(offset).ok_or_else(||engine(at,"branch overflow"))?;Ok(())}
pub(super) fn selected_effects_equal(a:&[EffectRecord],b:&[EffectRecord],view:View)->bool {a.iter().filter(|r|view.holds(&r.effect)).map(|r|(&r.effect,r.ticks)).eq(b.iter().filter(|r|view.holds(&r.effect)).map(|r|(&r.effect,r.ticks)))}
pub(super) fn state_eq(a:&State,b:&State,view:View,loop_item:Option<usize>)->bool {
    a.frames==b.frames&&a.ints==b.ints&&a.strings==b.strings&&a.player==b.player&&a.protected==b.protected&&a.active_loc==b.active_loc&&a.active_npc.iter().zip(&b.active_npc).enumerate().all(|(slot,(a,b))|loop_item==Some(slot)||a==b)&&a.loc_iter==b.loc_iter&&a.npc_iter==b.npc_iter&&a.vars==b.vars&&a.world==b.world&&a.modal==b.modal&&a.layout==b.layout&&a.ticks==b.ticks&&a.epoch==b.epoch&&a.suspended==b.suspended&&a.population_epoch==b.population_epoch&&selected_effects_equal(&a.effects,&b.effects,view)
}
fn result_eq(a:&PathResult,b:&PathResult,view:View)->bool {matches!(a.end,End::Finished|End::AwaitingInput)&&a.end==b.end&&a.player==b.player&&a.modal==b.modal&&selected_effects_equal(&a.effects,&b.effects,view)}
impl State {
    fn owned_bytes(&self)->usize {
        self.frames.capacity()*std::mem::size_of::<Frame>()
            +self.frames.iter().map(|f|f.ints.capacity()*std::mem::size_of::<Value>()+f.strings.capacity()*std::mem::size_of::<Text>()+f.strings.iter().map(Text::bytes).sum::<usize>()).sum::<usize>()
            +self.ints.capacity()*std::mem::size_of::<Value>()
            +self.strings.capacity()*std::mem::size_of::<Text>()+self.strings.iter().map(Text::bytes).sum::<usize>()
            +self.vars.capacity()*std::mem::size_of::<(VarpId,Stored)>()+self.vars.iter().map(|(_,v)|match v {Stored::Str(t)=>t.bytes(),_=>0}).sum::<usize>()
            +self.conds.capacity()*std::mem::size_of::<Cond>()+self.conds.iter().map(|c|c.pred.bytes()).sum::<usize>()
            +self.effects.capacity()*std::mem::size_of::<EffectRecord>()+self.effects.iter().map(|e|e.effect.bytes()).sum::<usize>()
            +self.trail.capacity()*std::mem::size_of::<(u32,u16)>()
            +self.reads.capacity()*std::mem::size_of::<(Site,u32)>()
            +self.forks.capacity()*std::mem::size_of::<(Vec<Site>,u32)>()+self.forks.iter().map(|(c,_)|c.capacity()*std::mem::size_of::<Site>()).sum::<usize>()
            +self.domains.bytes()+self.world.owned_bytes()
            +self.loc_iter.as_ref().map_or(0,|i|i.items.capacity()*std::mem::size_of::<LocObj>())
            +self.loops.capacity()*std::mem::size_of::<LoopVisit>()
            +self.loops.iter().map(|l|std::mem::size_of::<State>()+l.baseline.owned_bytes()).sum::<usize>()
    }
}
impl Text {fn bytes(&self)->usize {match self {Self::Lit(s)=>s.len(),Self::Unknown(_)=>0}}}
impl Pred {fn bytes(&self)->usize {match self {Self::In {set,..}=>std::mem::size_of_val(set.as_ref()),_=>0}}}
impl Effect {fn bytes(&self)->usize {match self {Self::Message{text}=>text.bytes(),Self::VarWrite{value:Stored::Str(t),..}=>t.bytes(),_=>0}}}
