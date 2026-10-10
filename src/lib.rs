//! Backend-neutral execution assembly, scheduling, profiling and fallback.
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
    match assemble(selected.module(), dir, None) {
        Ok(()) => Ok(()),
        Err(_) => module::emit_selected(selected, dir),
    }
}
pub fn emit_gpu(p: &Program, dir: &Path) -> LangResult<()> {
    let checked = core::CheckedModule::from_source(p.clone())?;
    match assemble(&checked, dir, None) {
        Ok(()) => Ok(()),
        Err(_) => module::emit(&checked, dir),
    }
}
pub fn emit_routed(route: &ink_core::source_routing::CheckedRouting, dir: &Path) -> LangResult<()> {
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
    assemble(route.module(), dir, Some(route))
}
fn assemble(
    checked: &core::CheckedModule,
    dir: &Path,
    route: Option<&ink_core::source_routing::CheckedRouting>,
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
    if compute {
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
    }
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
    let route_json = route
        .map(|r| serde_json::to_string(r.package()).map_err(|e| e.to_string()))
        .transpose()?;
    if let Some(s) = &route_json {
        put(dir, "source-route.json", s)?;
        put(
            dir,
            "ink-route.mjs",
            format!(
                "const ROUTE={s};\n{}",
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
    assets.push_str(&match route_json {
        Some(s) => format!("pub const SOURCE_ROUTE:Option<&str>=Some({s:?});\n"),
        None => "pub const SOURCE_ROUTE:Option<&str>=None;\n".into(),
    });
    put(dir, "native/src/assets.rs", assets)
}

pub const STATE_WASM_ABI: &str = include_str!("../hosts/state-wasm-support.rs");
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
fn emit_c_artifact(emission: ink_lowering_c::complete::Emission, dir: &Path) -> LangResult<()> {
    fs::create_dir_all(dir.join("src")).map_err(|e| e.to_string())?;
    put(dir, "program.c", emission.c)?;
    put(dir, "state-abi.h", include_str!("../hosts/state-abi.h"))?;
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
    runtime.push_str(ink_core::TEMPORARY_RUST_SOURCE);
    runtime.push_str("\n}\nuse temporary::{OwnedBuffer,OwnedMap};\n");
    runtime.push_str(include_str!("../hosts/value-abi.rs"));
    runtime.push_str(STATE_WASM_ABI);
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
    code.push_str(NATIVE_STATE_ADAPTER);
    emit_native_state_host(dir)?;
    put(dir, "src/lib.rs", code)?;
    put(dir, "src/main.rs", COMPLETE_STATE_RUNNER)?;
    put(dir,"Cargo.toml","[package]\nname=\"compiled-state\"\nversion=\"0.1.0\"\nedition=\"2021\"\n[lib]\ncrate-type=[\"rlib\",\"cdylib\",\"staticlib\"]\n[dependencies]\nnum-bigint=\"=0.4.8\"\nsha2=\"=0.10.9\"\nserde_json={version=\"=1.0.151\",features=[\"float_roundtrip\"]}\n[workspace]\n")?;
    put(dir, "state-wasm.mjs", STATE_WASM_HOST)?;
    emit_durable_hosts(dir)?;
    put(dir, "state-abi.h", include_str!("../hosts/state-abi.h"))?;
    Ok(())
}
