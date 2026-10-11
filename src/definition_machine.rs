//! Literal bounded execution, separate from proof normalisation budgets.
use ink_core::{definition::CheckedDefinition,logic::{Declaration,Sort,Term},LangResult};
use std::{collections::BTreeMap,sync::Arc};
#[derive(Clone)]
pub enum Value { Bool(bool), U64(u64), Data(String,usize,Arc<Vec<Value>>) }
pub struct Machine { declarations:BTreeMap<String,Declaration>,choices:BTreeMap<String,String>,exports:Vec<String>,pub identity:String }
impl Value {
 pub fn from_term(t:&Term)->LangResult<Self>{
  enum Work<'a>{Read(&'a Term,usize),Build(&'a str,usize,usize)}
  let mut work=vec![Work::Read(t,0)];let mut values=Vec::new();let mut nodes=0;
  while let Some(w)=work.pop(){match w{
   Work::Read(t,depth)=>{nodes+=1;if nodes>100000||depth>8192{return Err("machine input limit".into());}match t{Term::Bool(x)=>values.push(Self::Bool(*x)),Term::U64(x)=>values.push(Self::U64(*x)),Term::Construct{datatype,constructor,arguments}=>{if arguments.len()>64{return Err("machine constructor limit".into());}work.push(Work::Build(datatype,*constructor,arguments.len()));work.extend(arguments.iter().rev().map(|x|Work::Read(x,depth+1)));},_=>return Err("machine non-ground input".into())}},
   Work::Build(id,tag,n)=>{let xs=values.split_off(values.len()-n);values.push(Self::Data(id.into(),tag,Arc::new(xs)));}
  }}values.pop().ok_or("machine empty value".into())
 }
 pub fn term(&self)->LangResult<Term>{
  enum Work<'a>{Read(&'a Value,usize),Build(&'a str,usize,usize)}
  let mut work=vec![Work::Read(self,0)];let mut values=Vec::new();let mut nodes=0;
  while let Some(w)=work.pop(){match w{
   Work::Read(t,depth)=>{nodes+=1;if nodes>100000||depth>8192{return Err("machine output limit".into());}match t{Self::Bool(x)=>values.push(Term::Bool(*x)),Self::U64(x)=>values.push(Term::U64(*x)),Self::Data(id,tag,xs)=>{work.push(Work::Build(id,*tag,xs.len()));work.extend(xs.iter().rev().map(|x|Work::Read(x,depth+1)));}}},
   Work::Build(id,tag,n)=>{let arguments=values.split_off(values.len()-n);values.push(Term::Construct{datatype:id.into(),constructor:tag,arguments});}
  }}values.pop().ok_or("machine empty output".into())
 }
}
type Env=Arc<BTreeMap<String,Value>>;
enum Frame<'a> { Eval(&'a Term,Env,&'a str), If(&'a Term,&'a Term,Env,&'a str), Match(&'a [ink_core::logic::Branch],Env,&'a str), Construct(&'a str,usize,usize), Call(&'a str,usize), Binary(&'a str) }
impl Machine {
 pub fn new(d:&CheckedDefinition)->LangResult<Self>{Ok(Self{declarations:d.executable_declarations()?,choices:d.choices().clone(),exports:d.exports().to_vec(),identity:ink_core::registry::identity(&("ink-sealed-executable-v1",d.bundle_sha256(),d.exports(),d.choices()))?})}
 fn function(&self,id:&str)->LangResult<(&[(String,Sort)],&Term)>{match self.declarations.get(id){Some(Declaration::Function{params,body,..})=>Ok((params,body)),_=>Err("machine function identity".into())}}
 fn validate(&self,args:&[Value],params:&[(String,Sort)])->LangResult<()> {
  if args.len()!=params.len()||args.len()>64{return Err("machine argument count".into());}
  let mut work=args.iter().zip(params).map(|(v,(_,s))|(v,s.clone(),0usize)).collect::<Vec<_>>();let mut nodes=0;
  while let Some((v,s,depth))=work.pop(){nodes+=1;if nodes>100000||depth>8192{return Err("machine value limit".into());}match(v,s){(Value::Bool(_),Sort::Bool)|(Value::U64(_),Sort::U64)=>{},(Value::Data(id,tag,xs),Sort::Data(expected))if id==&expected=>{let Some(Declaration::Datatype{constructors})=self.declarations.get(id)else{return Err("machine datatype".into());};let c=constructors.get(*tag).ok_or("machine tag")?;if c.fields.len()!=xs.len(){return Err("machine arity".into());}work.extend(xs.iter().zip(&c.fields).map(|(v,s)|(v,if *s==Sort::SelfType{Sort::Data(id.clone())}else{s.clone()},depth+1)));},_=>return Err("machine sort".into())}}
  Ok(())
 }
 pub fn invoke(&self,id:&str,args:Vec<Value>)->LangResult<Value>{ self.invoke_impl(id,args,true) }
 pub(crate) fn invoke_prepared(&self,id:&str,args:Vec<Value>)->LangResult<Value>{ self.invoke_impl(id,args,false) }
 fn invoke_impl(&self,id:&str,args:Vec<Value>,validate:bool)->LangResult<Value>{
  if !self.exports.iter().any(|x|x==id){return Err("machine unexported function".into());}let (params,_)=self.function(id)?;if validate{self.validate(&args,params)?;}else if args.len()!=params.len(){return Err("machine argument count".into());}
  let target=self.choices.get(id).map(String::as_str).unwrap_or(id);
  let (params,body)=self.function(target)?;let env=Arc::new(params.iter().zip(args).map(|((n,_),v)|(n.clone(),v)).collect());
  let mut frames=vec![Frame::Eval(body,env,target)];let mut values=Vec::new();let mut work=0;let mut allocations=0;
  while let Some(frame)=frames.pop(){work+=1;if work>2000000||frames.len()>16384{return Err("machine execution limit".into());}match frame{
   Frame::Eval(t,env,current)=>match t{
    Term::Var(n)=>values.push(env.get(n).cloned().ok_or("machine variable")?),Term::Bool(x)=>values.push(Value::Bool(*x)),Term::U64(x)=>values.push(Value::U64(*x)),
    Term::If{condition,on_true,on_false}=>{frames.push(Frame::If(on_true,on_false,env.clone(),current));frames.push(Frame::Eval(condition,env,current));},
    Term::Match{scrutinee,branches}=>{frames.push(Frame::Match(branches,env.clone(),current));frames.push(Frame::Eval(scrutinee,env,current));},
    Term::Construct{datatype,constructor,arguments}=>{frames.push(Frame::Construct(datatype,*constructor,arguments.len()));frames.extend(arguments.iter().rev().map(|x|Frame::Eval(x,env.clone(),current)));},
    Term::Call{function,arguments}=>{frames.push(Frame::Call(function,arguments.len()));frames.extend(arguments.iter().rev().map(|x|Frame::Eval(x,env.clone(),current)));},
    Term::SelfCall(arguments)=>{frames.push(Frame::Call(current,arguments.len()));frames.extend(arguments.iter().rev().map(|x|Frame::Eval(x,env.clone(),current)));},
    Term::Binary{op,left,right}=>{frames.push(Frame::Binary(op));frames.push(Frame::Eval(right,env.clone(),current));frames.push(Frame::Eval(left,env,current));},
   },
   Frame::If(a,b,env,current)=>{let Some(Value::Bool(test))=values.pop()else{return Err("machine condition".into());};frames.push(Frame::Eval(if test{a}else{b},env,current));},
   Frame::Match(branches,env,current)=>{let Some(Value::Data(_,tag,xs))=values.pop()else{return Err("machine match".into());};let branch=branches.get(tag).ok_or("machine constructor")?;if branch.bindings.len()!=xs.len(){return Err("machine bindings".into());}let mut local=(*env).clone();local.extend(branch.bindings.iter().cloned().zip(xs.iter().cloned()));frames.push(Frame::Eval(&branch.body,Arc::new(local),current));},
   Frame::Construct(id,tag,n)=>{allocations+=1;if allocations>100000{return Err("machine allocation limit".into());}let xs=values.split_off(values.len().checked_sub(n).ok_or("machine stack")?);values.push(Value::Data(id.into(),tag,Arc::new(xs)));},
   Frame::Call(id,n)=>{let xs=values.split_off(values.len().checked_sub(n).ok_or("machine stack")?);let (params,body)=self.function(id)?;if xs.len()!=params.len(){return Err("machine call arity".into());}frames.push(Frame::Eval(body,Arc::new(params.iter().zip(xs).map(|((n,_),v)|(n.clone(),v)).collect()),id));},
   Frame::Binary(op)=>{let rhs=values.pop().ok_or("machine stack")?;let lhs=values.pop().ok_or("machine stack")?;let result=match(lhs,rhs){
    (Value::U64(a),Value::U64(b))if op.starts_with("f32:")=>match ink_core::logic::f32_primitive(op,a,b)?{Term::U64(x)=>Value::U64(x),Term::Bool(x)=>Value::Bool(x),_=>unreachable!()},
    (Value::U64(a),Value::U64(b))=>match op{"+"=>Value::U64(a.wrapping_add(b)),"-"=>Value::U64(a.wrapping_sub(b)),"*"=>Value::U64(a.wrapping_mul(b)),"&"=>Value::U64(a&b),"quot"=>Value::U64(a.checked_div(b).unwrap_or(0)),"rem"=>Value::U64(a.checked_rem(b).unwrap_or(0)),"=="=>Value::Bool(a==b),"!="=>Value::Bool(a!=b),"<"=>Value::Bool(a<b),"<="=>Value::Bool(a<=b),">"=>Value::Bool(a>b),">="=>Value::Bool(a>=b),_=>return Err("machine word operator".into())},
    (Value::Bool(a),Value::Bool(b))=>match op{"&&"=>Value::Bool(a&&b),"||"=>Value::Bool(a||b),"=="=>Value::Bool(a==b),"!="=>Value::Bool(a!=b),_=>return Err("machine Bool operator".into())},_=>return Err("machine operand sort".into())};values.push(result);},
  }}if values.len()!=1{return Err("machine result stack".into());}values.pop().ok_or("machine empty result".into())
 }
}
