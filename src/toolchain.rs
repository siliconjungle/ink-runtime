//! Physical toolchain invocation belongs to execution orchestration.
use ink_core::LangResult;
use std::{path::Path, process::Command};
/// Compile a generated Rust CPU fallback project for a browser GPU bundle.
pub fn gpu_cpu_wasm(project: &Path, output: &Path) -> LangResult<()> {
    let result = Command::new(std::env::var("INK_CARGO").unwrap_or_else(|_| "cargo".into()))
        .args([
            "build",
            "--release",
            "--lib",
            "--target",
            ink_lowering_wasm::RUST_TARGET,
            "--manifest-path",
        ])
        .arg(project.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(project.join("target"))
        .output()
        .map_err(|e| e.to_string())?;
    if !result.status.success() {
        return Err(String::from_utf8_lossy(&result.stderr).into_owned());
    }
    std::fs::copy(
        project.join("target/wasm32-unknown-unknown/release/ink_packed_cpu.wasm"),
        output,
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
pub fn complete_program(dir: &Path, wasm: bool, output: &Path, cargo: &str) -> LangResult<()> {
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
