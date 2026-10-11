//! Bounded JSON-lines host transport for separately emitted target executors.
use ink_core::{definition::CheckedDefinition,logic::{Context,Term},LangResult};
use serde_json::{json,Value};
use std::{io::{BufRead,BufReader,Write},process::{Child,ChildStdin,ChildStdout,Command,Stdio},sync::Mutex};
struct Process {child:Child,input:ChildStdin,output:BufReader<ChildStdout>}
pub struct Executor {process:Mutex<Process>}
pub fn pack(value:&Term)->LangResult<Value> {
 enum Work<'a>{Read(&'a Term,usize),Build(&'a str,usize,usize)}
 let mut work=vec![Work::Read(value,0)];let mut nodes=Vec::new();let mut roots=Vec::new();
 while let Some(w)=work.pop(){match w {
  Work::Read(t,depth)=>{if depth>8192||nodes.len()+work.len()>100000{return Err("flat value limit".into());}match t{
   Term::Bool(x)=>{roots.push(nodes.len());nodes.push(json!(["Bool",x]));},Term::U64(x)=>{roots.push(nodes.len());nodes.push(json!(["U64",x.to_string()]));},
   Term::Construct{datatype,constructor,arguments}=>{if arguments.len()>64{return Err("flat arity limit".into());}work.push(Work::Build(datatype,*constructor,arguments.len()));work.extend(arguments.iter().rev().map(|x|Work::Read(x,depth+1)));},_=>return Err("flat non-ground value".into())}},
  Work::Build(id,tag,n)=>{let arguments=roots.split_off(roots.len()-n);roots.push(nodes.len());nodes.push(json!(["Data",id,tag,arguments]));}
 }}if nodes.len()>100000{return Err("flat node limit".into());}Ok(json!(nodes))
}
pub fn unpack(value:Value)->LangResult<Term> {
 let xs=value.as_array().ok_or("flat nodes must be array")?;if xs.is_empty()||xs.len()>100000{return Err("flat node limit".into());}
 let mut values:Vec<Option<(Term,usize)>>=Vec::new();
 for x in xs {let node=x.as_array().ok_or("flat node array")?;let (term,depth)=match node.first().and_then(Value::as_str){
  Some("Bool") if node.len()==2=>(Term::Bool(node[1].as_bool().ok_or("flat Bool")?),0),
  Some("U64") if node.len()==2=>(Term::U64(node[1].as_str().ok_or("flat U64")?.parse().map_err(|_|"flat U64 range")?),0),
  Some("Data") if node.len()==4=>{let datatype=node[1].as_str().ok_or("flat datatype")?.to_string();if datatype.len()!=64{return Err("flat datatype identity".into());}let constructor=usize::try_from(node[2].as_u64().ok_or("flat tag")?).map_err(|_|"flat tag range")?;let refs=node[3].as_array().ok_or("flat fields")?;if refs.len()>64{return Err("flat arity limit".into());}let mut arguments=Vec::new();let mut depth=0;
   for i in refs {let index=usize::try_from(i.as_u64().ok_or("flat reference")?).map_err(|_|"flat reference range")?;let (term,d)=values.get_mut(index).and_then(Option::take).ok_or("flat alias or forward reference")?;depth=depth.max(d+1);arguments.push(term);}if depth>8192{return Err("flat depth limit".into());}(Term::Construct{datatype,constructor,arguments},depth)},_=>return Err("flat node kind".into())};values.push(Some((term,depth)));
 }
 let result=values.pop().and_then(|x|x).ok_or("flat root")?.0;if values.iter().any(Option::is_some){return Err("flat unused node".into());}Ok(result)
}
impl Executor {
 pub fn spawn(command:&mut Command)->LangResult<Self>{
  let mut child=command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn().map_err(|e|e.to_string())?;
  let input=child.stdin.take().ok_or("target stdin")?;let output=BufReader::new(child.stdout.take().ok_or("target stdout")?);
  Ok(Self{process:Mutex::new(Process{child,input,output})})
 }
}
impl Drop for Executor {fn drop(&mut self){if let Ok(p)=self.process.get_mut(){let _=p.child.kill();let _=p.child.wait();}}}
impl crate::representation::Executor for Executor {
 fn execute(&self,d:&CheckedDefinition,_context:&Context,function:&str,args:Vec<Term>)->LangResult<Term>{
  let identity=ink_core::registry::identity(&("ink-sealed-executable-v1",d.bundle_sha256(),d.exports(),d.choices()))?;
  let packed=args.iter().map(pack).collect::<LangResult<Vec<_>>>()?;
  let request=json!({"identity":identity,"function":function,"args":packed});
  let bytes=serde_json::to_vec(&request).map_err(|e|e.to_string())?;if bytes.len()>16000000{return Err("target input byte limit".into());}
  let mut p=self.process.lock().map_err(|_|"target process lock")?;p.input.write_all(&bytes).and_then(|_|p.input.write_all(b"\n")).and_then(|_|p.input.flush()).map_err(|e|e.to_string())?;
  let mut bytes=Vec::new();loop {let available=p.output.fill_buf().map_err(|e|e.to_string())?;if available.is_empty(){return Err("target executor closed output".into());}let n=available.iter().position(|b|*b==b'\n').map(|n|n+1).unwrap_or(available.len());if bytes.len()+n>16000000{return Err("target output byte limit".into());}bytes.extend_from_slice(&available[..n]);let done=available[n-1]==b'\n';p.output.consume(n);if done{break;}}
  let response:Value=serde_json::from_slice(&bytes).map_err(|e|e.to_string())?;
  if let Some(error)=response.get("error").and_then(Value::as_str){return Err(error.into());}
  unpack(response["result"].clone())
 }
}

#[cfg(test)]
mod tests {
 use super::*;
 #[test] fn flat_transport_handles_realistic_spines_and_rejects_invalid_graphs(){
  let mut term=Term::Construct{datatype:"a".repeat(64),constructor:0,arguments:vec![]};
  for n in 0..2048{term=Term::Construct{datatype:"a".repeat(64),constructor:1,arguments:vec![Term::U64(n),term]};}
  assert_eq!(unpack(pack(&term).unwrap()).unwrap(),term);
  for wire in [json!([]),json!([["Data","list",1,[0]]]),json!([["Bool",true],["Data","list",1,[0,0]]]),json!([["Bool",true],["Bool",false]]),json!([["U64","18446744073709551616"]])] {assert!(unpack(wire).is_err());}
 }
}
