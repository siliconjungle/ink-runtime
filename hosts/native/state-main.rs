
use compiled_state::*;
fn run()->Result<(),String>{
 let mut args=std::env::args().skip(1);let file=args.next().ok_or("expected SCRIPT.json")?;
 let mut restore=None;let mut snapshot_out=None;
 while let Some(flag)=args.next(){let value=args.next().ok_or("missing option value")?;match flag.as_str(){"--restore"=>restore=Some(value),"--snapshot-out"=>snapshot_out=Some(value),_=>return Err(format!("unknown option {flag}"))}}
 let script:serde_json::Value=serde_json::from_slice(&std::fs::read(file).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
 let mut state=if let Some(file)=restore{State::restore(&std::fs::read(file).map_err(|e|e.to_string())?)?}else{State::new()};let mut output=vec![];
 for step in script.as_array().ok_or("expected script array")? {
  output.push(state.invoke_json(step.get("call").and_then(serde_json::Value::as_str).ok_or("missing call")?,step.get("args").ok_or("missing args")?)?);
 }
 if let Some(file)=snapshot_out{std::fs::write(file,state.checkpoint()?).map_err(|e|e.to_string())?;}
 println!("{}",serde_json::to_string_pretty(&output).map_err(|e|e.to_string())?);Ok(())
}
fn main()->std::process::ExitCode{match run(){Ok(())=>std::process::ExitCode::SUCCESS,Err(e)=>{eprintln!("error: {e}");std::process::ExitCode::FAILURE}}}
