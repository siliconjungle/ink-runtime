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
    emit_gpu(selected.module().pure_program()?, dir)
}
pub fn emit_gpu(p: &Program, dir: &Path) -> LangResult<()> {
    assemble(&core::CheckedModule::from_source(p.clone())?, dir, None)
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

pub mod execution;
pub mod toolchain;

pub const STATE_RUNNER: &str = include_str!("../hosts/native/state-main.rs");
