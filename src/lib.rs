//! Backend-neutral execution assembly, scheduling, saved-model decisions and fallback.
use ink_core::{core, syntax::*, LangResult};
use ink_lowering_c::native;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{fs, path::Path};
fn put(dir: &Path, name: &str, bytes: impl AsRef<[u8]>) -> LangResult<()> {
    fs::write(dir.join(name), bytes).map_err(|e| e.to_string())
}
fn scalar_cpu(p: &Program) -> LangResult<String> {
    let mut c = native::emit(p)?;
    c.push_str("\nuint64_t ink_cpu_call(uint32_t function,const void *const *lists,const size_t *lengths,const uint64_t *scalars){\n(void)lists;(void)lengths;(void)scalars;\nswitch(function){\n");
    for (index, f) in p.functions.iter().enumerate() {
        let mut args = Vec::new();
        for (i, (_, t)) in f.params.iter().enumerate() {
            args.push(match t {
                Type::List(inner) => format!(
                    "(const {}*)lists[{i}], lengths[{i}]",
                    if **inner == Type::U32 {
                        "uint32_t"
                    } else {
                        "uint64_t"
                    }
                ),
                Type::U32 => format!("(uint32_t)scalars[{i}]"),
                Type::U64 => format!("scalars[{i}]"),
                Type::Bool => format!("(bool)scalars[{i}]"),
                _ => return Err("unsupported CPU ABI parameter".into()),
            });
        }
        c.push_str(&format!(
            "case {index}:return (uint64_t)lang_fn_{}({});\n",
            f.name,
            args.join(",")
        ));
    }
    c.push_str("default:lang_fail();return 0;\n}\n}\n");
    Ok(c)
}
/// Physical host ABI selection follows the GPU package's emitted capabilities.
/// This does not change language semantics or select an optimisation law.
pub fn uses_packed_compute(p: &Program) -> bool {
    ink_lowering_gpu::compute::needed(p)
}
pub fn emit_selected(
    selected: &ink_core::optimisation::CheckedSelection,
    dir: &Path,
) -> LangResult<()> {
    match assemble(selected.module(), dir, None, None) {
        Ok(()) => Ok(()),
        Err(_) => module::emit_selected(selected, dir),
    }
}
pub fn emit_gpu(p: &Program, dir: &Path) -> LangResult<()> {
    let checked = core::CheckedModule::from_source(p.clone())?;
    match assemble(&checked, dir, None, None) {
        Ok(()) => Ok(()),
        Err(_) => module::emit(&checked, dir),
    }
}
pub fn emit_planned(plan:&ink_core::planning::CheckedPlan,dir:&Path)->LangResult<()> {
    plan.require_source_storage_capability(false,false)?;
    match assemble(plan.module(),dir,None,plan.resources()){Ok(())=>Ok(()),Err(_)=>module::emit_planned(plan,dir)}
}
pub fn emit_routed_planned(plan: &ink_core::planning::CheckedPlan, dir: &Path) -> LangResult<()> {
    plan.require_source_storage_capability(false,false)?;
    routed(plan.route().ok_or("resource emission requires a route")?, dir, plan.resources())
}
pub fn emit_routed(route: &ink_core::source_routing::CheckedRouting, dir: &Path) -> LangResult<()> {
    routed(route, dir, None)
}
fn routed(route: &ink_core::source_routing::CheckedRouting, dir: &Path,
    resources: Option<&ink_core::resources::CheckedResources>) -> LangResult<()> {
    let component;
    let route=if route.module().pure_program().is_err() {
        component=route.pure_component()?;&component
    }else{route};
    if uses_packed_compute(route.module().pure_program()?) {
        return Err("routed scalar ABI does not support packed compute plans".into());
    }
    for s in &route.package().stages {
        if !matches!(
            (s.backend.as_str(), s.domain.as_str()),
            ("c", "cpu") | ("gpu", "gpu")
        ) {
            return Err("mixed runtime supports c/cpu and gpu/gpu placements".into());
        }
    }
    assemble(route.module(), dir, Some(route), resources)
}
fn assemble(
    checked: &core::CheckedModule,
    dir: &Path,
    route: Option<&ink_core::source_routing::CheckedRouting>,
    resources: Option<&ink_core::resources::CheckedResources>,
) -> LangResult<()> {
    let p = checked.pure_program()?;
    let compute = uses_packed_compute(p);
    let c = if compute {
        ink_lowering_c::compute::emit_c(p)?
    } else {
        scalar_cpu(p)?
    };
    let javascript = ink_lowering_js::lower(checked)?;
    let mut emitted = ink_lowering_gpu::gpu::lower(checked)?;
    emitted.manifest["javascript"] = javascript.manifest.clone();
    emitted.manifest["javascript_sha256"] = json!(format!(
        "{:x}",
        Sha256::digest(javascript.javascript.as_bytes())
    ));
    if compute {
        let hosts = json!([
            ["ink-gpu.mjs", include_str!("../hosts/ink-compute.mjs")],
            ["pipeline-contract.mjs", include_str!("../hosts/pipeline-contract.mjs")],
            ["backend-selection.mjs", include_str!("../hosts/backend-selection.mjs")],
            ["prepared-wasm.mjs", include_str!("../hosts/prepared-wasm.mjs")],
            ["ink-abi.mjs", ink_abi::PACKED_JS_SOURCE],
            ["ink-selection.mjs", ink_abi::SELECTION_JS_SOURCE],
            ["ink-selection.wasm.sha256", &format!("{:x}", Sha256::digest(ink_abi::SELECTION_WASM))],
            ["pipeline-workers.mjs", include_str!("../hosts/pipeline-workers.mjs")],
            ["pipeline-worker.mjs", include_str!("../hosts/pipeline-worker.mjs")],
            ["ink-device.mjs", ink_lowering_gpu::COMPUTE_DEVICE_JS]
        ]);
        emitted.manifest["runtime_host_sha256"] = json!(format!("{:x}", Sha256::digest(hosts.to_string().as_bytes())));
        emitted.manifest["destination_abi"] = json!({
            "name":"ink-packed-into-v1", "symbol":"ink_compute_into",
            "count":"words32", "ownership":"caller-owned",
            "capacity":"trusted", "alignment":8,
            "aliasing":"destination-disjoint-from-all-reachable-inputs",
            "scratch_lifetime":"call-mark-rewind",
            "prepared_instance":"exclusive"
        });
    }
    emitted.manifest["cpu_c_sha256"] = json!(format!("{:x}", Sha256::digest(c.as_bytes())));
    emitted.manifest["runtime_abi"] = json!(if compute {
        "ink-packed-pipeline-v2"
    } else {
        "ink-scalar-pipeline-v1"
    });
    fs::create_dir_all(dir.join("native/src")).map_err(|e| e.to_string())?;
    let manifest = serde_json::to_string_pretty(&emitted.manifest).map_err(|e| e.to_string())?;
    put(dir, "manifest.json", &manifest)?;
    put(dir, "program.mjs", &javascript.javascript)?;
    if compute {
        put(dir, "prepared-wasm.mjs", include_str!("../hosts/prepared-wasm.mjs"))?;
        put(dir, "pipeline-profile.mjs", include_str!("../hosts/pipeline-profile.mjs"))?;
        put(dir, "pipeline-contract.mjs", include_str!("../hosts/pipeline-contract.mjs"))?;
        put(dir, "pipeline-workers.mjs", include_str!("../hosts/pipeline-workers.mjs"))?;
        put(dir, "pipeline-worker.mjs", include_str!("../hosts/pipeline-worker.mjs"))?;
    }
    put(dir, "usage-profile.mjs", include_str!("../hosts/usage-profile.mjs"))?;
    put(dir, "ink-selection.mjs", ink_abi::SELECTION_JS_SOURCE)?;
    put(dir, "ink-selection.wasm", ink_abi::SELECTION_WASM)?;
    put(dir, "ink-abi.mjs", ink_abi::PACKED_JS_SOURCE)?;
    put(
        dir,
        "backend-selection.mjs",
        include_str!("../hosts/backend-selection.mjs"),
    )?;
    put(dir, "cpu.c", &c)?;
    put(dir, "native/cpu.c", &c)?;
    put(
        dir,
        "ink-gpu.mjs",
        if compute {
            include_str!("../hosts/ink-compute.mjs")
        } else {
            include_str!("../hosts/ink-gpu.mjs")
        },
    )?;
    put(
        dir,
        "ink-device.mjs",
        if compute {
            ink_lowering_gpu::COMPUTE_DEVICE_JS
        } else {
            ink_lowering_gpu::SCALAR_DEVICE_JS
        },
    )?;
    put(
        dir,
        "native/Cargo.toml",
        include_str!("../hosts/native/Cargo.toml"),
    )?;
    put(
        dir,
        "native/Cargo.lock",
        include_str!("../hosts/native/Cargo.lock"),
    )?;
    put(
        dir,
        "native/build.rs",
        include_str!("../hosts/native/build.rs"),
    )?;
    put(
        dir,
        "native/src/backend.rs",
        include_str!("../hosts/native/backend.rs"),
    )?;
    put(
        dir,
        "native/src/lib.rs",
        if compute {
            include_str!("../hosts/native/compute.rs")
        } else {
            include_str!("../hosts/native/scalar.rs")
        },
    )?;
    put(
        dir,
        "native/src/device.rs",
        if compute {
            ink_lowering_gpu::COMPUTE_DEVICE_RS
        } else {
            ink_lowering_gpu::SCALAR_DEVICE_RS
        },
    )?;
    put(
        dir,
        "native/src/main.rs",
        if compute {
            include_str!("../hosts/native/compute-main.rs")
        } else {
            include_str!("../hosts/native/scalar-main.rs")
        },
    )?;
    let resources_json = resources.map(|r| serde_json::to_string(r.plan()).map_err(|e|e.to_string())).transpose()?;
    let route_json = route
        .map(|r| serde_json::to_string(r.package()).map_err(|e| e.to_string()))
        .transpose()?;
    if let Some(s) = &route_json {
        put(dir, "source-route.json", s)?;
        put(
            dir,
            "ink-route.mjs",
            format!(
                "const ROUTE={s};\nconst RESOURCES={};\n{}",
                resources_json.as_deref().unwrap_or("null"),
                include_str!("../hosts/ink-route.mjs")
            ),
        )?;
    } else {
        for n in ["source-route.json", "ink-route.mjs"] {
            match fs::remove_file(dir.join(n)) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.to_string()),
            }
        }
    }
    let mut assets =
        format!("pub const MANIFEST:&str={manifest:?};\npub const SHADERS:&[(&str,&str)]=&[\n");
    for (n, s) in emitted.shaders {
        assets.push_str(&format!("({n:?},{s:?}),\n"));
        put(dir, &n, s)?;
    }
    assets.push_str("];\n");
    assets.push_str(&format!("pub const SOURCE_RESOURCES:Option<&str>={};\n", resources_json.as_ref().map(|s|format!("Some({s:?})")).unwrap_or("None".into())));
    assets.push_str(&match route_json {
        Some(s) => format!("pub const SOURCE_ROUTE:Option<&str>=Some({s:?});\n"),
        None => "pub const SOURCE_ROUTE:Option<&str>=None;\n".into(),
    });
    put(dir, "native/src/assets.rs", assets)
}

