//! Bounded ground-value ABI for literal C definitions. No AST interpreter.
use ink_core::{logic::Term, LangResult};
use std::{ffi::c_void,sync::Mutex};
type P=*mut c_void;
static LOCK:Mutex<()>=Mutex::new(());
pub struct Api {
 pub reset:unsafe extern "C" fn(), pub error:unsafe extern "C" fn()->i32,
 pub boolean:unsafe extern "C" fn(i32)->P, pub word:unsafe extern "C" fn(u64)->P,
 pub data:unsafe extern "C" fn(i32,i32,i32,*const P)->P,
 pub call:unsafe extern "C" fn(i32,*const P,i32)->P,
 pub kind:unsafe extern "C" fn(P)->i32, pub datatype:unsafe extern "C" fn(P)->i32,
 pub tag:unsafe extern "C" fn(P)->i32, pub count:unsafe extern "C" fn(P)->i32,
 pub scalar:unsafe extern "C" fn(P)->u64, pub field:unsafe extern "C" fn(P,i32)->P,
 pub datatypes:&'static [&'static str], pub functions:&'static [&'static str],
}
impl Api {
 /// The function table must come from the matching emitted C artifact. This
 /// trusted host linkage, not a receipt string, establishes the ABI boundary.
 pub unsafe fn execute(&self,function:&str,args:&[Term])->LangResult<Term>{
  let _guard=LOCK.lock().map_err(|_|"C definition executor lock")?;
  let index=self.functions.iter().position(|x|*x==function).ok_or("C unexported function")?;
  if args.len()>64{return Err("C input count limit".into());}
  (self.reset)();
  enum Read<'a>{Value(&'a Term,usize),Construct(&'a str,usize,usize)}
  let mut work=args.iter().rev().map(|x|Read::Value(x,0)).collect::<Vec<_>>();
  let mut values=Vec::new();let mut nodes=0;
  while let Some(w)=work.pop(){match w{
   Read::Value(t,depth)=>{nodes+=1;if nodes>100000||depth>8192{return Err("C transport limit".into());}
    match t {Term::Bool(x)=>values.push((self.boolean)(i32::from(*x))),Term::U64(x)=>values.push((self.word)(*x)),
     Term::Construct{datatype,constructor,arguments}=>{if arguments.len()>64{return Err("C constructor limit".into());}work.push(Read::Construct(datatype,*constructor,arguments.len()));work.extend(arguments.iter().rev().map(|x|Read::Value(x,depth+1)));},_=>return Err("C input is not ground".into())}
   },
   Read::Construct(id,tag,n)=>{let type_index=self.datatypes.iter().position(|x|*x==id).ok_or("C datatype identity")?;let xs=values.split_off(values.len()-n);values.push((self.data)(type_index as i32,i32::try_from(tag).map_err(|_|"C tag overflow")?,n as i32,xs.as_ptr()));}
  }}
  if (self.error)()!=0{return Err("C input allocation limit".into());}
  let result=(self.call)(index as i32,values.as_ptr(),values.len()as i32);
  if (self.error)()!=0{return Err("C definition execution limit or malformed input".into());}
  enum Output{Value(P,usize),Construct(String,usize,usize)}
  let mut work=vec![Output::Value(result,0)];let mut values=Vec::new();let mut nodes=0;
  while let Some(w)=work.pop(){match w{
   Output::Value(p,depth)=>{nodes+=1;if nodes>100000||depth>8192{return Err("C result transport limit".into());}match (self.kind)(p){
    0=>values.push(Term::Bool((self.scalar)(p)!=0)),1=>values.push(Term::U64((self.scalar)(p))),
    2=>{let i=(self.datatype)(p);let id=self.datatypes.get(usize::try_from(i).map_err(|_|"C datatype")?).ok_or("C result datatype")?;let n=(self.count)(p);if !(0..=64).contains(&n){return Err("C result arity".into());}let tag=(self.tag)(p);work.push(Output::Construct((*id).into(),usize::try_from(tag).map_err(|_|"C result tag")?,n as usize));for i in (0..n).rev(){work.push(Output::Value((self.field)(p,i),depth+1));}},_=>return Err("C result kind".into())}
   },
   Output::Construct(datatype,constructor,n)=>{let arguments=values.split_off(values.len()-n);values.push(Term::Construct{datatype,constructor,arguments});}
  }}
  values.pop().ok_or("C empty result".into())
 }
}
