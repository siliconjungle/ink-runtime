// Primitive value/storage runtime for literal emitted C. No parser or evaluator.
use num_bigint::BigInt;
use serde_json::{json, Value as Json};
use std::collections::BTreeMap;
use std::ffi::{c_char, CStr};
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum V {
    Unit,
    Bool(bool),
    U32(u32),
    U64(u64),
    I32(i32),
    F32(u32),
    Int(BigInt),
    String(String),
    Id(String, u128),
    Record(String, BTreeMap<String, V>),
    Enum(String),
    Option(Option<Box<V>>),
    Result(bool, Box<V>),
    List(Vec<V>),
    Vector(Vec<V>),
    Buffer(OwnedBuffer<V>),
    Map(OwnedMap<V,V>),
}
impl V {
    fn weight(&self) -> usize {
        64 + match self {
            Self::Int(v) => (v.bits() as usize + 7) / 8,
            Self::String(v) | Self::Enum(v) => v.len(),
            Self::Record(_, fs) => fs.values().map(Self::weight).sum(),
            Self::List(xs) | Self::Vector(xs) => xs.iter().map(Self::weight).sum(),
            Self::Option(Some(v)) | Self::Result(_, v) => v.weight(),
            _ => 0,
        }
    }
    fn json(&self) -> Json {
        match self {
            Self::Buffer(_) | Self::Map(_) => Json::Null,
            Self::Unit => Json::Null,
            Self::Bool(v) => json!(v),
            Self::U32(v) => json!(v),
            Self::U64(v) => json!(v),
            Self::I32(v) => json!(v),
            Self::F32(b) => {
                let f = f32::from_bits(*b);
                if f.is_finite() {
                    json!(f)
                } else {
                    json!({"F32Bits":b})
                }
            }
            Self::Int(v) => json!({"Int":v.to_string()}),
            Self::String(v) | Self::Enum(v) => json!(v),
            Self::Id(_, v) => json!(format!("{v:032x}")),
            Self::Record(_, fs) => fs.iter().map(|(k, v)| (k.clone(), v.json())).collect(),
            Self::Option(Some(v)) => json!({"Some":v.json()}),
            Self::Option(None) => json!({"None":null}),
            Self::Result(true, v) => json!({"Ok":v.json()}),
            Self::Result(false, v) => json!({"Err":v.json()}),
            Self::List(xs) | Self::Vector(xs) => xs.iter().map(Self::json).collect(),
        }
    }
}
type R<T> = Result<T, String>;
fn from_json(v: &Json, t: &Json, meta: &Json, left: &mut usize, depth: usize) -> R<V> {
    if depth > 128 || *left == 0 {
        return Err("value resource limit exceeded".into());
    }
    *left -= 1;
    if let Some(kind) = t.as_str() {
        return Ok(match kind {
            "Unit" if v.is_null() => V::Unit,
            "Bool" => V::Bool(v.as_bool().ok_or("expected Bool")?),
            "U32" => V::U32(
                v.as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or("expected u32")?,
            ),
            "U64" => {
                let n = if let Some(n) = v.as_u64() {
                    n
                } else {
                    let s = v.as_str().ok_or("expected u64")?;
                    let n = s.parse::<u64>().map_err(|_| "expected u64")?;
                    if n.to_string() != s {
                        return Err("noncanonical u64".into());
                    }
                    n
                };
                V::U64(n)
            }
            "I32" => V::I32(
                v.as_i64()
                    .and_then(|n| i32::try_from(n).ok())
                    .ok_or("expected i32")?,
            ),
            "F32" => {
                if let Some(n) = v.as_f64() {
                    let f = n as f32;
                    if !f.is_finite() {
                        return Err("f32 overflow".into());
                    }
                    V::F32(f.to_bits())
                } else {
                    let o = v.as_object().ok_or("expected F32Bits")?;
                    if o.len() != 1 {
                        return Err("expected F32Bits".into());
                    }
                    V::F32(
                        o.get("F32Bits")
                            .and_then(Json::as_u64)
                            .and_then(|n| u32::try_from(n).ok())
                            .ok_or("invalid F32Bits")?,
                    )
                }
            }
            "Int" => {
                let text = v
                    .get("Int")
                    .and_then(Json::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| v.to_string());
                if text.len() > 1000000 {
                    return Err("Int input exceeds resource limit".into());
                }
                V::Int(text.parse().map_err(|_| "expected Int")?)
            }
            "String" => V::String(v.as_str().ok_or("expected String")?.into()),
            _ => return Err("unsupported primitive type".into()),
        });
    }
    let object = t.as_object().ok_or("invalid compiled type")?;
    let (kind, inner) = object.iter().next().ok_or("empty compiled type")?;
    Ok(match kind.as_str() {
        "List" => V::List(
            v.as_array()
                .ok_or("expected List")?
                .iter()
                .map(|x| from_json(x, inner, meta, left, depth + 1))
                .collect::<R<Vec<_>>>()?,
        ),
        "Vector" => {
            let xs = v.as_array().ok_or("expected vector")?;
            let parts = inner.as_array().ok_or("vector type")?;
            if xs.len() != parts[1].as_u64().ok_or("vector size")? as usize {
                return Err("vector dimension mismatch".into());
            }
            V::Vector(
                xs.iter()
                    .map(|x| from_json(x, &parts[0], meta, left, depth + 1))
                    .collect::<R<Vec<_>>>()?,
            )
        }
        "Option" => {
            let obj = v.as_object().ok_or("expected Option")?;
            if obj.len() != 1 {
                return Err("expected one tag".into());
            }
            if let Some(x) = obj.get("Some") {
                V::Option(Some(Box::new(from_json(x, inner, meta, left, depth + 1)?)))
            } else if obj.get("None") == Some(&Json::Null) {
                V::Option(None)
            } else {
                return Err("invalid Option".into());
            }
        }
        "Result" => {
            let obj = v.as_object().ok_or("expected Result")?;
            if obj.len() != 1 {
                return Err("expected one tag".into());
            }
            let ts = inner.as_array().ok_or("result type")?;
            if let Some(x) = obj.get("Ok") {
                V::Result(true, Box::new(from_json(x, &ts[0], meta, left, depth + 1)?))
            } else if let Some(x) = obj.get("Err") {
                V::Result(
                    false,
                    Box::new(from_json(x, &ts[1], meta, left, depth + 1)?),
                )
            } else {
                return Err("invalid Result".into());
            }
        }
        "Named" => {
            let name = inner.as_str().ok_or("named type")?;
            if meta["ids"]
                .as_array()
                .is_some_and(|xs| xs.iter().any(|x| x == name))
            {
                let s = v.as_str().ok_or("expected ID")?;
                if s.len() != 32 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err("invalid ID".into());
                }
                V::Id(
                    name.into(),
                    u128::from_str_radix(s, 16).map_err(|_| "invalid ID")?,
                )
            } else if name == "ArithmeticError" {
                if v.as_str() != Some("ArithmeticError.Overflow") {
                    return Err("invalid arithmetic error".into());
                }
                V::Enum("ArithmeticError.Overflow".into())
            } else if let Some(fs) = meta["records"].get(name).and_then(Json::as_array) {
                let obj = v.as_object().ok_or("expected record")?;
                if obj.len() != fs.len() {
                    return Err("record field mismatch".into());
                }
                let mut values = BTreeMap::new();
                for f in fs {
                    let key = f[0].as_str().ok_or("field name")?;
                    values.insert(
                        key.into(),
                        from_json(
                            obj.get(key).ok_or("missing field")?,
                            &f[1],
                            meta,
                            left,
                            depth + 1,
                        )?,
                    );
                }
                V::Record(name.into(), values)
            } else {
                let s = v.as_str().ok_or("expected enum")?;
                let prefix = format!("{name}.");
                let variant = s.strip_prefix(&prefix).ok_or("enum type mismatch")?;
                if !meta["enums"][name]
                    .as_array()
                    .is_some_and(|vs| vs.iter().any(|v| v == variant))
                {
                    return Err("invalid enum variant".into());
                }
                V::Enum(s.into())
            }
        }
        _ => return Err("unsupported compiled type".into()),
    })
}
#[derive(Clone, Debug)]
enum Failure {
    Host(String),
    Abort(V),
}
pub struct InkContext {
    values: Vec<V>,
    error: Option<Failure>,
    fuel: u64,
    arena_bytes: usize,
    meta: Json,
    tables: Vec<BTreeMap<V, V>>,
    version: u64,
    staged: Vec<(String, V)>,
    outbox: Vec<Event>,
}
#[derive(Clone, Debug)]
pub struct Event {
    pub commit: u64,
    pub position: u64,
    pub channel: String,
    value: V,
}
impl Event {
    pub fn to_json(&self) -> Json {
        json!({"commit":self.commit,"position":self.position,"channel":self.channel,"value":self.value.json()})
    }
}
impl InkContext {
    fn put(&mut self, v: V) -> u64 {
        // Opaque value handles carry fixed-width scalar payloads directly.
        // Heap values remain arena indices; language and snapshot types do not change.
        let immediate = match &v {
            V::Unit => Some((1u64, 0u32)),
            V::Bool(x) => Some((2, *x as u32)),
            V::U32(x) => Some((3, *x)),
            V::I32(x) => Some((4, *x as u32)),
            V::F32(x) => Some((5, *x)),
            _ => None,
        };
        if let Some((tag, bits)) = immediate {
            return (tag << 32) | u64::from(bits);
        }
        let n = v.weight();
        if n > 64 * 1024 * 1024 - self.arena_bytes {
            return self.fail("Ink value arena limit exceeded".into());
        }
        self.arena_bytes += n;
        self.values.push(v);
        self.values.len() as u64
    }
    fn value(&self, h: u64) -> R<V> {
        let bits = h as u32;
        match h >> 32 {
            1 if bits == 0 => Ok(V::Unit),
            2 if bits <= 1 => Ok(V::Bool(bits != 0)),
            3 => Ok(V::U32(bits)),
            4 => Ok(V::I32(bits as i32)),
            5 => Ok(V::F32(bits)),
            0 => self
                .values
                .get(
                    usize::try_from(h.checked_sub(1).ok_or("invalid value handle")?)
                        .map_err(|_| "invalid value handle")?,
                )
                .cloned()
                .ok_or("invalid value handle".into()),
            _ => Err("invalid value handle".into()),
        }
    }
    fn fail(&mut self, e: String) -> u64 {
        if self.error.is_none() {
            self.error = Some(Failure::Host(e))
        }
        0
    }
    fn result(&mut self, r: R<V>) -> u64 {
        match r {
            Ok(v) => self.put(v),
            Err(e) => self.fail(e),
        }
    }
    fn tick(&mut self) -> bool {
        if self.error.is_some() {
            return false;
        }
        if self.fuel == 0 {
            self.fail("Ink execution budget exhausted".into());
            false
        } else {
            self.fuel -= 1;
            true
        }
    }
    fn primitive(&mut self, op: &str, a: &mut [V]) -> R<V> {
        if op=="list_length" {let V::List(xs)=&a[0] else{return Err("list length".into())};return Ok(V::U32(xs.len().try_into().map_err(|_|"list exceeds u32")?));}
        if op=="buffer_empty" {return Ok(V::Buffer(OwnedBuffer::new(vec![])));}
        if op=="map_empty" {return Ok(V::Map(OwnedMap::new()));}
        if op=="buffer_new" {let V::List(xs)=std::mem::replace(&mut a[0],V::Unit) else{return Err("buffer list".into())};return Ok(V::Buffer(OwnedBuffer::new(xs)));}
        if op=="buffer_fill" {
            let V::U32(n)=a[0] else{return Err("buffer length".into())};if n>1000000{return Err("temporary allocation limit exceeded".into())};
            let weight=(n as usize).checked_mul(a[1].weight()).ok_or("temporary allocation limit exceeded")?;
            if weight>64*1024*1024-self.arena_bytes{return Err("temporary allocation limit exceeded".into())};self.arena_bytes+=weight;
            return Ok(V::Buffer(OwnedBuffer::new(vec![a[1].clone();n as usize])));
        }
        if op=="buffer_split" {
            let V::Buffer(b)=a[0].clone() else{return Err("buffer receiver".into())};
            let V::U32(at)=a[1] else{return Err("buffer index".into())};
            let (l,r)=b.split(at);return Ok(V::List(vec![V::Buffer(l),V::Buffer(r)]));
        }
        if op=="buffer_append" {
            let (V::Buffer(left),V::Buffer(right))=(a[0].clone(),a[1].clone()) else{return Err("buffer append".into())};
            if (left.len() as u64)+(right.len() as u64)>1000000{return Err("temporary allocation limit exceeded".into())};
            return Ok(V::Buffer(left.append(right)));
        }
        if op=="list_drop"||op=="list_take" {
            let V::List(mut xs)=std::mem::replace(&mut a[0],V::Unit) else{return Err("list slice".into())};
            let V::U32(k)=a[1] else{return Err("list slice count".into())};let k=(k as usize).min(xs.len());
            if op=="list_drop"{xs.drain(..k);}else{xs.truncate(k);}
            return Ok(V::List(xs));
        }
        if let Some(method)=op.strip_prefix("buffer_") {
            let V::Buffer(b)=a[0].clone() else{return Err("buffer receiver".into())};
            let index=|v:&V|->R<u32>{if let V::U32(x)=v{Ok(*x)}else{Err("buffer index".into())}};
            return Ok(match method {
                "len"=>V::U32(b.len()),"freeze"=>V::List(b.freeze()),"pop"=>V::Buffer(b.pop()),
                "contains"=>V::Bool(index(&a[1])?<b.len()),"get"=>b.get(index(&a[1])?).ok_or("buffer index outside bounds")?,
                "set"=>V::Buffer(b.set(index(&a[1])?,a[2].clone())),"swap"=>V::Buffer(b.swap(index(&a[1])?,index(&a[2])?)),
                "push"=>{if b.len()>=1000000{return Err("temporary allocation limit exceeded".into())};let weight=a[1].weight();if weight>64*1024*1024-self.arena_bytes{return Err("temporary allocation limit exceeded".into())};self.arena_bytes+=weight;V::Buffer(b.push(a[1].clone()))},
                "truncate"=>V::Buffer(b.truncate(index(&a[1])?)),_=>return Err("buffer method".into())
            });
        }
        if let Some(method)=op.strip_prefix("map_") {
            let V::Map(m)=a[0].clone() else{return Err("map receiver".into())};
            return Ok(match method {
                "len"=>V::U32(m.len()),"keys"=>V::List(m.keys()),"values"=>V::List(m.values()),
                "contains"=>V::Bool(m.contains(&a[1])),"get"=>m.get(&a[1]).ok_or("missing map key")?,
                "set"=>{let weight=a[1].weight()+a[2].weight();if weight>64*1024*1024-self.arena_bytes{return Err("temporary allocation limit exceeded".into())};self.arena_bytes+=weight;V::Map(m.set(a[1].clone(),a[2].clone()))},
                "remove"=>V::Map(m.remove(&a[1])),_=>return Err("map method".into())
            });
        }
        if let Some(name) = op.strip_prefix("record:") {
            let fs = self.meta["records"][name]
                .as_array()
                .ok_or("record schema")?;
            if fs.len() != a.len() {
                return Err("record arity".into());
            }
            return Ok(V::Record(
                name.into(),
                fs.iter()
                    .zip(a.iter_mut())
                    .map(|(f, v)| (f[0].as_str().unwrap().into(), std::mem::replace(v, V::Unit)))
                    .collect(),
            ));
        }
        if let Some(field) = op.strip_prefix("field:") {
            return match &a[0] {
                V::Record(_, fs) => fs.get(field).cloned().ok_or("unknown field".into()),
                V::Vector(xs) => ["x", "y", "z", "w"]
                    .iter()
                    .position(|f| *f == field)
                    .and_then(|i| xs.get(i))
                    .cloned()
                    .ok_or("unknown component".into()),
                _ => Err("field requires record/vector".into()),
            };
        }
        if let Some(tag) = op.strip_prefix("tag:") {
            return Ok(V::Bool(matches!(
                (&a[0], tag),
                (V::Option(Some(_)), "Some")
                    | (V::Result(true, _), "Ok")
                    | (V::Result(false, _), "Err")
            )));
        }
        if let Some(name) = op.strip_prefix("emit:") {
            self.staged.push((name.into(), a[0].clone()));
            return Ok(V::Unit);
        }
        if let Some(table) = op.strip_prefix("table:") {
            let (index, method) = table.split_once(':').ok_or("table operation")?;
            let index = index.parse::<usize>().map_err(|_| "table index")?;
            let t = self.tables.get_mut(index).ok_or("table index")?;
            return Ok(match method {
                "values" => V::List(t.values().cloned().collect()),
                "get" => V::Option(t.get(&a[0]).cloned().map(Box::new)),
                "contains" => V::Bool(t.contains_key(&a[0])),
                "remove" => V::Option(t.remove(&a[0]).map(Box::new)),
                "insert" => {
                    if t.contains_key(&a[0]) {
                        return Err("insert key already present".into());
                    }
                    t.insert(a[0].clone(), a[1].clone());
                    V::Unit
                }
                "replace" => {
                    if !t.contains_key(&a[0]) {
                        return Err("replace key absent".into());
                    }
                    t.insert(a[0].clone(), a[1].clone());
                    V::Unit
                }
                _ => return Err("table method".into()),
            });
        }
        Ok(match op {
            "list" => V::List(vec![]),
            "vector" => V::Vector(
                a.iter_mut()
                    .map(|v| std::mem::replace(v, V::Unit))
                    .collect(),
            ),
            "Some" => V::Option(Some(Box::new(a[0].clone()))),
            "None" => V::Option(None),
            "Ok" | "Err" => V::Result(op == "Ok", Box::new(a[0].clone())),
            "unwrap" => match &a[0] {
                V::Option(Some(v)) | V::Result(_, v) => *v.clone(),
                _ => return Err("expected wrapped value".into()),
            },
            "try" | "nested_change" => match &a[0] {
                V::Result(false, e) => {
                    self.error = Some(Failure::Abort(*e.clone()));
                    return Ok(V::Unit);
                }
                V::Result(true, v) => {
                    if op == "try" {
                        *v.clone()
                    } else {
                        a[0].clone()
                    }
                }
                _ => return Err("expected Result".into()),
            },
            "ok_or" => match &a[0] {
                V::Option(Some(v)) => V::Result(true, v.clone()),
                V::Option(None) => V::Result(false, Box::new(a[1].clone())),
                _ => return Err("expected Option".into()),
            },
            "Int" => V::Int(match &a[0] {
                V::U32(n) => BigInt::from(*n),
                V::U64(n) => BigInt::from(*n),
                V::I32(n) => BigInt::from(*n),
                V::Int(n) => n.clone(),
                _ => return Err("expected integer".into()),
            }),
            "sort" => match &a[0] {
                V::List(xs) => {
                    let mut xs = xs.clone();
                    xs.sort();
                    V::List(xs)
                }
                _ => return Err("expected list".into()),
            },
            "div_ok" => V::Bool(!matches!(
                (&a[0], &a[1]),
                (_, V::U32(0)) | (_, V::U64(0)) | (_, V::I32(0)) | (V::I32(i32::MIN), V::I32(-1))
            )),
            "neg" => match &a[0] {
                V::U32(n) => V::U32(n.wrapping_neg()),
                V::U64(n) => V::U64(n.wrapping_neg()),
                V::I32(n) => V::I32(n.wrapping_neg()),
                V::F32(n) => V::F32(n ^ 0x80000000),
                _ => return Err("numeric negation".into()),
            },
            "==" | "!=" | "<" | ">" | "<=" | ">=" => {
                let equal = if let (V::F32(x), V::F32(y)) = (&a[0], &a[1]) {
                    f32::from_bits(*x) == f32::from_bits(*y)
                } else {
                    a[0] == a[1]
                };
                let order = if let (V::F32(x), V::F32(y)) = (&a[0], &a[1]) {
                    f32::from_bits(*x).partial_cmp(&f32::from_bits(*y))
                } else {
                    Some(a[0].cmp(&a[1]))
                };
                use std::cmp::Ordering::*;
                V::Bool(match op {
                    "==" => equal,
                    "!=" => !equal,
                    "<" => order == Some(Less),
                    ">" => order == Some(Greater),
                    "<=" => matches!(order, Some(Less | Equal)),
                    _ => matches!(order, Some(Greater | Equal)),
                })
            }
            "checked_add" | "checked_sub" | "checked_mul" => {
                let value = match (&a[0], &a[1]) {
                    (V::I32(x), V::I32(y)) => match op {
                        "checked_add" => x.checked_add(*y),
                        "checked_sub" => x.checked_sub(*y),
                        _ => x.checked_mul(*y),
                    }
                    .map(V::I32),
                    (V::U32(x), V::U32(y)) => match op {
                        "checked_add" => x.checked_add(*y),
                        "checked_sub" => x.checked_sub(*y),
                        _ => x.checked_mul(*y),
                    }
                    .map(V::U32),
                    (V::U64(x), V::U64(y)) => match op {
                        "checked_add" => x.checked_add(*y),
                        "checked_sub" => x.checked_sub(*y),
                        _ => x.checked_mul(*y),
                    }
                    .map(V::U64),
                    _ => return Err("checked arithmetic types".into()),
                };
                if let Some(v) = value {
                    V::Result(true, Box::new(v))
                } else {
                    V::Result(false, Box::new(V::Enum("ArithmeticError.Overflow".into())))
                }
            }
            "+" | "-" | "*" | "quot" | "rem" => match (&a[0], &a[1]) {
                (V::U32(x), V::U32(y)) => V::U32(match op {
                    "+" => x.wrapping_add(*y),
                    "-" => x.wrapping_sub(*y),
                    "*" => x.wrapping_mul(*y),
                    "quot" => x.checked_div(*y).ok_or("invalid division")?,
                    _ => x.checked_rem(*y).ok_or("invalid division")?,
                }),
                (V::U64(x), V::U64(y)) => V::U64(match op {
                    "+" => x.wrapping_add(*y),
                    "-" => x.wrapping_sub(*y),
                    "*" => x.wrapping_mul(*y),
                    "quot" => x.checked_div(*y).ok_or("invalid division")?,
                    _ => x.checked_rem(*y).ok_or("invalid division")?,
                }),
                (V::I32(x), V::I32(y)) => V::I32(match op {
                    "+" => x.wrapping_add(*y),
                    "-" => x.wrapping_sub(*y),
                    "*" => x.wrapping_mul(*y),
                    "quot" => x.checked_div(*y).ok_or("invalid division")?,
                    _ => x.checked_rem(*y).ok_or("invalid division")?,
                }),
                (V::F32(x), V::F32(y)) => {
                    let x = f32::from_bits(*x);
                    let y = f32::from_bits(*y);
                    V::F32(
                        match op {
                            "+" => x + y,
                            "-" => x - y,
                            "*" => x * y,
                            _ => return Err("float division".into()),
                        }
                        .to_bits(),
                    )
                }
                (V::Int(x), V::Int(y)) => V::Int(match op {
                    "+" => x + y,
                    "-" => x - y,
                    "*" => x * y,
                    _ => return Err("exact division unsupported".into()),
                }),
                _ => return Err("numeric operand types".into()),
            },
            _ => return Err(format!("unknown primitive {op}")),
        })
    }
}
unsafe extern "C" {
    fn ink_program_call(
        ctx: *mut std::ffi::c_void,
        kind: u32,
        index: u32,
        args: *const u64,
        len: usize,
    ) -> u64;
}
#[no_mangle]
pub unsafe extern "C" fn ink_failed(ctx: *mut InkContext) -> bool {
    (*ctx).error.is_some()
}
#[no_mangle]
pub unsafe extern "C" fn ink_tick(ctx: *mut InkContext) -> bool {
    (*ctx).tick()
}
#[no_mangle]
pub unsafe extern "C" fn ink_error(ctx: *mut InkContext, text: *const c_char) -> u64 {
    (*ctx).fail(CStr::from_ptr(text).to_string_lossy().into())
}
#[no_mangle]
pub unsafe extern "C" fn ink_literal(
    ctx: *mut InkContext,
    kind: u32,
    n: u64,
    text: *const c_char,
    len: usize,
) -> u64 {
    let c = &mut *ctx;
    if !c.tick() {
        return 0;
    }
    let r = (|| -> R<V> {
        Ok(match kind {
            0 => V::Unit,
            1 => V::Bool(n != 0),
            2 => V::U32(n as u32),
            3 => V::U64(n),
            4 => V::I32(n as i32),
            5 => V::F32(n as u32),
            6 => V::Int(BigInt::from(n)),
            7 | 8 => {
                let text = std::str::from_utf8(std::slice::from_raw_parts(text as *const u8, len))
                    .map_err(|_| "invalid UTF-8")?
                    .to_owned();
                if kind == 7 {
                    V::String(text)
                } else {
                    V::Enum(text)
                }
            }
            _ => return Err("invalid literal kind".into()),
        })
    })();
    c.result(r)
}
#[no_mangle]
pub unsafe extern "C" fn ink_op(
    ctx: *mut InkContext,
    op: *const c_char,
    args: *const u64,
    n: usize,
) -> u64 {
    let c = &mut *ctx;
    if !c.tick() {
        return 0;
    }
    let r = (|| -> R<V> {
        let op = CStr::from_ptr(op)
            .to_str()
            .map_err(|_| "invalid primitive")?;
        let hs = if n == 0 {
            &[]
        } else {
            std::slice::from_raw_parts(args, n)
        };
        // Most primitives have at most four operands; keep their scratch values
        // on the stack rather than allocating a fresh Vec for every operation.
        if hs.len() <= 4 {
            let mut scratch: [V; 4] = std::array::from_fn(|_| V::Unit);
            for (slot, h) in scratch.iter_mut().zip(hs) {
                *slot = c.value(*h)?;
            }
            c.primitive(op, &mut scratch[..hs.len()])
        } else {
            let mut vs = hs.iter().map(|h| c.value(*h)).collect::<R<Vec<_>>>()?;
            c.primitive(op, &mut vs)
        }
    })();
    if c.error.is_some() {
        0
    } else {
        c.result(r)
    }
}
#[no_mangle]
pub unsafe extern "C" fn ink_item(ctx: *mut InkContext, h: u64, i: usize) -> u64 {
    let c = &mut *ctx;
    let r = match h
        .checked_sub(1)
        .and_then(|n| usize::try_from(n).ok())
        .and_then(|i| c.values.get(i))
    {
        Some(V::List(xs) | V::Vector(xs)) => {
            xs.get(i).cloned().ok_or("index outside collection".into())
        }
        _ => Err("expected collection".into()),
    };
    c.result(r)
}
#[no_mangle]
pub unsafe extern "C" fn ink_length(ctx: *mut InkContext, h: u64) -> usize {
    let c = &mut *ctx;
    match h
        .checked_sub(1)
        .and_then(|n| usize::try_from(n).ok())
        .and_then(|i| c.values.get(i))
    {
        Some(V::List(xs) | V::Vector(xs)) => xs.len(),
        _ => {
            c.fail("expected collection".into());
            0
        }
    }
}
#[no_mangle]
pub unsafe extern "C" fn ink_append(ctx: *mut InkContext, h: u64, item: u64) {
    let c = &mut *ctx;
    if !c.tick() {
        return;
    }
    match c.value(item) {
        Ok(v) => match h
            .checked_sub(1)
            .and_then(|n| usize::try_from(n).ok())
            .and_then(|i| c.values.get_mut(i))
        {
            Some(V::List(xs)) => {
                let weight = v.weight();
                if weight > 64 * 1024 * 1024 - c.arena_bytes {
                    c.fail("Ink value arena limit exceeded".into());
                } else {
                    c.arena_bytes += weight;
                    xs.push(v);
                }
            }
            _ => {
                c.fail("expected list".into());
            }
        },
        Err(e) => {
            c.fail(e);
        }
    }
}
#[no_mangle]
pub unsafe extern "C" fn ink_bool(ctx: *mut InkContext, h: u64) -> bool {
    let c = &mut *ctx;
    match c.value(h) {
        Ok(V::Bool(v)) => v,
        _ => {
            c.fail("expected Bool".into());
            false
        }
    }
}
#[no_mangle]
pub unsafe extern "C" fn ink_u32(ctx: *mut InkContext, h: u64) -> u32 {
    let c = &mut *ctx;
    match c.value(h) {
        Ok(V::U32(v)) => v,
        _ => {
            c.fail("expected u32".into());
            0
        }
    }
}
#[no_mangle]
pub unsafe extern "C" fn ink_finish_action(ctx: *mut InkContext, h: u64) -> u64 {
    let c = &mut *ctx;
    match c.error.take() {
        Some(Failure::Abort(v)) => c.put(V::Result(false, Box::new(v))),
        Some(e) => {
            c.error = Some(e);
            0
        }
        None => h,
    }
}
pub struct State {
    ctx: Box<InkContext>,
}
impl State {
    pub fn new() -> Self {
        let meta: Json =
            serde_json::from_str(include_str!("metadata.json")).expect("compiled metadata");
        let count = meta["roots"].as_array().unwrap().len();
        Self {
            ctx: Box::new(InkContext {
                meta,
                tables: vec![BTreeMap::new(); count],
                values: vec![],
                error: None,
                fuel: 100000000,
                arena_bytes: 0,
                version: 0,
                staged: vec![],
                outbox: vec![],
            }),
        }
    }
    fn call(&mut self, name: &str, args: &Json, kind: u32) -> R<Json> {
        let section = if kind == 0 { "functions" } else { "actions" };
        let declarations = self.ctx.meta[section]
            .as_array()
            .ok_or("compiled declarations")?;
        let (index, d) = declarations
            .iter()
            .enumerate()
            .find(|(_, d)| d["name"] == name)
            .ok_or_else(|| format!("unknown {section} {name}"))?;
        let d = d.clone();
        if kind==0 && d["internal"].as_bool()==Some(true) {return Err("temporary storage is internal to a pure invocation".into());}
        let args = args.as_array().ok_or("expected argument array")?;
        let params = d["params"].as_array().ok_or("compiled parameters")?;
        if args.len() != params.len() {
            return Err("argument count mismatch".into());
        }
        let mut left = 1000000;
        let captured = args
            .iter()
            .zip(params)
            .map(|(v, t)| from_json(v, t, &self.ctx.meta, &mut left, 0))
            .collect::<R<Vec<_>>>()?;
        self.ctx.values.clear();
        self.ctx.arena_bytes = 0;
        self.ctx.error = None;
        self.ctx.fuel = 100000000;
        self.ctx.staged.clear();
        let handles = captured
            .into_iter()
            .map(|v| self.ctx.put(v))
            .collect::<Vec<_>>();
        let tables = self.ctx.tables.clone();
        let version = self.ctx.version;
        let h = unsafe {
            ink_program_call(
                (&mut *self.ctx as *mut InkContext).cast(),
                kind,
                index as u32,
                handles.as_ptr(),
                handles.len(),
            )
        };
        let result = if let Some(e) = self.ctx.error.take() {
            Err(match e {
                Failure::Host(e) => e,
                Failure::Abort(_) => "unhandled domain error".into(),
            })
        } else {
            self.ctx.value(h).map(|v| v.json())
        };
        if kind == 0 {
            self.ctx.values.clear();
            return result;
        }
        match result {
            Err(e) => {
                self.ctx.tables = tables;
                self.ctx.staged.clear();
                self.ctx.values.clear();
                Err(e)
            }
            Ok(result) => {
                let change = d["change"].as_bool().unwrap_or(false);
                let succeeded = result.get("Ok").is_some();
                if change {
                    let next = match transaction::decide(version, succeeded) {
                        transaction::Decision::Rollback => {
                            self.ctx.tables = tables;
                            self.ctx.staged.clear();
                            self.ctx.values.clear();
                            return Ok(
                                json!({"result":result,"committed":false,"version":version,"events":[]}),
                            );
                        }
                        transaction::Decision::Exhausted => {
                            self.ctx.tables = tables;
                            self.ctx.staged.clear();
                            self.ctx.values.clear();
                            return Err("commit sequence exhausted".into());
                        }
                        transaction::Decision::Commit { version } => version,
                    };
                    self.ctx.version = next;
                    let events = transaction::publish(next, self.ctx.staged.drain(..))
                        .map(|p| {
                            let (channel, value) = p.value;
                            Event {
                                commit: p.commit,
                                position: p.position,
                                channel,
                                value,
                            }
                        })
                        .collect::<Vec<_>>();
                    let wire = events.iter().map(Event::to_json).collect::<Vec<_>>();
                    self.ctx.outbox.extend(events);
                    self.ctx.values.clear();
                    Ok(json!({"result":result,"committed":true,"version":next,"events":wire}))
                } else {
                    self.ctx.tables = tables;
                    self.ctx.staged.clear();
                    self.ctx.values.clear();
                    Ok(json!({"result":result,"committed":false,"version":version,"events":[]}))
                }
            }
        }
    }
    pub fn call_json(&mut self, name: &str, args: &Json) -> R<Json> {
        self.call(name, args, 0)
    }
    pub fn invoke_json(&mut self, name: &str, args: &Json) -> R<Json> {
        self.call(name, args, 1)
    }
    pub fn version(&self) -> u64 {
        self.ctx.version
    }
    pub fn outbox(&self) -> &[Event] {
        &self.ctx.outbox
    }
    pub fn acknowledge_through(&mut self, commit: u64, position: u64) {
        self.ctx
            .outbox
            .retain(|e| (e.commit, e.position) > (commit, position));
    }
    pub fn checkpoint(&self) -> R<Vec<u8>> {
        self.checkpoint_with_limits(portable::Limits::default())
    }
    pub fn checkpoint_with_limits(&self, limits: portable::Limits) -> R<Vec<u8>> {
        let layout = snapshot_layout(&self.ctx.meta)?;
        let mut roots = self.ctx.meta["roots"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .collect::<Vec<_>>();
        roots.sort_by(|a, b| a.1["name"].as_str().cmp(&b.1["name"].as_str()));
        let tables = roots
            .iter()
            .map(|(i, _)| {
                self.ctx.tables[*i]
                    .iter()
                    .map(|(k, v)| (k.json(), v.json()))
                    .collect()
            })
            .collect();
        let outbox = self
            .ctx
            .outbox
            .iter()
            .map(|e| {
                Ok(portable::Event {
                    commit: e.commit,
                    position: e.position,
                    channel: layout
                        .events
                        .iter()
                        .position(|(n, _)| n == &e.channel)
                        .ok_or("event channel")?,
                    value: e.value.json(),
                })
            })
            .collect::<R<Vec<_>>>()?;
        portable::encode(
            &layout,
            &portable::Logical {
                version: self.ctx.version,
                tables,
                outbox,
            },
            limits,
        )
    }
    pub fn restore(bytes: &[u8]) -> R<Self> {
        Self::restore_with_limits(bytes, portable::Limits::default())
    }
    pub fn restore_with_limits(bytes: &[u8], limits: portable::Limits) -> R<Self> {
        let mut s = Self::new();
        let layout = snapshot_layout(&s.ctx.meta)?;
        let logical = portable::decode(&layout, bytes, limits)?;
        let mut roots = s.ctx.meta["roots"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .collect::<Vec<_>>();
        roots.sort_by(|a, b| a.1["name"].as_str().cmp(&b.1["name"].as_str()));
        let mut left = 1000000;
        for ((index, r), rows) in roots.iter().zip(logical.tables) {
            let kt = &r["type"]["Table"][0];
            let vt = &r["type"]["Table"][1];
            for (k, v) in rows {
                s.ctx.tables[*index].insert(
                    from_json(&k, kt, &s.ctx.meta, &mut left, 0)?,
                    from_json(&v, vt, &s.ctx.meta, &mut left, 0)?,
                );
            }
        }
        s.ctx.version = logical.version;
        let event_names = s.ctx.meta["events"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for e in logical.outbox {
            let channel = event_names[e.channel].clone();
            let value = from_json(
                &e.value,
                &s.ctx.meta["events"][&channel],
                &s.ctx.meta,
                &mut left,
                0,
            )?;
            s.ctx.outbox.push(Event {
                commit: e.commit,
                position: e.position,
                channel,
                value,
            });
        }
        Ok(s)
    }
}
fn snapshot_schema(t: &Json, meta: &Json) -> R<portable::Schema> {
    use portable::Schema as S;
    Ok(if let Some(n) = t.as_str() {
        match n {
            "Unit" => S::Unit,
            "Bool" => S::Bool,
            "U32" => S::U32,
            "U64" => S::U64,
            "I32" => S::I32,
            "F32" => S::F32,
            "Int" => S::Int,
            "String" => S::String,
            _ => return Err("unsupported state schema".into()),
        }
    } else {
        let (kind, v) = t
            .as_object()
            .and_then(|o| o.iter().next())
            .ok_or("schema")?;
        match kind.as_str() {
            "List" => S::List(Box::new(snapshot_schema(v, meta)?)),
            "Vector" => S::Vector(
                Box::new(snapshot_schema(&v[0], meta)?),
                u8::try_from(v[1].as_u64().ok_or("vector dimension")?)
                    .map_err(|_| "vector dimension")?,
            ),
            "Option" => S::Option(Box::new(snapshot_schema(v, meta)?)),
            "Result" => S::Result(
                Box::new(snapshot_schema(&v[0], meta)?),
                Box::new(snapshot_schema(&v[1], meta)?),
            ),
            "Named" => {
                let n = v.as_str().ok_or("named schema")?;
                if meta["ids"].as_array().unwrap().iter().any(|id| id == n) {
                    S::Id
                } else if let Some(fs) = meta["records"].get(n).and_then(Json::as_array) {
                    S::Record(
                        fs.iter()
                            .map(|f| {
                                Ok((f[0].as_str().unwrap().into(), snapshot_schema(&f[1], meta)?))
                            })
                            .collect::<R<Vec<_>>>()?,
                    )
                } else {
                    S::Enum(
                        meta["enums"][n]
                            .as_array()
                            .ok_or("enum schema")?
                            .iter()
                            .map(|v| format!("{n}.{}", v.as_str().unwrap()))
                            .collect(),
                    )
                }
            }
            _ => return Err("state schema".into()),
        }
    })
}
fn snapshot_layout(meta: &Json) -> R<portable::Layout> {
    let mut roots = meta["roots"].as_array().unwrap().iter().collect::<Vec<_>>();
    roots.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Ok(portable::Layout {
        program: serde_json::from_value(meta["snapshot_program"].clone())
            .map_err(|e| e.to_string())?,
        schema: serde_json::from_value(meta["snapshot_schema"].clone())
            .map_err(|e| e.to_string())?,
        roots: roots
            .iter()
            .map(|r| {
                Ok((
                    r["name"].as_str().unwrap().into(),
                    snapshot_schema(&r["type"]["Table"][0], meta)?,
                    snapshot_schema(&r["type"]["Table"][1], meta)?,
                ))
            })
            .collect::<R<_>>()?,
        events: meta["events"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(n, t)| Ok((n.clone(), snapshot_schema(t, meta)?)))
            .collect::<R<_>>()?,
    })
}
