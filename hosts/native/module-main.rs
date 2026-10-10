fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1)
    }
}
fn run() -> Result<(), String> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let script = args.first().ok_or("expected script JSON path")?;
    let script: serde_json::Value =
        serde_json::from_slice(&std::fs::read(script).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let mut state = compiled_state::State::new();
    let mut engines = GpuEngines::new();
    let mut output = vec![];
    for step in script.as_array().ok_or("expected array")? {
        if let Some(bytes) = step["restore"].as_array() {
            let bytes = bytes
                .iter()
                .map(|v| {
                    v.as_u64()
                        .and_then(|n| u8::try_from(n).ok())
                        .ok_or("snapshot byte")
                })
                .collect::<Result<Vec<_>, _>>()?;
            state = compiled_state::State::restore(&bytes)?;
            continue;
        }
        if let Some(ack) = step["ack"].as_array() {
            state.acknowledge_through(
                ack[0].as_u64().ok_or("commit")?,
                ack[1].as_u64().ok_or("position")?,
            );
            continue;
        }
        let result = if let Some(name) = step["pure"].as_str() {
            let backend = step["backend"].as_str().unwrap_or("auto");
            if !["auto", "cpu", "gpu"].contains(&backend) {
                return Err("invalid native backend".into());
            }
            if backend == "cpu" {
                state
                    .call_json(name, &step["args"])
                    .map(|v| serde_json::json!({"value":v,"backend":"cpu"}))
            } else {
                engines.call(name, &step["args"], backend).or_else(|e| {
                    state
                        .call_json(name, &step["args"])
                        .map(|v| serde_json::json!({"value":v,"backend":"cpu","reason":e}))
                })
            }
        } else {
            state.invoke_json(step["call"].as_str().ok_or("missing call")?, &step["args"])
        };
        output.push(match result {
            Ok(v) => v,
            Err(e) => serde_json::json!({"host_error":e}),
        });
    }
    println!(
        "{}",
        serde_json::to_string(&output).map_err(|e| e.to_string())?
    );
    Ok(())
}
