//! Physical toolchain invocation belongs to execution orchestration.
use ink_core::{core::CheckedModule, LangResult};
use std::{path::Path, process::Command};
pub fn wasm(module: &CheckedModule, c: &Path, output: &Path, zig: &str) -> LangResult<()> {
    wasm_with_abi(module,c,output,zig,ink_core::compute_values::needed(module.pure_program()?))
}
pub fn wasm_with_abi(module: &CheckedModule, c: &Path, output: &Path, zig: &str, packed:bool) -> LangResult<()> {
    let target = ink_lowering_wasm::lower_with_abi(module,packed)?;
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
