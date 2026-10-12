#[cfg(target_endian = "big")]
compile_error!("packed compute CPU ABI currently requires little endian");
// Compiled CPU and resident wgpu pipelines using the same packed ABI and WGSL
// as the browser host. Profiles measure whole executions, never correctness.
mod assets;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    ffi::c_void,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
type Result<T> = std::result::Result<T, String>;
const MAX_BYTES: usize = 256 * 1024 * 1024;
extern "C" {
    fn ink_compute_call(
        f: u32,
        data: *const *const c_void,
        lengths: *const usize,
        out: *mut *mut c_void,
        count: *mut usize,
    );
    fn ink_compute_reset();
    fn ink_compute_into(
        f: u32,
        data: *const *const c_void,
        lengths: *const usize,
        destination: *mut c_void,
    ) -> usize;
}
mod backend;
pub use backend::Backend;
use ink_abi::packed::{width,Data,pack,unpack};
#[derive(Clone)]
struct Ref {
    step: bool,
    index: usize,
    input: Option<String>,
    ty: Value,
    count: Option<usize>,
}
struct Step {
    id: String,
    f: Value,
    args: Vec<Ref>,
    count: Option<usize>,
}
struct Plan {
    constants: Vec<Data>,
    inputs: BTreeMap<String, Ref>,
    steps: Vec<Step>,
    outputs: Vec<usize>,
    feedback: BTreeMap<String, usize>,
    iterations: usize,
    reason: Option<String>,
    bytes: usize,
}
fn compile(v: &Value, m: &Value) -> Result<Plan> {
    let source = v["steps"]
        .as_array()
        .ok_or("pipeline steps must be an array")?;
    if source.is_empty() || source.len() > 64 {
        return Err("pipeline requires 1..64 steps".into());
    }
    let iterations = v
        .get("iterations")
        .map(|n| n.as_u64().ok_or("invalid iterations"))
        .transpose()?
        .unwrap_or(1) as usize;
    if iterations == 0 || iterations > 10000 || iterations * source.len() > 65536 {
        return Err("pipeline iteration/dispatch limit".into());
    }
    let mut constants = Vec::new();
    let mut inputs: BTreeMap<String, Ref> = BTreeMap::new();
    let mut steps: Vec<Step> = Vec::new();
    let mut ids = BTreeMap::new();
    let mut reason = None;
    for s in source {
        let id = s["id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or("invalid step id")?
            .to_string();
        if ids.contains_key(&id) {
            return Err("duplicate step id".into());
        }
        let f = m["functions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["name"] == s["call"])
            .ok_or("unknown function")?
            .clone();
        let a = s["args"].as_array().ok_or("arguments must be an array")?;
        let params = f["params"].as_array().unwrap();
        if a.len() != params.len() {
            return Err("argument count mismatch".into());
        }
        let mut args = Vec::new();
        for (a, param) in a.iter().zip(params) {
            let t = &param["type"];
            let reference = a.as_object().filter(|o| o.len() == 1);
            let r = if let Some(id) = reference.and_then(|o| o.get("step")) {
                let id = id.as_str().ok_or("invalid step reference")?;
                let index = *ids
                    .get(id)
                    .ok_or("step reference must name an earlier step")?;
                let s: &Step = &steps[index];
                if s.f["result"] != *t {
                    return Err("step reference type mismatch".into());
                }
                Ref {
                    step: true,
                    index,
                    input: None,
                    ty: t.clone(),
                    count: s.count,
                }
            } else if let Some(input) = reference.and_then(|o| o.get("input")) {
                let input = input.as_str().ok_or("invalid input reference")?;
                if let Some(r) = inputs.get(input) {
                    if r.ty != *t {
                        return Err("pipeline input used with different types".into());
                    }
                    r.clone()
                } else {
                    let value = v["inputs"].get(input).ok_or("unknown pipeline input")?;
                    let d = pack(value, t)?;
                    let count = d.count;
                    constants.push(d);
                    let r = Ref {
                        step: false,
                        index: constants.len() - 1,
                        input: Some(input.into()),
                        ty: t.clone(),
                        count: Some(count),
                    };
                    inputs.insert(input.into(), r.clone());
                    r
                }
            } else {
                let d = pack(a, t)?;
                let count = d.count;
                constants.push(d);
                Ref {
                    step: false,
                    index: constants.len() - 1,
                    input: None,
                    ty: t.clone(),
                    count: Some(count),
                }
            };
            args.push(r);
        }
        if let Some(limit) = f["gpu_max_elements"].as_u64() {
            if args.iter().any(|a| a.count.unwrap_or(0) > limit as usize) {
                reason.get_or_insert(
                    "literal GPU collection work limit; compiled CPU fallback".into(),
                );
            }
        }
        let count = if f["graph_required"].as_bool() == Some(true) {
            Some(
                device::graph_capacity(
                    &f["graph"],
                    &args
                        .iter()
                        .map(|a| a.count.unwrap_or(0))
                        .collect::<Vec<_>>(),
                )?
                .1,
            )
        } else if f["shader"].is_string() {
            f["lengths"]
                .as_array()
                .unwrap()
                .iter()
                .map(|i| args[i.as_u64().unwrap() as usize].count)
                .collect::<Option<Vec<_>>>()
                .and_then(|a| a.into_iter().max())
        } else {
            reason.get_or_insert_with(|| {
                f["fallback_reason"]
                    .as_str()
                    .unwrap_or("CPU-only function")
                    .into()
            });
            None
        };
        ids.insert(id.clone(), steps.len());
        steps.push(Step { id, f, args, count });
    }
    let output_ids = v
        .get("outputs")
        .cloned()
        .unwrap_or(json!([steps.last().unwrap().id]));
    let output_ids = output_ids.as_array().ok_or("outputs must be an array")?;
    let mut outputs = Vec::new();
    for id in output_ids {
        let i = *ids
            .get(id.as_str().ok_or("invalid output id")?)
            .ok_or("unknown output")?;
        if outputs.contains(&i) {
            return Err("duplicate output".into());
        }
        outputs.push(i);
    }
    if outputs.is_empty() {
        return Err("pipeline requires outputs".into());
    }
    if steps
        .iter()
        .any(|s| s.f["graph_required"].as_bool() == Some(true))
        && steps.iter().any(|s| !s.f["graph"].is_object())
    {
        reason.get_or_insert(
            "GPU mixed stage graph requires a graph descriptor for every stage".into(),
        );
    }
    let mut feedback = BTreeMap::new();
    if let Some(f) = v.get("feedback") {
        for (input, id) in f.as_object().ok_or("feedback must be an object")? {
            let r = inputs.get(input).ok_or("unknown feedback input")?;
            let i = *ids
                .get(id.as_str().ok_or("invalid feedback step")?)
                .ok_or("unknown feedback step")?;
            if r.ty != steps[i].f["result"] {
                return Err("feedback type mismatch".into());
            }
            if r.count != steps[i].count {
                reason.get_or_insert("feedback changes array shape; compiled CPU pipeline".into());
            }
            feedback.insert(input.clone(), i);
        }
    }
    let feedback_bytes = if steps
        .iter()
        .any(|s| s.f["graph_required"].as_bool() == Some(true))
    {
        feedback.values().try_fold(0usize, |bytes, i| {
            let step = &steps[*i];
            let words = step
                .count
                .unwrap_or(0)
                .checked_mul(width(&step.f["result"]))
                .and_then(|n| n.checked_add(usize::from(step.f["result"]["kind"] == "list")))
                .ok_or("feedback snapshot size overflow")?;
            bytes
                .checked_add(
                    words
                        .checked_mul(4)
                        .ok_or("feedback snapshot size overflow")?
                        .max(4),
                )
                .ok_or("feedback snapshot size overflow")
        })?
    } else {
        0
    };
    let bytes = feedback_bytes
        + constants
            .iter()
            .map(|c| ((c.words.len() + 1) * 4).max(4))
            .sum::<usize>()
        + steps
            .iter()
            .map(|s| {
                2 * ((1 + s.count.unwrap_or(0) * width(&s.f["result"])) * 4).max(4)
                    + 4 * (s.args.len() + 1)
            })
            .sum::<usize>()
        + outputs
            .iter()
            .map(|i| {
                ((1 + steps[*i].count.unwrap_or(0) * width(&steps[*i].f["result"])) * 4).max(4)
            })
            .sum::<usize>();
    if bytes > MAX_BYTES {
        reason.get_or_insert("GPU live-buffer budget exceeds 256 MiB".into());
    }
    Ok(Plan {
        constants,
        inputs,
        steps,
        outputs,
        feedback,
        iterations,
        reason,
        bytes,
    })
}
fn cpu_call(f: &Value, args: &[Data]) -> Result<Data> {
    if !f["precondition"].is_null() && cpu_call(&f["precondition"],args)?.words.first()!=Some(&1) {
        return Err("contract precondition failed".into());
    }
    let pointers: Vec<*const c_void> = args.iter().map(|d| d.words.as_ptr().cast()).collect();
    let lengths: Vec<usize> = args.iter().map(|d| d.count).collect();
    unsafe {
        ink_compute_reset();
        let mut out = std::ptr::null_mut();
        let mut count = 0;
        ink_compute_call(
            f["index"].as_u64().unwrap() as u32,
            pointers.as_ptr(),
            lengths.as_ptr(),
            &mut out,
            &mut count,
        );
        if count > MAX_BYTES / 4 {
            ink_compute_reset();
            return Err("CPU output exceeds 256 MiB".into());
        }
        let words = std::slice::from_raw_parts(out.cast::<u32>(), count).to_vec();
        ink_compute_reset();
        let count = if f["result"]["kind"] == "list" {
            count / width(&f["result"]["element"])
        } else {
            1
        };
        Ok(Data {
            words: Arc::new(words),
            count,
            finite: true,
        })
    }
}
fn cpu(p: &Plan) -> Result<Value> {
    let mut inputs: BTreeMap<String, Data> = p
        .inputs
        .iter()
        .map(|(n, r)| (n.clone(), p.constants[r.index].clone()))
        .collect();
    let mut values: Vec<Option<Data>> = vec![None; p.steps.len()];
    for _ in 0..p.iterations {
        for (i, s) in p.steps.iter().enumerate() {
            let args = s
                .args
                .iter()
                .map(|r| {
                    if let Some(n) = &r.input {
                        inputs[n].clone()
                    } else if r.step {
                        values[r.index].as_ref().unwrap().clone()
                    } else {
                        p.constants[r.index].clone()
                    }
                })
                .collect::<Vec<_>>();
            values[i] = Some(cpu_call(&s.f, &args)?);
        }
        for (n, i) in &p.feedback {
            inputs.insert(n.clone(), values[*i].as_ref().unwrap().clone());
        }
    }
    let values: serde_json::Map<_, _> = p
        .outputs
        .iter()
        .map(|i| {
            (
                p.steps[*i].id.clone(),
                unpack(values[*i].as_ref().unwrap(), &p.steps[*i].f["result"]),
            )
        })
        .collect();
    Ok(json!({"values":values,"counters":{"upload_bytes":0,"readback_bytes":0,"dispatches":0}}))
}
mod device;
use device::Gpu;
pub struct Engine {
    manifest: Value,
    gpu: Option<Gpu>,
    gpu_error: Option<String>,
    selection_options: Value,
}
impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}
impl Engine {
    pub fn new() -> Self {
        Self {
            manifest: serde_json::from_str(assets::MANIFEST).unwrap(),
            gpu: None,
            gpu_error: None,
            selection_options: json!({}),
        }
    }
    /// Execute a compiled list function into caller-owned packed storage.
    ///
    /// # Safety
    /// Parameters must match the declared function ABI; lengths are logical
    /// element counts. The destination must be sufficiently large, aligned to
    /// eight bytes, writable and disjoint from every reachable input. No other
    /// arena-backed result may remain live across this call. Returned count is
    /// in 32-bit words. Caller storage remains valid after scratch is reset.
    pub unsafe fn call_packed_into(
        &mut self,
        index: u32,
        data: &[*const c_void],
        lengths: &[usize],
        destination: *mut u32,
    ) -> usize {
        ink_compute_reset();
        let count = ink_compute_into(index, data.as_ptr(), lengths.as_ptr(), destination.cast());
        ink_compute_reset();
        count
    }
    pub fn configure_selection(&mut self,options:Value)->Result<()>{
        ink_abi::selection::decide(&options,&json!({}),&["cpu".into()])?;
        self.selection_options=options;Ok(())
    }
    pub fn manifest(&self) -> &Value {
        &self.manifest
    }
    pub fn call(&mut self, name: &str, args: &Value, backend: Backend) -> Result<Value> {
        let mut r = self.pipeline(
            &json!({"steps":[{"id":"result","call":name,"args":args}],"outputs":["result"]}),
            backend,
        )?;
        r["value"] = r["values"]["result"].clone();
        Ok(r)
    }
    pub fn pipeline(&mut self, source: &Value, backend: Backend) -> Result<Value> {
        let p = compile(source, &self.manifest)?;
        if backend==Backend::GpuRequired && p.reason.is_some(){return Err(p.reason.clone().unwrap());}
        if backend == Backend::Cpu || p.reason.is_some() {
            let mut r = cpu(&p)?;
            r["backend"] = json!("cpu");
            r["reason"] = json!(if backend == Backend::Cpu {
                "explicit CPU selection"
            } else {
                p.reason.as_deref().unwrap()
            });
            return Ok(r);
        }
        let scope=json!([self.manifest["core_sha256"],self.manifest["cpu_c_sha256"],self.manifest["javascript_sha256"],p.steps.iter().map(|s|s.f["name"].as_str().unwrap()).collect::<Vec<_>>().join("|")]).to_string();
        let graph_mode=p.steps.iter().any(|s|s.f["graph_required"].as_bool()==Some(true));
        let list_constants=p.inputs.values().chain(p.steps.iter().flat_map(|s|s.args.iter())).filter(|r|!r.step&&r.ty["kind"]=="list").map(|r|r.index).collect::<std::collections::BTreeSet<_>>();
        let inputs=p.constants.iter().enumerate().map(|(i,c)|json!({"list":list_constants.contains(&i),"count":c.count,"bytes":c.words.len()*4})).collect::<Vec<_>>();
        let outputs=p.outputs.iter().map(|i|p.steps[*i].count.unwrap_or(0)).collect::<Vec<_>>();
        let stages=p.steps.iter().map(|s|json!({"dispatches":if graph_mode{s.f["graph"]["nodes"].as_array().map_or(0,Vec::len)}else{usize::from(s.count.unwrap_or(0)>0)}})).collect::<Vec<_>>();
        let features=ink_abi::selection::features(&json!({"inputs":inputs,"outputs":outputs,"stages":stages,"iterations":p.iterations}))?;
        let decision=ink_abi::selection::decide(&self.selection_options,&json!({"scope":scope,"boundary":"captured-owned","features":features}),&["cpu".into(),"gpu".into()])?;
        if backend==Backend::Auto && decision["backend"]=="cpu"{let mut r=cpu(&p)?;r["backend"]=json!("cpu");r["reason"]=decision["reason"].clone();r["profile"]=decision;return Ok(r);}
        let selected = (|| -> Result<Value> {
            if let Some(e) = &self.gpu_error {
                return Err(e.clone());
            }
            if self.gpu.is_none() {
                match Gpu::new() {
                    Ok(g) => self.gpu = Some(g),
                    Err(e) => {
                        self.gpu_error = Some(e.clone());
                        return Err(e);
                    }
                }
            }
            self.gpu.as_mut().unwrap().prepare(&p)?;
            let mut r=self.gpu.as_ref().unwrap().execute(&p)?;
            r["backend"]=json!("gpu");r["reason"]=if matches!(backend,Backend::Gpu|Backend::GpuRequired){json!("explicit GPU selection")}else{decision["reason"].clone()};r["profile"]=decision;
            Ok(r)
        })();
        match selected {
            Ok(r) => Ok(r),
            Err(e) => {
                if backend==Backend::GpuRequired{return Err(e);}
                let mut r = cpu(&p)?;
                r["backend"] = json!("cpu");
                r["reason"] = json!(format!("GPU fallback: {e}"));
                Ok(r)
            }
        }
    }
}
