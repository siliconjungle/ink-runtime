//! Opt-in, sampled operation-site diagnostics. No document traversal or retained
//! raw keys, no model fitting, and no effect on implementation eligibility.
use ink_abi::usage::{self, Event, Summary};
use std::collections::VecDeque;
#[derive(Clone, Debug)]
pub struct Window {
    pub summary: Summary,
    pub operation_mix_drift: Option<f64>,
    pub calls: u64,
    pub total_calls: u64,
    pub sample_every: u64,
}
pub struct Recorder {
    window_calls: u64,
    sample_every: u64,
    max_samples: usize,
    history_windows: usize,
    calls: u64,
    total: u64,
    events: Vec<Event>,
    overflow: bool,
    recent: VecDeque<Window>,
}
impl Recorder {
    pub fn new(
        window_calls: u64,
        sample_every: u64,
        max_samples: usize,
        history_windows: usize,
    ) -> Result<Self, String> {
        if !(1..=1_000_000).contains(&window_calls)
            || !(1..=1_000_000).contains(&sample_every)
            || !(1..=256).contains(&max_samples)
            || !(1..=64).contains(&history_windows)
        {
            return Err("usage window budget".into());
        }
        Ok(Self {
            window_calls,
            sample_every,
            max_samples,
            history_windows,
            calls: 0,
            total: 0,
            events: vec![],
            overflow: false,
            recent: VecDeque::new(),
        })
    }
    pub fn observe(&mut self, event: Option<Event>) -> Result<Option<&Window>, String> {
        self.calls += 1;
        self.total += 1;
        if self.total % self.sample_every == 0 {
            if self.events.len() >= self.max_samples {
                self.overflow = true;
            } else if let Some(e) = event {
                self.events.push(e);
            } else {
                self.overflow = true;
            }
        }
        if self.calls < self.window_calls {
            return Ok(None);
        }
        let summary = usage::summarize(&self.events, self.overflow);
        self.events.clear();
        self.overflow = false;
        let calls = self.calls;
        self.calls = 0;
        let summary = summary?;
        let drift = self
            .recent
            .back()
            .and_then(|w| usage::drift(&w.summary, &summary));
        self.recent.push_back(Window {
            summary,
            operation_mix_drift: drift,
            calls,
            total_calls: self.total,
            sample_every: self.sample_every,
        });
        if self.recent.len() > self.history_windows {
            self.recent.pop_front();
        }
        Ok(self.recent.back())
    }
    pub fn recent(&self) -> &VecDeque<Window> {
        &self.recent
    }
}
