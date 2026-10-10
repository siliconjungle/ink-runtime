include!("state-cli.rs");
fn run() -> Result<(), String> {
    let mut engines = GpuEngines::new();
    run_with_pure(|state, step| {
        let name = step["pure"].as_str().ok_or("missing pure")?;
        let backend = step["backend"].as_str().unwrap_or("auto");
        if !["auto", "cpu", "gpu"].contains(&backend) {
            return Err("invalid native backend".into());
        }
        if backend == "cpu" {
            state
                .pure(name, &step["args"])
                .map(|v| serde_json::json!({"value":v,"backend":"cpu"}))
        } else {
            engines.call(name, &step["args"], backend).or_else(|e| {
                state
                    .pure(name, &step["args"])
                    .map(|v| serde_json::json!({"value":v,"backend":"cpu","reason":e}))
            })
        }
    })
}
fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
