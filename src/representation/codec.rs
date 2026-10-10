//! Fixed source-value and ordered-table correspondence; no optimisation authority.
use ink_core::{
    logic::Term,
    representation::CheckedRepresentation,
    stateful::Value,
    syntax::{Program, Type},
    LangResult,
};
use serde_json::Value as Json;
fn construct(id: &str, constructor: usize, arguments: Vec<Term>) -> Term {
    Term::Construct {
        datatype: id.into(),
        constructor,
        arguments,
    }
}
fn type_id<'a>(checked: &'a CheckedRepresentation, t: &Type) -> LangResult<&'a str> {
    match checked
        .projection()
        .types
        .get(&serde_json::to_string(t).map_err(|e| e.to_string())?)
    {
        Some(ink_core::logic::Sort::Data(id)) => Ok(id),
        _ => Err("source datatype not projected".into()),
    }
}
pub fn encode(checked: &CheckedRepresentation, t: &Type, value: &Json) -> LangResult<Term> {
    let p = checked.module().program();
    let v = Value::from_json(value, t, p)?;
    encode_value(checked, p, t, &v, 0)
}
fn encode_value(
    c: &CheckedRepresentation,
    p: &Program,
    t: &Type,
    v: &Value,
    depth: usize,
) -> LangResult<Term> {
    if depth > 32 {
        return Err("source value depth limit".into());
    }
    let inner = |t, v| encode_value(c, p, t, v, depth + 1);
    Ok(match (t, v) {
        (Type::U64, Value::U64(x)) => Term::U64(*x),
        (Type::Bool, Value::Bool(x)) => Term::Bool(*x),
        (Type::Unit, Value::Unit) => construct(type_id(c, t)?, 0, vec![]),
        (Type::Named(n), Value::Enum(_, x)) => construct(
            type_id(c, t)?,
            p.enums[n]
                .iter()
                .position(|v| v == x)
                .ok_or("enum variant")?,
            vec![],
        ),
        (Type::Named(n), Value::Record(_, xs)) => construct(
            type_id(c, t)?,
            0,
            p.records[n]
                .iter()
                .map(|(n, t)| inner(t, &xs[n]))
                .collect::<LangResult<_>>()?,
        ),
        (Type::Option(_), Value::Option(None)) => construct(type_id(c, t)?, 0, vec![]),
        (Type::Option(t0), Value::Option(Some(v))) => {
            construct(type_id(c, t)?, 1, vec![inner(t0, v)?])
        }
        (Type::Result(a, e), Value::Result(ok, v)) => construct(
            type_id(c, t)?,
            usize::from(!ok),
            vec![inner(if *ok { a } else { e }, v)?],
        ),
        _ => return Err("unsupported representation source value".into()),
    })
}
fn data<'a>(t: &'a Term, id: &str) -> LangResult<(usize, &'a [Term])> {
    match t {
        Term::Construct {
            datatype,
            constructor,
            arguments,
        } if datatype == id => Ok((*constructor, arguments)),
        _ => Err("representation source value carrier mismatch".into()),
    }
}
pub fn decode(c: &CheckedRepresentation, t: &Type, value: &Term) -> LangResult<Json> {
    fn go(c: &CheckedRepresentation, t: &Type, value: &Term, d: usize) -> LangResult<Json> {
        if d > 32 {
            return Err("representation value depth limit".into());
        }
        let p = c.module().program();
        match (t, value) {
            (Type::U64, Term::U64(x)) => return Ok((*x).into()),
            (Type::Bool, Term::Bool(x)) => return Ok((*x).into()),
            _ => {}
        }
        let (tag, args) = data(value, type_id(c, t)?)?;
        let inner = |t, v| go(c, t, v, d + 1);
        Ok(match t {
            Type::Unit if tag == 0 && args.is_empty() => Json::Null,
            Type::Named(n) if p.enums.contains_key(n) && args.is_empty() => {
                format!("{n}.{}", p.enums[n].get(tag).ok_or("enum tag")?).into()
            }
            Type::Named(n)
                if p.records.contains_key(n) && tag == 0 && args.len() == p.records[n].len() =>
            {
                p.records[n]
                    .iter()
                    .zip(args)
                    .map(|((n, t), v)| Ok((n.clone(), inner(t, v)?)))
                    .collect::<LangResult<serde_json::Map<_, _>>>()?
                    .into()
            }
            Type::Option(_) if tag == 0 && args.is_empty() => serde_json::json!({"None":null}),
            Type::Option(t) if tag == 1 && args.len() == 1 => {
                serde_json::json!({"Some":inner(t,&args[0])?})
            }
            Type::Result(a, e) if tag < 2 && args.len() == 1 => {
                let v = inner(if tag == 0 { a } else { e }, &args[0])?;
                if tag == 0 {
                    serde_json::json!({"Ok":v})
                } else {
                    serde_json::json!({"Err":v})
                }
            }
            _ => return Err("representation source value shape mismatch".into()),
        })
    }
    go(c, t, value, 0)
}
pub fn from_tables(c: &CheckedRepresentation, tables: &[Vec<(Json, Json)>]) -> LangResult<Term> {
    let p = c.module().program();
    if tables.len() != p.states.len() {
        return Err("representation table count mismatch".into());
    }
    let mut out = vec![];
    let mut order: Vec<_> = p.states.iter().map(|s| &s.name).collect();
    order.sort();
    for (root, model) in p.states.iter().zip(&c.projection().tables) {
        let rows = &tables[order
            .iter()
            .position(|n| *n == &root.name)
            .ok_or("snapshot root missing")?];
        if rows.len() > 4096 {
            return Err("representation table execution limit".into());
        }
        let Type::Table(_, t) = &root.ty else {
            return Err("table root required".into());
        };
        let mut list = construct(&model.datatype, 0, vec![]);
        let mut prior = None;
        for (key, row) in rows {
            let key = key.as_u64().ok_or("u64 table key required")?;
            if prior.is_some_and(|x| x >= key) {
                return Err("source keys must be unique and ordered".into());
            }
            prior = Some(key);
            encode(c, t, row)?;
        }
        for (key, row) in rows.iter().rev() {
            list = construct(
                &model.datatype,
                1,
                vec![Term::U64(key.as_u64().unwrap()), encode(c, t, row)?, list],
            );
        }
        out.push(list);
    }
    Ok(construct(&c.projection().state, 0, out))
}
pub fn tables(c: &CheckedRepresentation, state: &Term) -> LangResult<Vec<Vec<(Json, Json)>>> {
    let (tag, roots) = data(state, &c.projection().state)?;
    if tag != 0 || roots.len() != c.projection().tables.len() {
        return Err("source state shape mismatch".into());
    }
    let mut all = vec![];
    for ((root, model), mut rows) in c
        .module()
        .program()
        .states
        .iter()
        .zip(&c.projection().tables)
        .zip(roots)
    {
        let Type::Table(_, t) = &root.ty else {
            return Err("table root required".into());
        };
        let mut out = vec![];
        let mut prior = None;
        loop {
            if out.len() > 4096 {
                return Err("representation table execution limit".into());
            }
            let (tag, args) = data(rows, &model.datatype)?;
            match (tag, args) {
                (0, []) => break,
                (1, [Term::U64(key), value, tail]) => {
                    if prior.is_some_and(|x| x >= *key) {
                        return Err(
                            "representation produced unordered/duplicate source keys".into()
                        );
                    }
                    prior = Some(*key);
                    out.push(((*key).into(), decode(c, t, value)?));
                    rows = tail;
                }
                _ => return Err("source rows shape mismatch".into()),
            }
        }
        all.push(out);
    }
    let p = c.module().program();
    let mut order: Vec<_> = (0..p.states.len()).collect();
    order.sort_by_key(|i| &p.states[*i].name);
    Ok(order.into_iter().map(|i| all[i].clone()).collect())
}
pub fn events(c: &CheckedRepresentation, list: &Term) -> LangResult<Vec<(usize, Json)>> {
    let channels = c.module().program().events.iter().collect::<Vec<_>>();
    let mut xs = list;
    let mut out = vec![];
    // Source event constructors are fixed by the actual source channel order.
    let entry = &c.projection().objects[&c.projection().events];
    let event_id = entry.payload["declaration"]["Datatype"]["constructors"][1]["fields"][0]["Data"]
        .as_str()
        .ok_or("source event carrier")?;
    loop {
        if out.len() > 4096 {
            return Err("representation event execution limit".into());
        }
        let (tag, args) = data(xs, &c.projection().events)?;
        match (tag, args) {
            (0, []) => break,
            (1, [event, tail]) => {
                let (channel, fields) = data(event, event_id)?;
                let (_, t) = channels.get(channel).ok_or("unknown source event")?;
                if fields.len() != 1 {
                    return Err("source event payload".into());
                }
                out.push((channel, decode(c, t, &fields[0])?));
                xs = tail;
            }
            _ => return Err("source event list shape".into()),
        }
    }
    Ok(out)
}