pub const STATE_WASM_ABI: &str = ink_native_support::STATE_EXPORTS_RUST_SOURCE;
pub const STATE_WASM_HOST: &str = include_str!("../hosts/state-wasm.mjs");

pub fn emit_durable_hosts(dir: &Path) -> LangResult<()> {
    put(
        dir,
        "durable-state.mjs",
        include_str!("../hosts/durable-state.mjs"),
    )?;
    put(
        dir,
        "durable-file.mjs",
        include_str!("../hosts/durable-file.mjs"),
    )?;
    Ok(())
}

pub const NATIVE_STATE_ADAPTER: &str = include_str!("../hosts/durable-native-adapter.rs");
/// Shared native host implementation for complete and specialised state projects.
pub fn emit_native_state_host(dir: &Path) -> LangResult<()> {
    fs::create_dir_all(dir.join("src")).map_err(|e| e.to_string())?;
    put(dir, "src/durable.rs", include_str!("durable.rs"))?;
    put(dir, "src/state-cli.rs", NATIVE_STATE_CLI)
}

pub mod durable;
pub mod execution;
pub mod module;
pub mod toolchain;

const NATIVE_STATE_CLI: &str = include_str!("../hosts/native/state-cli.rs");

pub const STATE_RUNNER: &str = include_str!("../hosts/native/state-main.rs");
pub const COMPLETE_STATE_RUNNER: &str = include_str!("../hosts/value-main.rs");

