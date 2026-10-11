// Per-instance task transport is separate from the state-handle registry. Host
// scheduling can re-enter this ABI without borrowing or publishing transaction state.
#[cfg(target_arch="wasm32")]
thread_local! {static PARALLEL_IO:std::cell::RefCell<Vec<u8>>=const{std::cell::RefCell::new(Vec::new())};}
#[cfg(target_arch="wasm32")]
#[link(wasm_import_module="ink_parallel")]
unsafe extern "C" {#[link_name="run"] fn __ink_parallel_import(ptr:u32,len:u32)->u32;}
#[cfg(target_arch="wasm32")]
fn __ink_parallel_host(task:Json)->Result<Json,String>{
    let bytes=serde_json::to_vec(&task).map_err(|e|e.to_string())?;
    if bytes.len()>64*1024*1024{return Err("parallel request limit".into());}
    let length=unsafe{__ink_parallel_import(bytes.as_ptr() as u32,bytes.len() as u32)};
    PARALLEL_IO.with(|io|{let io=io.borrow();if length as usize!=io.len(){return Err("parallel response length".into());}let reply:Json=serde_json::from_slice(&io).map_err(|e|e.to_string())?;if let Some(error)=reply["error"].as_str(){return Err(error.into());}reply.get("ok").cloned().ok_or("parallel response envelope".into())})
}
#[cfg(target_arch="wasm32")]
#[no_mangle]
pub extern "C" fn lang_parallel_alloc(len:u32)->u32{if len>64*1024*1024{return 0;}PARALLEL_IO.with(|io|{let mut io=io.borrow_mut();io.resize(len as usize,0);io.as_mut_ptr() as u32})}
#[cfg(target_arch="wasm32")]
#[no_mangle]
pub extern "C" fn lang_parallel_ptr()->u32{PARALLEL_IO.with(|io|io.borrow().as_ptr() as u32)}
#[cfg(target_arch="wasm32")]
#[no_mangle]
pub unsafe extern "C" fn lang_parallel_worker(ptr:u32,len:u32)->u32{
    let result=(||->Result<Json,String>{if len>64*1024*1024{return Err("parallel request limit".into());}let task:Json=serde_json::from_slice(std::slice::from_raw_parts(ptr as *const u8,len as usize)).map_err(|e|e.to_string())?;State::new().parallel_worker(&task)})();
    let reply=match result{Ok(v)=>json!({"ok":v}),Err(e)=>json!({"error":e})};
    let bytes=serde_json::to_vec(&reply).expect("parallel response");if bytes.len()>64*1024*1024{return 0;}
    let len=bytes.len() as u32;PARALLEL_IO.with(|io|*io.borrow_mut()=bytes);len
}
