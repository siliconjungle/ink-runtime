//! Physical toolchain invocation belongs to execution orchestration.
use ink_core::LangResult;
use std::{path::Path, process::Command};
fn target_dir(project:&Path)->LangResult<(std::path::PathBuf,Option<std::fs::File>)> {
    if let Some(path)=std::env::var_os("INK_COMPILED_TARGET_DIR") {
        let path=std::path::PathBuf::from(path);
        std::fs::create_dir_all(&path).map_err(|e|e.to_string())?;
        let lock=std::fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(path.join(".artifact-lock")).map_err(|e|e.to_string())?;
        // Cargo's own lock ends before our copy. Retain a separate lock through
        // publication so another project with the same crate name cannot replace
        // the shared output between successful compilation and copying it.
        lock.lock().map_err(|e|e.to_string())?;
        Ok((path,Some(lock)))
    }else{Ok((project.join("target"),None))}
}
/// Compile a generated Rust CPU fallback project for a browser GPU bundle.
pub fn gpu_cpu_wasm(project: &Path, output: &Path) -> LangResult<()> {
    let (target,_cache_guard)=target_dir(project)?;
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
        .arg(&target)
        .output()
        .map_err(|e| e.to_string())?;
    if !result.status.success() {
        return Err(String::from_utf8_lossy(&result.stderr).into_owned());
    }
    std::fs::copy(
        target.join("wasm32-unknown-unknown/release/ink_packed_cpu.wasm"),
        output,
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
pub fn complete_program(dir: &Path, wasm: bool, output: &Path, cargo: &str) -> LangResult<()> {
    let (target,_cache_guard)=target_dir(dir)?;
    let mut command = Command::new(cargo);
    command
        .arg("build")
        .arg("--release")
        .arg("--manifest-path")
        .arg(dir.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(&target);
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
        target.join("wasm32-unknown-unknown/release/compiled_state.wasm")
    } else {
        target.join(format!(
            "release/compiled-state{}",
            std::env::consts::EXE_SUFFIX
        ))
    };
    std::fs::copy(source, output).map_err(|e| e.to_string())?;
    Ok(())
}