/// Assemble a complete literal C program and its shared primitive runtime.
/// C functions/branches/loops execute the program; Rust supplies exact values,
/// storage and portable codecs through an explicit C ABI, without an evaluator.
pub fn emit_c_program(module: &core::CheckedModule, dir: &Path) -> LangResult<()> {
    emit_c_artifact(ink_lowering_c::complete::lower(module)?, dir)
}
pub fn emit_c_program_selected(
    selected: &ink_core::optimisation::CheckedSelection,
    dir: &Path,
) -> LangResult<()> {
    emit_c_artifact(ink_lowering_c::complete::lower_selected(selected)?, dir)
}
pub fn emit_c_program_planned(plan: &ink_core::planning::CheckedPlan, dir: &Path) -> LangResult<()> {
    emit_c_artifact(ink_lowering_c::complete::lower_planned(plan)?,dir)
}
fn emit_c_artifact(emission: ink_lowering_c::complete::Emission, dir: &Path) -> LangResult<()> {
    fs::create_dir_all(dir.join("src")).map_err(|e| e.to_string())?;
    put(dir, "program.c", emission.c)?;
    put(dir, "state-abi.h", ink_abi::STATE_C_HEADER)?;
    put(dir, "state-wasm.mjs", STATE_WASM_HOST)?;
    emit_durable_hosts(dir)?;
    put(
        dir,
        "manifest.json",
        serde_json::to_vec_pretty(&emission.metadata).map_err(|e| e.to_string())?,
    )?;
    put(
        dir,
        "src/metadata.json",
        serde_json::to_vec_pretty(&emission.metadata).map_err(|e| e.to_string())?,
    )?;
    let mut runtime = String::from("#![allow(dead_code)]\nmod portable {\n");
    runtime.push_str(ink_core::SNAPSHOT_WIRE_RUST_SOURCE);
    runtime.push_str("\n}\npub use portable::Limits as SnapshotLimits;\n");
    runtime.push_str("mod transaction {\n");
    runtime.push_str(ink_core::TRANSACTION_RUST_SOURCE);
    runtime.push_str("\n}\n");
    runtime.push_str("mod temporary {\n");
    runtime.push_str(ink_native_support::OWNED_VALUES_RUST_SOURCE);
    runtime.push_str("\n}\nuse temporary::{OwnedBuffer,OwnedMap};\n");
    runtime.push_str(ink_abi::VALUE_KIND_RUST_SOURCE);
    runtime.push_str(ink_native_support::DYNAMIC_VALUES_RUST_SOURCE);
    runtime.push_str(ink_native_support::NUMERIC_INT_RUST_SOURCE);
    runtime.push_str("\nmod value_wire {\n");
    runtime.push_str(ink_abi::VALUE_WIRE_RUST_SOURCE);
    runtime.push_str("\n}\npub use value_wire::Limits as ValueWireLimits;\n");
    runtime.push_str("pub const INK_WIRE_CONTRACT:&str=include_str!(\"wire.json\");\n");
    put(dir,"src/wire.json",emission.metadata["wire"].to_string())?;
    runtime.push_str(ink_native_support::BINARY_VALUES_RUST_SOURCE);
    runtime.push_str(ink_native_support::DYNAMIC_BINARY_RUST_SOURCE);

    runtime.push_str(STATE_WASM_ABI);
    runtime.push_str(ink_native_support::BINARY_EXPORTS_RUST_SOURCE);
    runtime.push_str(NATIVE_STATE_ADAPTER);
    emit_native_state_host(dir)?;
    put(dir, "src/lib.rs", runtime)?;
    put(dir, "src/main.rs", COMPLETE_STATE_RUNNER)?;
    put(dir, "build.rs", include_str!("../hosts/value-build.rs"))?;
    put(dir,"Cargo.toml","[package]\nname=\"compiled-state\"\nversion=\"0.1.0\"\nedition=\"2021\"\n[lib]\ncrate-type=[\"rlib\",\"cdylib\",\"staticlib\"]\n[dependencies]\nnum-bigint=\"=0.4.8\"\nsha2=\"=0.10.9\"\nserde_json={version=\"=1.0.151\",features=[\"float_roundtrip\"]}\n[build-dependencies]\ncc=\"1\"\n[workspace]\n")?;
    Ok(())
}

