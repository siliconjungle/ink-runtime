use ink_runtime::durable::{CompiledState, Host, Store};
use serde_json::{json, Value};
use std::{cell::RefCell, rc::Rc};
#[derive(Clone)]
struct State {
    value: u64,
    version: u64,
    events: Vec<Value>,
}
impl CompiledState for State {
    fn create() -> Self {
        Self {
            value: 0,
            version: 0,
            events: vec![],
        }
    }
    fn restore(bytes: &[u8]) -> Result<Self, String> {
        let v: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        Ok(Self {
            value: v["value"].as_u64().unwrap(),
            version: v["version"].as_u64().unwrap(),
            events: v["events"].as_array().unwrap().clone(),
        })
    }
    fn checkpoint(&self) -> Result<Vec<u8>, String> {
        Ok(serde_json::to_vec(
            &json!({"value":self.value,"version":self.version,"events":self.events}),
        )
        .unwrap())
    }
    fn invoke(&mut self, call: &str, args: &Value) -> Result<Value, String> {
        if call == "fail" {
            self.value = 999;
            return Err("failed action".into());
        }
        if call == "read" {
            return Ok(args.clone());
        }
        self.value += args[0].as_u64().unwrap();
        self.version += 1;
        self.events
            .push(json!({"commit":self.version,"position":0,"value":self.value}));
        Ok(json!({"value":self.value,"version":self.version}))
    }
    fn pure(&mut self, _: &str, args: &Value) -> Result<Value, String> {
        Ok(args.clone())
    }
    fn events(&self) -> Vec<Value> {
        self.events.clone()
    }
    fn acknowledge(&mut self, c: u64, p: u64) {
        self.events.retain(|e| {
            (
                e["commit"].as_u64().unwrap(),
                e["position"].as_u64().unwrap(),
            ) > (c, p)
        });
    }
    fn version(&self) -> u64 {
        self.version
    }
}
#[derive(Default)]
struct Data {
    saved: Option<(String, Vec<u8>)>,
    fail: u8,
    sequence: u64,
}
#[derive(Clone, Default)]
struct Memory(Rc<RefCell<Data>>);
impl Store for Memory {
    fn load(&mut self) -> Result<Option<(String, Vec<u8>)>, String> {
        Ok(self.0.borrow().saved.clone())
    }
    fn commit(&mut self, expected: Option<&str>, bytes: &[u8]) -> Result<String, String> {
        let mut d = self.0.borrow_mut();
        if d.saved.as_ref().map(|(t, _)| t.as_str()) != expected {
            return Err("conflict".into());
        }
        if d.fail == 1 {
            return Err("before publication".into());
        }
        d.sequence += 1;
        let token = d.sequence.to_string();
        d.saved = Some((token.clone(), bytes.to_vec()));
        if d.fail == 2 {
            return Err("after publication".into());
        }
        Ok(token)
    }
}
#[test]
fn uncertain_writes_poison_until_reopen_and_retries_execute_once() {
    for failure in [1, 2] {
        let store = Memory::default();
        let mut host = Host::<State, _>::open(store.clone(), 2).unwrap();
        store.0.borrow_mut().fail = failure;
        assert!(host
            .invoke("lost", "add", &json!([7]))
            .unwrap_err()
            .contains("reopen"));
        assert!(host.version().is_err());
        assert!(host.checkpoint().is_err());
        assert!(host.pure("read", &json!([])).is_err());
        drop(host);
        store.0.borrow_mut().fail = 0;
        let mut host = Host::<State, _>::open(store.clone(), 4096).unwrap();
        assert_eq!(host.version().unwrap(), if failure == 1 { 0 } else { 1 });
        assert_eq!(host.invoke("lost", "add", &json!([7])).unwrap()["value"], 7);
        assert_eq!(host.version().unwrap(), 1);
        assert!(host.invoke("lost", "add", &json!([8])).is_err());
        assert!(host.invoke("abort", "fail", &json!([])).is_err());
        assert_eq!(host.version().unwrap(), 1);
        host.invoke("b", "add", &json!([1])).unwrap();
        host.invoke("c", "add", &json!([1])).unwrap();
        // The retained limit comes from storage, even when reopen supplied another value.
        host.invoke("lost", "add", &json!([7])).unwrap();
        assert_eq!(host.version().unwrap(), 4);
    }
}
#[test]
fn receiver_failure_and_uncertain_ack_allow_duplicate_delivery() {
    let store = Memory::default();
    let mut host = Host::<State, _>::open(store.clone(), 256).unwrap();
    host.invoke("one", "add", &json!([1])).unwrap();
    host.invoke("two", "add", &json!([2])).unwrap();
    assert!(host
        .deliver(|_, key| if key.0 == 2 {
            Err("receiver failed".into())
        } else {
            Ok(())
        })
        .is_err());
    assert_eq!(host.events().unwrap().len(), 1);
    store.0.borrow_mut().fail = 1;
    let mut seen = 0;
    assert!(host
        .deliver(|_, key| {
            assert_eq!(key, (2, 0));
            seen += 1;
            Ok(())
        })
        .is_err());
    drop(host);
    store.0.borrow_mut().fail = 0;
    let mut host = Host::<State, _>::open(store.clone(), 256).unwrap();
    host.deliver(|_, key| {
        assert_eq!(key, (2, 0));
        seen += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(seen, 2);
    drop(host);
    assert!(Host::<State, _>::open(store.clone(), 256)
        .unwrap()
        .events()
        .unwrap()
        .is_empty());
    store.0.borrow_mut().saved.as_mut().unwrap().1[12] ^= 1;
    assert!(Host::<State, _>::open(store, 256)
        .err()
        .unwrap()
        .contains("checksum"));
}
#[test]
fn exact_receipts_and_structure_limits() {
    let store = Memory::default();
    let mut host = Host::<State, _>::open(store.clone(), 256).unwrap();
    let args = json!([u64::MAX,i64::MIN,-0.0,0.125,"雪\u{0}😀",{"a":[true,null]}]);
    let reply = host.invoke("exact", "read", &args).unwrap();
    drop(host);
    let mut host = Host::<State, _>::open(store.clone(), 256).unwrap();
    assert_eq!(host.invoke("exact", "read", &args).unwrap(), reply);
    assert!(host.invoke("exact", "read", &args).unwrap()[2]
        .as_f64()
        .unwrap()
        .is_sign_negative());
    let mut deep = json!(0);
    for _ in 0..49 {
        deep = json!([deep]);
    }
    let before = store.0.borrow().sequence;
    assert!(host
        .invoke("deep", "read", &deep)
        .unwrap_err()
        .contains("structure limit"));
    assert_eq!(store.0.borrow().sequence, before);
    let mut stale = Host::<State, _>::open(store.clone(), 256).unwrap();
    host.invoke("new", "add", &json!([1])).unwrap();
    assert!(stale.invoke("stale", "add", &json!([1])).is_err());
}
