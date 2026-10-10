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
impl Executor for Reference {
    fn execute(
        &self,
        definition: &CheckedDefinition,
        context: &Context,
        function: &str,
        args: Vec<Term>,
    ) -> LangResult<Term> {
        if !definition.exports().iter().any(|x| x == function) {
            return Err("unexported representation function".into());
        }
        context.evaluate(
            &[],
            &Term::Call {
                function: function.into(),
                arguments: args,
            },
        )
    }
}
pub struct Session<E: Executor> {
    checked: CheckedRepresentation,
    // Exact source identity and transport schema do not change when a physical
    // representation changes. Avoid rebuilding/hashing them on every call.
    layout: snapshot_wire::Layout,
    physical: Term,
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
        Ok(Self {
            checked,
            layout,
            physical,
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
    pub fn checkpoint(&self) -> LangResult<Vec<u8>> {
        let state = self.run(&self.checked.plan().decode, vec![self.physical.clone()])?;
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
        let mut inputs = vec![self.physical.clone(), Term::U64(self.version)];
        for ((_, t), value) in source.params.iter().zip(xs) {
            inputs.push(codec::encode(&self.checked, t, value)?);
        }
        // Evaluate the proposal once. No authoritative mutation occurs until
        // all execution/decoding/transport checks have succeeded.
        let (observed, next) = self.executor.transition(&self.checked, name, inputs)?;
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
        let [reply, Term::Bool(committed), Term::U64(version), state, events] =
            arguments.as_slice()
        else {
            return Err("action observation shape".into());
        };
        let result = codec::decode(&self.checked, &source.result, reply)?;
        let new_events = codec::events(&self.checked, events)?;
        let tables = codec::tables(&self.checked, state)?;
        let mut outbox = self.outbox.clone();
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
                outbox.push(event);
            }
        } else if *version != self.version || !new_events.is_empty() {
            return Err("invalid aborted/query observation".into());
        }
        snapshot_wire::encode(
            &self.layout,
            &Logical {
                version: *version,
                tables,
                outbox: outbox.clone(),
            },
            Limits::default(),
        )?;
        self.physical = next;
        self.version = *version;
        self.outbox = outbox;
        Ok(json!({"result":result,"committed":committed,"version":version,"events":published}))
    }
    /// Generic migration reconstructs the same logical snapshot. Target encode
    /// establishes its invariant for every logical input; no profile is a premise.
    pub fn migrate(&mut self, to: CheckedRepresentation) -> LangResult<()> {
        if self.checked.module().identity()? != to.module().identity()? {
            return Err("migration changes source program".into());
        }
        let logical = self.run(&self.checked.plan().decode, vec![self.physical.clone()])?;
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
        self.physical = next;
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
        let logical = self.run(&self.checked.plan().decode, vec![self.physical.clone()])?;
        let next = self.executor.execute(
            migration.definition(),
            migration.context(),
            migration.function(),
            vec![self.physical.clone()],
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
        self.physical = next;
        self.checked = to;
        Ok(())
    }
}