/// Assemble caller-supplied literal Rust emission without coupling the runtime
/// package to the Rust emitter. The distribution owns that dependency.
pub fn emit_rust_program(code: &str, dir: &Path) -> LangResult<()> {
    fs::create_dir_all(dir.join("src")).map_err(|e| e.to_string())?;
    let mut code = code.to_owned();
    code.push_str(STATE_WASM_ABI);
    if code.contains("pub const INK_WIRE_CONTRACT:"){code.push_str(ink_native_support::BINARY_EXPORTS_RUST_SOURCE);}
    code.push_str(NATIVE_STATE_ADAPTER);
    emit_native_state_host(dir)?;
    put(dir, "src/lib.rs", code)?;
    put(dir, "src/main.rs", COMPLETE_STATE_RUNNER)?;
    put(dir,"Cargo.toml","[package]\nname=\"compiled-state\"\nversion=\"0.1.0\"\nedition=\"2021\"\n[lib]\ncrate-type=[\"rlib\",\"cdylib\",\"staticlib\"]\n[dependencies]\nnum-bigint=\"=0.4.8\"\nsha2=\"=0.10.9\"\nserde_json={version=\"=1.0.151\",features=[\"float_roundtrip\"]}\n[workspace]\n")?;
    put(dir, "state-wasm.mjs", STATE_WASM_HOST)?;
    emit_durable_hosts(dir)?;
    put(dir, "state-abi.h", ink_abi::STATE_C_HEADER)?;
    Ok(())
}


