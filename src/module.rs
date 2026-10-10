//! Complete module assembly. Each eligible acyclic pure closure can use the
//! packed GPU adapter; values and effects outside that ABI use complete hosts.
use ink_core::{core::CheckedModule, syntax::*, LangResult};
use serde_json::json;
use std::{collections::BTreeSet, fs, path::Path};
fn expr(e: &Expr, calls: &mut BTreeSet<String>, types: &mut BTreeSet<String>) {
    match e {
        Expr::Call(n, a) => {
            calls.insert(n.clone());
            for e in a {
                expr(e, calls, types)
            }
        }
        Expr::Record(n, fs) => {
            types.insert(n.clone());
            for (_, e) in fs {
                expr(e, calls, types)
            }
        }
        Expr::Let(_, t, a, b) => {
            if let Some(t) = t {
                ty(t, types)
            }
            expr(a, calls, types);
            expr(b, calls, types)
        }
        Expr::Binary(_, a, b) => {
            expr(a, calls, types);
            expr(b, calls, types)
        }
        Expr::Neg(a) | Expr::Lambda(_, a) | Expr::Field(a, _) | Expr::Try(a) => {
            expr(a, calls, types)
        }
        Expr::Method(a, _, xs) => {
            expr(a, calls, types);
            for e in xs {
                expr(e, calls, types)
            }
        }
        _ => (),
    }
}
fn ty(t: &Type, names: &mut BTreeSet<String>) {
    match t {
        Type::Named(n) => {
            names.insert(n.clone());
        }
        Type::List(t) | Type::Option(t) | Type::Vector(t, _) => ty(t, names),
        Type::Result(a, b) | Type::Table(a, b) => {
            ty(a, names);
            ty(b, names)
        }
        _ => (),
    }
}
fn closure(p: &Program, name: &str) -> LangResult<CheckedModule> {
    let mut names = BTreeSet::from([name.to_owned()]);
    let mut types = BTreeSet::new();
    loop {
        let before = names.len();
        for f in &p.functions {
            if names.contains(&f.name) {
                for (_, t) in &f.params {
                    ty(t, &mut types)
                }
                ty(&f.result, &mut types);
                expr(&f.body, &mut names, &mut types)
            }
        }
        if before == names.len() {
            break;
        }
    }
    loop {
        let before = types.len();
        for (n, fs) in &p.records {
            if types.contains(n) {
                for (_, t) in fs {
                    ty(t, &mut types)
                }
            }
        }
        if before == types.len() {
            break;
        }
    }
    let mut sub = p.clone();
    sub.functions.retain(|f| names.contains(&f.name));
    sub.records.retain(|n, _| types.contains(n));
    sub.ids.retain(|n| types.contains(n));
    sub.enums.retain(|n, _| types.contains(n));
    sub.states.clear();
    sub.actions.clear();
    sub.keeps.clear();
    sub.events.clear();
    sub.rules.clear();
    CheckedModule::from_source(sub)
}
fn symbols(source: &str, prefix: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1
            }
            let word = &source[start..i];
            if word.starts_with("ink_") || word.starts_with("lang_") {
                out.push_str(prefix)
            }
            out.push_str(word)
        } else {
            let ch = source[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8()
        }
    }
    out
}
pub fn emit(module: &CheckedModule, dir: &Path) -> LangResult<()> {
    let p = module.program();
    fs::create_dir_all(dir.join("native/src")).map_err(|e| e.to_string())?;
    super::emit_c_program(module, &dir.join("cpu"))?;
    let js = ink_lowering_js::lower(module)?;
    super::put(dir, "program.mjs", js.javascript)?;
    super::put(dir, "state-wasm.mjs", super::STATE_WASM_HOST)?;
    super::emit_durable_hosts(dir)?;
    super::put(dir, "ink-gpu.mjs", include_str!("../hosts/ink-module.mjs"))?;
    let mut functions = vec![];
    let mut mods = String::new();
    let mut cases = String::new();
    let mut builds = String::new();
    let mut fields = String::new();
    let mut init = String::new();
    for (i, f) in p.functions.iter().enumerate() {
        let sub = closure(p, &f.name)?;
        let folder = format!("gpu/f{i}");
        let result = super::assemble(&sub, &dir.join(&folder), None);
        if result.is_ok() {
            super::put(&dir.join(&folder), "core.json", sub.bytes()?)?;
        }
        let capability = match result {
            Ok(()) => {
                let manifest: serde_json::Value = serde_json::from_slice(
                    &fs::read(dir.join(&folder).join("manifest.json"))
                        .map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                let entry = manifest["functions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|e| e["name"] == f.name)
                    .unwrap();
                let eligible = entry["shader"].is_string() || entry["gpu"].is_object();
                if eligible {
                    let prefix = format!("ink_part{i}_");
                    let cpath = dir.join(&folder).join("native/cpu.c");
                    let c = fs::read_to_string(&cpath).map_err(|e| e.to_string())?;
                    fs::write(&cpath, symbols(&c, &prefix)).map_err(|e| e.to_string())?;
                    let libpath = dir.join(&folder).join("native/src/lib.rs");
                    let lib = fs::read_to_string(&libpath).map_err(|e| e.to_string())?;
                    let lib = lib
                        .replace(
                            "fn ink_compute_call(",
                            &format!("fn {prefix}ink_compute_call("),
                        )
                        .replace("ink_compute_call(", &format!("{prefix}ink_compute_call("))
                        .replace(
                            "ink_compute_reset()",
                            &format!("{prefix}ink_compute_reset()"),
                        )
                        .replace("ink_cpu_call(", &format!("{prefix}ink_cpu_call("));
                    fs::write(libpath, lib.replace(&format!("{prefix}{prefix}"), &prefix))
                        .map_err(|e| e.to_string())?;
                    fields.push_str(&format!("part{i}:part{i}::Engine,"));
                    init.push_str(&format!("part{i}:part{i}::Engine::new(),"));
                    mods.push_str(&format!(
                        "#[path=\"../../{folder}/native/src/lib.rs\"] mod part{i};\n"
                    ));
                    cases.push_str(&format!("{:?}=>{{self.part{i}.call(name,args,part{i}::Backend::parse(backend)?) }},\n",f.name));
                    builds.push_str(&format!("cc::Build::new().file(\"../{folder}/native/cpu.c\").opt_level(3).compile(\"part{i}\");\n"));
                    json!({"name":f.name,"bundle":format!("{folder}/"),"gpu":true,"strategy":manifest["collection_strategy"],"fallback_reason":null})
                } else {
                    json!({"name":f.name,"gpu":false,"fallback_reason":entry["fallback_reason"]})
                }
            }
            Err(reason) => json!({"name":f.name,"gpu":false,"fallback_reason":reason}),
        };
        functions.push(capability);
    }
    let manifest = json!({"schema":3,"runtime_abi":"ink-module-v1","core_sha256":module.identity()?,"core_semantics":module.semantics(),"functions":functions,"actions":p.actions.iter().map(|a|json!({"name":a.name,"gpu":false,"fallback_reason":"transactional state, domain errors and ordered event publication execute on the host"})).collect::<Vec<_>>(),"javascript":js.manifest,"cpu":"cpu/compiled_state.wasm","trust":"literal source closures, target emission, host adapters, runtimes and toolchains remain trusted; capability is not a proof or profitability claim"});
    super::put(
        dir,
        "manifest.json",
        serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?,
    )?;
    super::put(dir,"native/Cargo.toml","[package]\nname=\"ink-module-program\"\nversion=\"0.1.0\"\nedition=\"2021\"\n[dependencies]\ncompiled-state={path=\"../cpu\"}\nwgpu=\"=30.0.1\"\npollster=\"0.4\"\nserde_json={version=\"1\",features=[\"float_roundtrip\"]}\n[build-dependencies]\ncc=\"1\"\n[workspace]\n")?;
    super::put(dir, "native/build.rs", format!("fn main(){{{builds}}}"))?;
    let code=format!("{mods}\nstruct GpuEngines{{{fields}}}impl GpuEngines{{fn new()->Self{{Self{{{init}}}}} fn call(&mut self,name:&str,args:&serde_json::Value,backend:&str)->Result<serde_json::Value,String>{{match name{{{cases}_=>Err(\"CPU-only function\".into())}}}}}}\n{}",include_str!("../hosts/native/module-main.rs"));
    super::put(dir, "native/src/main.rs", code)
}
