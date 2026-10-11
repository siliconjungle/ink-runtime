#[cfg(not(target_arch = "wasm32"))]
pub mod durable;
#[cfg(not(target_arch = "wasm32"))]
impl durable::CompiledState for State {
    fn create() -> Self {
        Self::new()
    }
    fn restore(bytes: &[u8]) -> Result<Self, String> {
        Self::restore(bytes)
    }
    fn checkpoint(&self) -> Result<Vec<u8>, String> {
        self.checkpoint()
    }
    fn invoke(
        &mut self,
        call: &str,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.invoke_json(call, args)
    }
    fn pure(&mut self, call: &str, args: &serde_json::Value) -> Result<serde_json::Value, String> {
        self.call_json(call, args)
    }
    fn events(&self) -> Vec<serde_json::Value> {
        self.outbox().iter().map(Event::to_json).collect()
    }
    fn acknowledge(&mut self, commit: u64, position: u64) {
        self.acknowledge_through(commit, position)
    }
    fn begin_transaction(&mut self) -> Result<bool, String> {
        self.begin_transaction()
    }
    fn commit_transaction(&mut self) {
        self.commit_transaction()
    }
    fn rollback_transaction(&mut self) {
        self.rollback_transaction()
    }
    fn version(&self) -> u64 {
        self.version()
    }
}
#[cfg(not(target_arch = "wasm32"))]
pub type DurableState = durable::Host<State, durable::FileStore>;
