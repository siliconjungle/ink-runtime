//! Opt-in synchronous host for already compiled state variants. Adapter code and
//! deployment binding are trusted; no interpreter, proof search or trial calls.
use ink_abi::adaptive::{self, Binding, Policy, Profile, Request, State, Window};
use std::{
    collections::{BTreeMap, VecDeque},
    time::Instant,
};

pub trait CompiledState {
    type Reply;
    fn call(&mut self, action: &str, arguments: &serde_json::Value) -> Result<Self::Reply, String>;
    fn checkpoint(&self) -> Result<Vec<u8>, String>;
}
pub trait Factories {
    type Instance: CompiledState;
    fn create(&self, variant: &str) -> Result<Self::Instance, String>;
    fn restore(&self, variant: &str, snapshot: &[u8]) -> Result<Self::Instance, String>;
}
#[derive(Clone, Debug)]
pub struct WindowStats {
    pub window: Window,
    pub sampled_calls: u64,
    pub sampled_ns: u128,
}
/// Exclusive &mut access is the transaction boundary. The facade does not expose
/// an instance reference, so callers cannot interleave an action with migration.
pub struct Host<F: Factories> {
    factories: F,
    live: F::Instance,
    request: Request,
    window_calls: u64,
    sample_every: u64,
    max_snapshot_bytes: usize,
    sampled_calls: u64,
    sampled_ns: u128,
    recent: VecDeque<WindowStats>,
    pub disabled: bool,
    recording: bool,
    pub last_diagnostic: String,
}
impl<F: Factories> Host<F> {
    pub fn new(
        factories: F,
        binding: Binding,
        profile: Profile,
        policy: Policy,
        baseline: String,
        window_calls: u64,
        sample_every: u64,
        max_snapshot_bytes: usize,
    ) -> Result<Self, String> {
        if !(1..=1_000_000).contains(&window_calls)
            || !(1..=1_000_000).contains(&sample_every)
            || max_snapshot_bytes == 0
        {
            return Err("adaptive host budget".into());
        }
        let request = Request {
            binding,
            profile,
            policy,
            current: baseline,
            state: State::default(),
            window: Window {
                counts: BTreeMap::from([("validate@0".into(), 1)]),
                calls: 1,
                total_calls: 1,
                state_elements: 0,
                saturated: false,
            },
        };
        adaptive::validate(&request)?;
        let live = factories.create(&request.current)?;
        let mut host = Self {
            factories,
            live,
            request,
            window_calls,
            sample_every,
            max_snapshot_bytes,
            sampled_calls: 0,
            sampled_ns: 0,
            recent: VecDeque::new(),
            disabled: false,
            recording: true,
            last_diagnostic: String::new(),
        };
        host.request.window.counts.clear();
        host.request.window.calls = 0;
        host.request.window.total_calls = 0;
        Ok(host)
    }
    /// Stop all runtime recording as well as adaptation. Ordinary fixed calls
    /// then invoke the compiled state directly, without timing or window work.
    pub fn disable_recording(&mut self) {
        self.disabled = true;
        self.recording = false;
    }
    pub fn current(&self) -> &str {
        &self.request.current
    }
    pub fn recent(&self) -> &VecDeque<WindowStats> {
        &self.recent
    }
    pub fn checkpoint(&self) -> Result<Vec<u8>, String> {
        let bytes = self.live.checkpoint()?;
        if bytes.len() > self.max_snapshot_bytes {
            return Err("adaptive snapshot budget".into());
        }
        Ok(bytes)
    }
    pub fn call(
        &mut self,
        action: &str,
        args: &serde_json::Value,
        shape: &str,
        state_elements: u64,
    ) -> Result<<F::Instance as CompiledState>::Reply, String> {
        if !self.recording {
            return self.live.call(action, args);
        }
        let sampled = self.request.window.total_calls % self.sample_every == 0;
        let start = sampled.then(Instant::now);
        let reply = self.live.call(action, args);
        if let Some(t) = start {
            self.sampled_calls += 1;
            self.sampled_ns += t.elapsed().as_nanos();
        }
        let w = &mut self.request.window;
        w.calls += 1;
        w.total_calls = w.total_calls.saturating_add(1);
        w.state_elements = state_elements;
        let cell = format!(
            "{}action:{}@{}",
            if reply.is_err() { "failed:" } else { "" },
            action,
            shape
        );
        if cell.len() > 256
            || !cell.is_ascii()
            || (w.counts.len() >= 64 && !w.counts.contains_key(&cell))
        {
            w.saturated = true;
        } else {
            *w.counts.entry(cell).or_default() += 1;
        }
        if w.calls == self.window_calls {
            self.recent.push_back(WindowStats {
                window: w.clone(),
                sampled_calls: self.sampled_calls,
                sampled_ns: self.sampled_ns,
            });
            if self.recent.len() > 4 {
                self.recent.pop_front();
            }
            if !self.disabled {
                if w.saturated {
                    self.request.state.pending.clear();
                    self.request.state.confirmations = 0;
                } else if let Err(e) = self.consider() {
                    self.disabled = true;
                    self.last_diagnostic = e;
                }
            }
            self.request.window.calls = 0;
            self.request.window.counts.clear();
            self.request.window.saturated = false;
            self.sampled_calls = 0;
            self.sampled_ns = 0;
        }
        // A failed migration never changes or retries an action's result.
        reply
    }
    fn consider(&mut self) -> Result<(), String> {
        let started = Instant::now();
        let decision = adaptive::decide(&self.request)?;
        self.last_diagnostic = decision.reason;
        self.request.state = decision.state;
        if decision.selected == self.request.current {
            return Ok(());
        }
        let bytes = self.checkpoint()?;
        let candidate = self.factories.restore(&decision.selected, &bytes)?;
        if candidate.checkpoint()? != bytes {
            return Err("adaptive restore changed snapshot".into());
        }
        if started.elapsed().as_nanos() as f64 > self.request.policy.max_pause_ns {
            return Err("adaptive observed pause budget".into());
        }
        self.live = candidate;
        self.request.current = decision.selected;
        self.request.state = State {
            pending: String::new(),
            confirmations: 0,
            last_switch_call: self.request.window.total_calls,
            switches: self.request.state.switches + 1,
        };
        Ok(())
    }
}

/// Explicitly independent collections. No inferred source partitioning or atomic
/// transaction across hosts; each Host retains its own exclusive action boundary.
pub struct Collections<F: Factories> {
    hosts: BTreeMap<String, Host<F>>,
}
impl<F: Factories> Default for Collections<F> {
    fn default() -> Self {
        Self {
            hosts: BTreeMap::new(),
        }
    }
}
impl<F: Factories> Collections<F> {
    pub fn insert(&mut self, id: String, host: Host<F>) -> Result<(), String> {
        if id.is_empty()
            || id.len() > 64
            || !id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
            || self.hosts.len() >= 64
            || self.hosts.contains_key(&id)
        {
            return Err("adaptive collection budget/identity".into());
        }
        self.hosts.insert(id, host);
        Ok(())
    }
    pub fn get_mut(&mut self, id: &str) -> Result<&mut Host<F>, String> {
        self.hosts
            .get_mut(id)
            .ok_or_else(|| "unknown collection".into())
    }
}
