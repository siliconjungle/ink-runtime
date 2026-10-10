use compiled_state::State;
use serde_json::Value;
fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let file = args.next().ok_or("expected SCRIPT.json")?;
    let mut restore = None;
    let mut snapshot_out = None;
    while let Some(flag) = args.next() {
        let value = args.next().ok_or("missing option value")?;
        match flag.as_str() {
            "--restore" => restore = Some(value),
            "--snapshot-out" => snapshot_out = Some(value),
            _ => return Err(format!("unknown option {flag}")),
        }
    }
    let script: Value = serde_json::from_slice(&std::fs::read(file).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let mut state = if let Some(file) = restore {
        State::restore(&std::fs::read(file).map_err(|e| e.to_string())?)?
    } else {
        State::new()
    };
    let mut output = vec![];
    for step in script.as_array().ok_or("expected script array")? {
        if let Some(bytes) = step.get("restore") {
            let bytes: Vec<u8> =
                serde_json::from_value(bytes.clone()).map_err(|e| e.to_string())?;
            state = State::restore(&bytes)?;
            continue;
        }
        if let Some(ack) = step.get("acknowledge").or_else(|| step.get("ack")) {
            state.acknowledge_through(
                ack[0].as_u64().ok_or("commit")?,
                ack[1].as_u64().ok_or("position")?,
            );
            continue;
        }
        let value = if let Some(name) = step.get("pure").and_then(Value::as_str) {
            state.call_json(name, &step["args"])
        } else {
            state.invoke_json(step["call"].as_str().ok_or("missing call")?, &step["args"])
        };
        output.push(match value {
            Ok(v) => v,
            Err(e) => serde_json::json!({"host_error":e}),
        });
    }
    if let Some(file) = snapshot_out {
        std::fs::write(file, state.checkpoint()?).map_err(|e| e.to_string())?;
    }
    println!(
        "{}",
        serde_json::to_string(&output).map_err(|e| e.to_string())?
    );
    Ok(())
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
