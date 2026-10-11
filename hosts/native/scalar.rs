//! Standalone native host: generated C CPU code, the same WGSL as the browser,
//! and bounded measured selection. No Ink AST interpreter is linked.
mod assets;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    ffi::c_void,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, String>;
extern "C" {
    fn ink_cpu_call(
        function: u32,
        lists: *const *const c_void,
        lengths: *const usize,
        scalars: *const u64,
    ) -> u64;
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Auto,
    Cpu,
    Gpu,
}
impl Backend {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "auto" => Ok(Self::Auto),
            "cpu" => Ok(Self::Cpu),
            "gpu" => Ok(Self::Gpu),
            _ => Err("backend must be auto, cpu or gpu".into()),
        }
    }
}
#[derive(Clone)]
enum Arg {
    Scalar(u64),
    U32List(Arc<[u32]>),
    U64List(Arc<[u64]>),
}
fn parse_args(f: &Value, args: &Value) -> Result<Vec<Arg>> {
    let args = args.as_array().ok_or("arguments must be a JSON array")?;
    let params = f["params"].as_array().ok_or("invalid compiled manifest")?;
    if args.len() != params.len() {
        return Err("argument count mismatch".into());
    }
    args.iter()
        .zip(params)
        .map(|(v, p)| {
            let t = p["type"].as_str().ok_or("invalid compiled type")?;
            let word = |x: &Value, small: bool| -> Result<u64> {
                let n = x.as_u64().ok_or("expected unsigned integer")?;
                if small && n > u32::MAX as u64 {
                    return Err("u32 outside range".into());
                }
                Ok(n)
            };
            Ok(match t {
                "u32" => Arg::Scalar(word(v, true)?),
                "u64" => Arg::Scalar(word(v, false)?),
                "bool" => Arg::Scalar(v.as_bool().ok_or("expected Bool")? as u64),
                "list_u32" => Arg::U32List(
                    v.as_array()
                        .ok_or("expected list")?
                        .iter()
                        .map(|x| word(x, true).map(|n| n as u32))
                        .collect::<Result<Vec<_>>>()?
                        .into(),
                ),
                "list_u64" => Arg::U64List(
                    v.as_array()
                        .ok_or("expected list")?
                        .iter()
                        .map(|x| word(x, false))
                        .collect::<Result<Vec<_>>>()?
                        .into(),
                ),
                _ => return Err("unsupported compiled type".into()),
            })
        })
        .collect()
}
fn cpu(f: &Value, args: &[Arg]) -> u64 {
    let mut lists = vec![std::ptr::null(); args.len()];
    let mut lengths = vec![0; args.len()];
    let mut scalars = vec![0; args.len()];
    for (i, v) in args.iter().enumerate() {
        match v {
            Arg::Scalar(n) => scalars[i] = *n,
            Arg::U32List(xs) => {
                lists[i] = xs.as_ptr().cast();
                lengths[i] = xs.len();
            }
            Arg::U64List(xs) => {
                lists[i] = xs.as_ptr().cast();
                lengths[i] = xs.len();
            }
        }
    }
    // Validated ABI arrays remain live for this synchronous generated C call.
    unsafe {
        ink_cpu_call(
            f["index"].as_u64().unwrap() as u32,
            lists.as_ptr(),
            lengths.as_ptr(),
            scalars.as_ptr(),
        )
    }
}
fn list<'a>(f: &Value, args: &'a [Arg]) -> Result<&'a [u32]> {
    match &args[f["gpu"]["list_param"].as_u64().ok_or("missing GPU list")? as usize] {
        Arg::U32List(xs) => Ok(xs),
        _ => Err("GPU needs u32 list".into()),
    }
}
fn ms(t:Instant)->f64{t.elapsed().as_secs_f64()*1000.0}
pub struct Engine {
    manifest: Value,
    gpu: Option<Gpu>,
    gpu_error: Option<String>,
    disabled: BTreeMap<u64, String>,
    selection_options: Value,
}
impl Engine {
    pub fn new() -> Self {
        Self {
            manifest: serde_json::from_str(assets::MANIFEST).expect("compiler-generated manifest"),
            gpu: None,
            gpu_error: None,
            disabled: BTreeMap::new(),
            selection_options: json!({}),
        }
    }
    pub fn configure_selection(&mut self,options:Value)->Result<()>{ink_abi::selection::decide(&options,&json!({}),&["cpu".into()])?;self.selection_options=options;Ok(())}
    pub fn manifest(&self) -> &Value {
        &self.manifest
    }
    /// Execute only the compiler-embedded, structurally checked pure call graph.
    /// This does not accept runtime proposal JSON as proof of source equivalence.
    pub fn call_route(&mut self, args: &Value) -> Result<Value> {
        let start = Instant::now();
        let route: Value = serde_json::from_str(assets::SOURCE_ROUTE.ok_or("bundle has no compiled route")?).map_err(|e| e.to_string())?;
        if route["input_core_sha256"] != self.manifest["core_sha256"] {
            return Err("compiled route core mismatch".into());
        }
        let entry = route["entry"].as_str().ok_or("missing route entry")?;
        let f = self.manifest["functions"].as_array().ok_or("missing compiled functions")?.iter().find(|f| f["name"] == entry).ok_or("missing compiled route entry")?;
        let result_bool = f["result"]=="bool";
        let inputs = parse_args(f, args)?; // Validate/capture before any device work.
        let mut values = BTreeMap::<String, Arg>::new();
        let mut evidence = Vec::new();
        let resolve = |r: &Value, values: &BTreeMap<String, Arg>| -> Result<Arg> {
            if let Some(i) = r["Input"].as_u64() {
                return inputs.get(i as usize).cloned().ok_or("route input out of range".into());
            }
            if let Some(id) = r["Stage"].as_str() {
                return values.get(id).cloned().ok_or("missing route result".into());
            }
            let literal = &r["Literal"];
            if let Some(n) = literal["U64"].as_str() {
                return Ok(Arg::Scalar(n.parse::<u64>().map_err(|_| "invalid u64 route literal")?));
            }
            if let Some(n) = literal["U32"].as_u64() { return Ok(Arg::Scalar(u32::try_from(n).map_err(|_| "invalid u32 route literal")? as u64)); }
            if let Some(b) = literal["Bool"].as_bool() { return Ok(Arg::Scalar(b as u64)); }
            Err("invalid compiled route reference".into())
        };
        for stage in route["stages"].as_array().ok_or("missing route stages")? {
            let bridge = Instant::now();
            let arguments = stage["arguments"].as_array().ok_or("missing route arguments")?.iter().map(|r|resolve(r,&values)).collect::<Result<Vec<_>>>()?;
            let bridge_ms = bridge.elapsed().as_secs_f64()*1000.;
            let mode = match (stage["backend"].as_str(), stage["domain"].as_str()) {
                (Some("c"), Some("cpu")) => Backend::Cpu,
                (Some("gpu"), Some("gpu")) => Backend::Gpu,
                _ => return Err("unsupported compiled placement".into()),
            };
            let called = Instant::now();
            let function = self.manifest["functions"].as_array().ok_or("missing functions")?.iter().find(|f|f["name"]==stage["function"]).ok_or("missing stage function")?.clone();
            let result = self.call_prepared(&function, &arguments, mode)?;
            let id = stage["id"].as_str().ok_or("missing stage id")?;
            values.insert(id.into(), Arg::Scalar(if function["result"]=="bool" {result["value"].as_bool().ok_or("invalid Bool result")? as u64} else {result["value"].as_u64().ok_or("invalid word result")?}));
            evidence.push(json!({"id":id,"function":stage["function"],"requested":stage["domain"],"backend":result["backend"],"reason":result["reason"],"bridge_ms":bridge_ms,"execution_ms":called.elapsed().as_secs_f64()*1000.}));
        }
        let value = match resolve(&route["output"],&values)? { Arg::Scalar(n) => n, _ => return Err("route result must be scalar".into()) };
        Ok(json!({"value":if result_bool {json!(value!=0)} else {json!(value)},"stages":evidence,"total_ms":start.elapsed().as_secs_f64()*1000.,"scope":"pure values; immutable inputs shared across stages; scalar stage results materialise on host; physical ABI and backends trusted"}))
    }
    fn prepare(&mut self, f: &Value, args: &[Arg]) -> Result<()> {
        if let Some(e) = &self.gpu_error {
            return Err(e.clone());
        }
        if let Some(e) = self.disabled.get(&f["index"].as_u64().unwrap()) {
            return Err(e.clone());
        }
        if self.gpu.is_none() {
            match Gpu::new() {
                Ok(gpu) => self.gpu = Some(gpu),
                Err(e) => {
                    self.gpu_error = Some(e.clone());
                    return Err(e);
                }
            }
        }
        self.gpu.as_mut().unwrap().prepare(f, args)
    }
    pub fn call(&mut self, name: &str, args: &Value, backend: Backend) -> Result<Value> {
        let f = self.manifest["functions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["name"] == name)
            .ok_or_else(|| format!("unknown function {name}"))?
            .clone();
        let args = parse_args(&f, args)?;
        self.call_prepared(&f, &args, backend)
    }
    fn call_prepared(&mut self, f: &Value, args: &[Arg], backend: Backend) -> Result<Value> {
        let output = |value:u64,backend:&str,reason:String,profile:Option<&Value>|json!({"value":if f["result"]=="bool"{json!(value!=0)}else{json!(value)},"backend":backend,"reason":reason,"profile":profile});
        if backend==Backend::GpuRequired && f["gpu"].is_null(){return Err(f["cpu_reason"].as_str().unwrap_or("CPU-only function").into());}
        if backend == Backend::Cpu || f["gpu"].is_null() {
            return Ok(output(
                cpu(&f, &args),
                "cpu",
                if backend == Backend::Cpu {
                    "explicit CPU selection".into()
                } else {
                    f["cpu_reason"]
                        .as_str()
                        .unwrap_or("CPU-only function")
                        .into()
                },
                None,
            ));
        }
        let scope=json!([self.manifest["core_sha256"],self.manifest["cpu_c_sha256"],self.manifest["javascript_sha256"],f["name"]]).to_string();
        let inputs=args.iter().enumerate().map(|(i,a)|match a{Arg::Scalar(_)=>json!({"list":false,"count":1,"bytes":if f["params"][i]["type"]=="u64"{8}else{4}}),Arg::U32List(xs)=>json!({"list":true,"count":xs.len(),"bytes":xs.len()*4}),Arg::U64List(xs)=>json!({"list":true,"count":xs.len(),"bytes":xs.len()*8})}).collect::<Vec<_>>();
        let features=ink_abi::selection::features(&json!({"inputs":inputs,"outputs":[1],"stages":[{"dispatches":1}],"iterations":1}))?;
        let decision=ink_abi::selection::decide(&self.selection_options,&json!({"scope":scope,"boundary":"captured-owned","features":features}),&["cpu".into(),"gpu".into()])?;
        if backend==Backend::Auto&&decision["backend"]=="cpu"{return Ok(output(cpu(f,args),"cpu",decision["reason"].as_str().unwrap().into(),Some(&decision)));}
        let selected=(||{self.prepare(f,args)?;self.gpu.as_mut().unwrap().execute(f,args)})();
        match selected{
            Ok(value)=>Ok(output(value,"gpu",if matches!(backend,Backend::Gpu|Backend::GpuRequired){"explicit GPU selection".into()}else{decision["reason"].as_str().unwrap().into()},Some(&decision))),
            Err(e)=>{if backend==Backend::GpuRequired{return Err(e);}if !e.contains("limits")&&!e.contains("budget"){self.disabled.insert(f["index"].as_u64().unwrap(),e.clone());}Ok(output(cpu(f,args),"cpu",format!("GPU fallback: {e}"),None))}
        }

    }
}
impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}
mod device;
use device::Gpu;
