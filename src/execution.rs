//! Backend-neutral execution of already checked pure source graphs.
//! Adapters own physical execution and bridges; profiles never grant proof authority.
use ink_core::{
    eval::Value as InkValue,
    source_routing::{CheckedRouting, Literal, ValueRef},
    LangResult,
};
use serde_json::Value;
use std::collections::BTreeMap;
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Backend {
    pub package: String,
    pub domain: String,
}
/// An adapter invokes a literal function of the plan's selected module.
/// Registration is a host trust decision; the core does not certify target code.
pub trait Adapter {
    fn call(&mut self, function: &str, arguments: &[Value]) -> LangResult<Value>;
}
#[derive(Debug)]
pub struct Placement {
    pub stage: String,
    pub backend: Backend,
    pub fallback_reason: Option<String>,
}
#[derive(Debug)]
pub struct Outcome {
    pub value: Value,
    pub placements: Vec<Placement>,
}
pub fn execute(
    route: &CheckedRouting,
    inputs: &[Value],
    adapters: &mut BTreeMap<Backend, Box<dyn Adapter>>,
    fallback: &Backend,
) -> LangResult<Outcome> {
    let p = route.module().pure_program()?;
    let package = route.package();
    let entry = p
        .functions
        .iter()
        .find(|f| f.name == package.entry)
        .ok_or("unknown route entry")?;
    if inputs.len() != entry.params.len() {
        return Err("route input arity mismatch".into());
    }
    // Capture and type-check all original values before any adapter is invoked.
    let inputs = inputs
        .iter()
        .zip(&entry.params)
        .map(|(v, (_, t))| InkValue::from_program_json(v, t, p).map(|v| v.json()))
        .collect::<LangResult<Vec<_>>>()?;
    let mut values = BTreeMap::new();
    let mut placements = vec![];
    let resolve = |r: &ValueRef, values: &BTreeMap<String, Value>| -> LangResult<Value> {
        match r {
            ValueRef::Input(i) => inputs.get(*i).cloned().ok_or("route input index".into()),
            ValueRef::Stage(id) => values.get(id).cloned().ok_or("missing route stage".into()),
            ValueRef::Literal(Literal::U32(v)) => Ok(serde_json::json!(v)),
            ValueRef::Literal(Literal::U64(v)) => Ok(serde_json::json!(v)),
            ValueRef::Literal(Literal::Bool(v)) => Ok(serde_json::json!(v)),
        }
    };
    for stage in &package.stages {
        let args = stage
            .arguments
            .iter()
            .map(|v| resolve(v, &values))
            .collect::<LangResult<Vec<_>>>()?;
        let function = p
            .functions
            .iter()
            .find(|f| f.name == stage.function)
            .ok_or("route function missing")?;
        let wanted = Backend {
            package: stage.backend.clone(),
            domain: stage.domain.clone(),
        };
        let invoke = |backend: &Backend,
                      adapters: &mut BTreeMap<Backend, Box<dyn Adapter>>|
         -> LangResult<Value> {
            let value = adapters
                .get_mut(backend)
                .ok_or("execution adapter unavailable")?
                .call(&stage.function, &args)?;
            InkValue::from_program_json(&value, &function.result, p).map(|v| v.json())
        };
        let (value, backend, reason) = match invoke(&wanted, adapters) {
            Ok(v) => (v, wanted, None),
            Err(error) if &wanted != fallback => {
                (invoke(fallback, adapters)?, fallback.clone(), Some(error))
            }
            Err(error) => return Err(error),
        };
        values.insert(stage.id.clone(), value);
        placements.push(Placement {
            stage: stage.id.clone(),
            backend,
            fallback_reason: reason,
        });
    }
    Ok(Outcome {
        value: resolve(&package.output, &values)?,
        placements,
    })
}
