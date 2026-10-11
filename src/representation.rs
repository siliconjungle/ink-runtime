//! Transaction-boundary execution and representation migration. Candidates are
//! checked core witnesses; selection, costs and target execution stay external.
use ink_core::{
    definition::CheckedDefinition,
    logic::{Context, Term},
    representation::{CheckedMigration, CheckedRepresentation},
    snapshot_wire::{self, Limits, Logical},
    LangResult,
};
use serde_json::{json, Value};
mod codec;
/// Implementations of literal target execution are disclosed trusted backends.
/// They must execute the exact sealed definition, never select new algorithms.
pub trait Executor {
    /// Optional transport projection for a fixed source read-only query.
    /// The logical-state slot is omitted; source admission proves it unchanged.
    fn transition_prepared_readonly(&self,_checked:&CheckedRepresentation,_action:&str,_state:&dyn std::any::Any,_args:Vec<Term>)->LangResult<Option<(Term,Box<dyn std::any::Any>)>> {Ok(None)}
    /// Optional immutable target state. The runtime publishes it only after
    /// observation and transport validation, exactly like the ground fallback.
    fn prepare(&self, _checked: &CheckedRepresentation, _physical: &Term) -> LangResult<Option<Box<dyn std::any::Any>>> { Ok(None) }
    fn materialise(&self, _checked: &CheckedRepresentation, _state: &dyn std::any::Any) -> LangResult<Term> { Err("executor has no prepared state codec".into()) }
    fn transition_prepared(&self, _checked: &CheckedRepresentation, _action: &str, _state: &dyn std::any::Any, _args: Vec<Term>) -> LangResult<(Term, Box<dyn std::any::Any>)> { Err("executor has no prepared transition".into()) }
    fn execute(
        &self,
        definition: &CheckedDefinition,
        context: &Context,
        function: &str,
        args: Vec<Term>,
    ) -> LangResult<Term>;
    /// Execute once and project both outputs. Native backends may keep the
    /// intermediate in their typed ABI rather than serialising it twice.
    fn transition(
        &self,
        checked: &CheckedRepresentation,
        action: &str,
        args: Vec<Term>,
    ) -> LangResult<(Term, Term)> {
        let candidate = checked
            .plan()
            .actions
            .get(action)
            .ok_or("unknown representation action")?;
        let executed = self.execute(
            checked.definition(),
            checked.context(),
            &candidate.execute,
            args,
        )?;
        let observed = self.execute(
            checked.definition(),
            checked.context(),
            &candidate.observe,
            vec![executed.clone()],
        )?;
        let next = self.execute(
            checked.definition(),
            checked.context(),
            &candidate.next,
            vec![executed],
        )?;
        Ok((observed, next))
    }
}
pub struct Reference;
struct PreparedReference { machine: std::sync::Arc<crate::definition_machine::Machine>, physical: crate::definition_machine::Value }
impl Executor for Reference {
    fn execute(&self,d:&CheckedDefinition,_context:&Context,function:&str,args:Vec<Term>)->LangResult<Term>{
        let machine=crate::definition_machine::Machine::new(d)?;
        let inputs=args.iter().map(crate::definition_machine::Value::from_term).collect::<LangResult<Vec<_>>>()?;
        machine.invoke(function,inputs)?.term()
    }
    fn prepare(&self,c:&CheckedRepresentation,p:&Term)->LangResult<Option<Box<dyn std::any::Any>>>{
        let machine=std::sync::Arc::new(crate::definition_machine::Machine::new(c.definition())?);
        Ok(Some(Box::new(PreparedReference{machine,physical:crate::definition_machine::Value::from_term(p)?})))
    }
    fn materialise(&self,c:&CheckedRepresentation,p:&dyn std::any::Any)->LangResult<Term>{
        let p=p.downcast_ref::<PreparedReference>().ok_or("reference prepared state")?;
        let expected=ink_core::registry::identity(&("ink-sealed-executable-v1",c.definition().bundle_sha256(),c.definition().exports(),c.definition().choices()))?;
        if p.machine.identity!=expected{return Err("reference prepared identity".into());}p.physical.term()
    }
    fn transition_prepared(&self,c:&CheckedRepresentation,action:&str,p:&dyn std::any::Any,args:Vec<Term>)->LangResult<(Term,Box<dyn std::any::Any>)>{
        let p=p.downcast_ref::<PreparedReference>().ok_or("reference prepared state")?;
        let expected=ink_core::registry::identity(&("ink-sealed-executable-v1",c.definition().bundle_sha256(),c.definition().exports(),c.definition().choices()))?;
        if p.machine.identity!=expected{return Err("reference prepared identity".into());}
        let mut inputs=vec![p.physical.clone()];inputs.extend(args.iter().map(crate::definition_machine::Value::from_term).collect::<LangResult<Vec<_>>>()?);
        let a=c.plan().actions.get(action).ok_or("reference prepared action")?;
        let executed=p.machine.invoke_prepared(&a.execute,inputs)?;
        let observed=p.machine.invoke_prepared(&a.observe,vec![executed.clone()])?.public_observation()?;
        let physical=p.machine.invoke_prepared(&a.next,vec![executed])?;
        // Private output is bounded before it can become the next call's input.
        physical.check_limits()?;
        Ok((observed,Box::new(PreparedReference{machine:p.machine.clone(),physical})))
    }
}
pub struct Session<E: Executor> {
    checked: CheckedRepresentation,
    // Exact source identity and transport schema do not change when a physical
    // representation changes. Avoid rebuilding/hashing them on every call.
    layout: snapshot_wire::Layout,
    physical: Term,
    prepared: Option<Box<dyn std::any::Any>>,
    version: u64,
    outbox: Vec<snapshot_wire::Event>,
    executor: E,
}
impl<E: Executor> Session<E> {
    fn run(&self, function: &str, args: Vec<Term>) -> LangResult<Term> {
        self.executor.execute(
            self.checked.definition(),
            self.checked.context(),
            function,
            args,
        )
    }
    pub fn restore(checked: CheckedRepresentation, bytes: &[u8], executor: E) -> LangResult<Self> {
        let layout = ink_core::snapshot::layout(checked.module().program())?;
        let logical = snapshot_wire::decode(&layout, bytes, Limits::default())?;
        let input = codec::from_tables(&checked, &logical.tables)?;
        let physical = executor.execute(
            checked.definition(),
            checked.context(),
            &checked.plan().encode,
            vec![input],
        )?;
        let prepared = executor.prepare(&checked, &physical)?;
        Ok(Self {
            checked,
            layout,
            physical: if prepared.is_some() { Term::Bool(false) } else { physical },
            prepared,
            version: logical.version,
            outbox: logical.outbox,
            executor,
        })
    }
    pub fn new(checked: CheckedRepresentation, executor: E) -> LangResult<Self> {
        let baseline = ink_core::stateful::Runtime::new(checked.module().program().clone())?;
        Self::restore(checked, &baseline.checkpoint_portable()?, executor)
    }
    pub fn representation(&self) -> &str {
        &self.checked.request().representation
    }
    pub fn version(&self) -> u64 {
        self.version
    }
    fn physical_value(&self) -> LangResult<Term> {
        match &self.prepared { Some(state) => self.executor.materialise(&self.checked, state.as_ref()), None => Ok(self.physical.clone()) }
    }
    pub fn checkpoint(&self) -> LangResult<Vec<u8>> {
        let state = self.run(&self.checked.plan().decode, vec![self.physical_value()?])?;
        let logical = Logical {
            version: self.version,
            tables: codec::tables(&self.checked, &state)?,
            outbox: self.outbox.clone(),
        };
        snapshot_wire::encode(&self.layout, &logical, Limits::default())
    }
    pub fn acknowledge_through(&mut self, commit: u64, position: u64) {
        self.outbox
            .retain(|e| (e.commit, e.position) > (commit, position));
    }
    pub fn invoke_json(&mut self, name: &str, args: &Value) -> LangResult<Value> {
        self.invoke_json_profiled(name, args, |_| {})
    }
    /// Calls the observer immediately before each complete stage and at `done`.
    /// Ordinary execution supplies an inlined no-op and reads no clock.
    pub fn invoke_json_profiled(
        &mut self, name: &str, args: &Value, mut stage: impl FnMut(&'static str),
    ) -> LangResult<Value> {
        stage("inputs");
        let source = self
            .checked
            .module()
            .program()
            .actions
            .iter()
            .find(|a| a.name == name)
            .ok_or("unknown source action")?;
        let xs = args.as_array().ok_or("action arguments must be an array")?;
        if xs.len() != source.params.len() {
            return Err("action argument count mismatch".into());
        }
        let mut inputs = if self.prepared.is_some() { vec![Term::U64(self.version)] } else { vec![self.physical.clone(), Term::U64(self.version)] };
        for ((_, t), value) in source.params.iter().zip(xs) {
            inputs.push(codec::encode(&self.checked, t, value)?);
        }
        // Evaluate the proposal once. No authoritative mutation occurs until
        // all execution/decoding/transport checks have succeeded.
        stage("execute");
        let mut readonly=false;
        let (observed, next, prepared) = if let Some(state) = &self.prepared {
            if matches!(source.kind,ink_core::syntax::ActionKind::Query) {
                if let Some((observed,next))=self.executor.transition_prepared_readonly(&self.checked,name,state.as_ref(),inputs.clone())? {readonly=true;(observed,Term::Bool(false),Some(next))}
                else {let(observed,next)=self.executor.transition_prepared(&self.checked,name,state.as_ref(),inputs)?;(observed,Term::Bool(false),Some(next))}
            }else{
            let (observed, next) = self.executor.transition_prepared(&self.checked, name, state.as_ref(), inputs)?;
            (observed, Term::Bool(false), Some(next))
        }} else { let (observed, next) = self.executor.transition(&self.checked, name, inputs)?; (observed, next, None) };
        stage("decode");
        let Term::Construct {
            datatype,
            constructor,
            arguments,
        } = observed
        else {
            return Err("action observation carrier".into());
        };
        let ink_core::logic::Sort::Data(expected) = &self.checked.projection().actions[name].result
        else {
            return Err("action observation sort".into());
        };
        if &datatype != expected {
            return Err("action observation identity mismatch".into());
        }
        if constructor == 1 {
            return Err("commit sequence exhausted".into());
        }
        if constructor != 0 {
            return Err("action observation tag".into());
        }
        let [reply, Term::Bool(committed), Term::U64(version), _state, events] =
            arguments.as_slice()
        else {
            return Err("action observation shape".into());
        };
        let result = codec::decode(&self.checked, &source.result, reply)?;
        let new_events = codec::events(&self.checked, events)?;
        if readonly {
            stage("validate");
            if *committed || *version!=self.version || !new_events.is_empty(){return Err("invalid read-only query observation".into());}
            stage("publish");self.prepared=prepared;self.physical=next;
            let reply=json!({"result":result,"committed":false,"version":version,"events":[]});stage("done");return Ok(reply);
        }

        // All-action admission establishes logical post-state correspondence.
        // Literal executors keep that state privately; reconstruct it only at
        // checkpoint/restore or migration. Snapshot limits are checkpoint limits.
        let mut pending = vec![];
        let mut published = vec![];
        if *committed {
            if Some(*version) != self.version.checked_add(1) {
                return Err("invalid observed commit sequence".into());
            }
            for (position, (channel, value)) in new_events.into_iter().enumerate() {
                let event = snapshot_wire::Event {
                    commit: *version,
                    position: position as u64,
                    channel,
                    value: value.clone(),
                };
                published.push(json!({"commit":*version,"position":position,"channel":self.checked.module().program().events.keys().nth(channel).unwrap(),"value":value}));
                pending.push(event);
            }
        } else if *version != self.version || !new_events.is_empty() {
            return Err("invalid aborted/query observation".into());
        }
        stage("validate");
        self.outbox.try_reserve(pending.len()).map_err(|_| "outbox allocation failure")?;
        stage("publish");
        self.physical = next;
        self.prepared = prepared;
        self.version = *version;
        self.outbox.extend(pending);
        let result = json!({"result":result,"committed":committed,"version":version,"events":published});
        stage("done");
        Ok(result)
    }
    /// Generic migration reconstructs the same logical snapshot. Target encode
    /// establishes its invariant for every logical input; no profile is a premise.
    pub fn migrate(&mut self, to: CheckedRepresentation) -> LangResult<()> {
        if self.checked.module().identity()? != to.module().identity()? {
            return Err("migration changes source program".into());
        }
        let logical = self.run(&self.checked.plan().decode, vec![self.physical_value()?])?;
        let next = self.executor.execute(
            to.definition(),
            to.context(),
            &to.plan().encode,
            vec![logical.clone()],
        )?;
        // Commit the switch only after target transport validates. Any failure
        // leaves the old representation, version and pending outbox untouched.
        let decoded = self.executor.execute(
            to.definition(),
            to.context(),
            &to.plan().decode,
            vec![next.clone()],
        )?;
        codec::tables(&to, &decoded)?;
        if decoded != logical {
            return Err("migration executor changed logical state".into());
        }
        let prepared = self.executor.prepare(&to, &next)?;
        self.physical = if prepared.is_some() { Term::Bool(false) } else { next };
        self.prepared = prepared;
        self.checked = to;
        Ok(())
    }
    pub fn migrate_direct(
        &mut self,
        to: CheckedRepresentation,
        migration: &CheckedMigration,
    ) -> LangResult<()> {
        if migration.source_identity() != self.checked.module().identity()?
            || migration.endpoints()
                != (self.representation(), to.request().representation.as_str())
            || self.checked.module().identity()? != to.module().identity()?
        {
            return Err("direct migration endpoint mismatch".into());
        }
        let logical = self.run(&self.checked.plan().decode, vec![self.physical_value()?])?;
        let next = self.executor.execute(
            migration.definition(),
            migration.context(),
            migration.function(),
            vec![self.physical_value()?],
        )?;
        let decoded = self.executor.execute(
            to.definition(),
            to.context(),
            &to.plan().decode,
            vec![next.clone()],
        )?;
        codec::tables(&to, &decoded)?;
        if decoded != logical {
            return Err("migration executor changed logical state".into());
        }
        let prepared = self.executor.prepare(&to, &next)?;
        self.physical = if prepared.is_some() { Term::Bool(false) } else { next };
        self.prepared = prepared;
        self.checked = to;
        Ok(())
    }
}