/// Plain handle/buffer ABI appended to standalone representation applications.
pub const REPRESENTATION_ABI_SOURCE: &str = ink_native_support::REPRESENTATION_EXPORTS_RUST_SOURCE;
/// Distribution supplies the independent planner asset; runtime contains no search.
pub fn install_pipeline_planner(dir: &Path, javascript: &str) -> LangResult<()> {
    let mut manifest: serde_json::Value = serde_json::from_slice(&fs::read(dir.join("manifest.json")).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    if manifest["runtime_abi"] == "ink-packed-pipeline-v2" {
        put(dir, "pipeline-selection.mjs", javascript)?;
        manifest["pipeline_planner_sha256"] = json!(format!("{:x}", Sha256::digest(javascript.as_bytes())));
        let encoded = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
        put(dir, "manifest.json", &encoded)?;
        let assets = fs::read_to_string(dir.join("native/src/assets.rs")).map_err(|e| e.to_string())?;
        let (_, remaining) = assets.split_once('\n').ok_or("invalid native assets")?;
        put(dir, "native/src/assets.rs", format!("pub const MANIFEST:&str={encoded:?};\n{remaining}"))?;
    }
    Ok(())
}

/// Bind runtime recordings to the original source identity across checked rewrites.
/// The manifest still binds execution to its selected module; this separate field
/// is only cohort provenance and grants no semantic authority.
pub fn install_usage_identity(dir: &Path, original: &str) -> LangResult<()> {
    let mut manifest: serde_json::Value=serde_json::from_slice(&fs::read(dir.join("manifest.json")).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    manifest["usage_core_sha256"]=json!(original);
    let encoded=serde_json::to_string_pretty(&manifest).map_err(|e|e.to_string())?;
    put(dir,"manifest.json",&encoded)?;
    let assets=fs::read_to_string(dir.join("native/src/assets.rs")).map_err(|e|e.to_string())?;
    let (_,remaining)=assets.split_once('\n').ok_or("invalid native asset header")?;
    put(dir,"native/src/assets.rs",format!("pub const MANIFEST:&str={encoded:?};\n{remaining}"))
}
