//! Physical toolchain invocation belongs to execution orchestration.
use ink_core::{core::CheckedModule, LangResult};
use std::{path::Path, process::Command};
pub fn wasm(module: &CheckedModule, c: &Path, output: &Path, zig: &str) -> LangResult<()> {
    wasm_with_abi(
        module,
        c,
        output,
        zig,
        ink_core::compute_values::needed(module.pure_program()?),
    )
}
pub fn wasm_with_abi(
    module: &CheckedModule,
    c: &Path,
    output: &Path,
    zig: &str,
    packed: bool,
) -> LangResult<()> {
    let target = ink_lowering_wasm::lower_with_abi(module, packed)?;
    let mut command = Command::new(zig);
    command.args([
        "cc",
        "-target",
        ink_lowering_wasm::TARGET,
        "-O3",
        "-msimd128",
        "-std=c11",
        "-ffreestanding",
        "-nostdlib",
        "-Wl,--no-entry",
        "-Wl,--export=__heap_base",
        "-Wl,--export-memory",
    ]);
    for symbol in target.exports {
        command.arg(format!("-Wl,--export={symbol}"));
    }
    let result = command
        .arg(c)
        .arg("-o")
        .arg(output)
        .output()
        .map_err(|e| format!("{zig}: {e}"))?;
    if !result.status.success() {
        return Err(String::from_utf8_lossy(&result.stderr).into_owned());
    }
    Ok(())
}

pub fn complete_program(
    dir: &Path,
    wasm: bool,
    output: &Path,
    cargo: &str,
    zig: Option<&str>,
) -> LangResult<()> {
    let mut command = Command::new(cargo);
    command
        .arg("build")
        .arg("--release")
        .arg("--manifest-path")
        .arg(dir.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(dir.join("target"));
    if wasm {
        command.args(["--lib", "--target", ink_lowering_wasm::RUST_TARGET]);
        if dir.join("program.c").exists() {
            if let Some(zig) = zig {
                let compiler = dir.join("zig-cc");
                let path = Path::new(zig)
                    .canonicalize()
                    .map_err(|e| format!("{zig}: {e}"))?;
                let quoted = format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"));
                std::fs::write(&compiler, format!("#!/bin/sh\nexec {quoted} cc \"$@\"\n"))
                    .map_err(|e| e.to_string())?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&compiler, std::fs::Permissions::from_mode(0o755))
                        .map_err(|e| e.to_string())?;
                }
                let archiver = dir.join("zig-ar");
                std::fs::write(&archiver, format!("#!/bin/sh\nexec {quoted} ar \"$@\"\n"))
                    .map_err(|e| e.to_string())?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&archiver, std::fs::Permissions::from_mode(0o755))
                        .map_err(|e| e.to_string())?;
                }
                command
                    .env(
                        "INK_CC",
                        compiler.canonicalize().map_err(|e| e.to_string())?,
                    )
                    .env("INK_CC_KIND", "zig")
                    .env(
                        "INK_AR",
                        archiver.canonicalize().map_err(|e| e.to_string())?,
                    );
            }
        }
    } else {
        command.arg("--bin").arg("compiled-state");
    }
    let result = command.output().map_err(|e| format!("{cargo}: {e}"))?;
    if !result.status.success() {
        return Err(String::from_utf8_lossy(&result.stderr).into_owned());
    }
    let source = if wasm {
        dir.join("target/wasm32-unknown-unknown/release/compiled_state.wasm")
    } else {
        dir.join(format!(
            "target/release/compiled-state{}",
            std::env::consts::EXE_SUFFIX
        ))
    };
    std::fs::copy(source, output).map_err(|e| e.to_string())?;
    Ok(())
}
