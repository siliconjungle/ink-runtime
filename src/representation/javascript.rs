//! Assembly for already checked definitions. Target bodies belong to the backend.
use ink_core::{
    representation::{CheckedMigration, CheckedRepresentation},
    LangResult,
};
use serde_json::json;
use std::collections::BTreeMap;
pub fn emit(
    representations: &[CheckedRepresentation],
    migrations: &[CheckedMigration],
) -> LangResult<String> {
    if representations.is_empty() || representations.len() > 16 || migrations.len() > 16 {
        return Err("representation bundle limit".into());
    }
    let source = representations[0].module().identity()?;
    if representations
        .iter()
        .any(|r| r.module().identity().ok().as_ref() != Some(&source))
        || migrations.iter().any(|m| m.source_identity() != source)
    {
        return Err("representation bundle changes source".into());
    }
    let mut output = ink_lowering_js::lower(representations[0].module())?.javascript;
    let mut plans = BTreeMap::new();
    let mut bridges = BTreeMap::new();
    let mut bindings = Vec::new();
    for r in representations {
        let p = r.module().program();
        let projection = r.projection();
        let id = &r.request().representation;
        let emission = ink_lowering_js::definition::lower(r.definition())?;
        let name = format!("__representation_definition_{}", bindings.len());
        output.push_str(&format!(
            "const {name}=(()=>{{{};return invokeNative;}})();\n",
            emission.javascript.replace("export function ", "function ")
        ));
        bindings.push(format!("{}:{name}", serde_json::to_string(id).unwrap()));
        let event = projection.objects[&projection.events].payload["declaration"]["Datatype"]
            ["constructors"][1]["fields"][0]["Data"]
            .clone();
        let mut actions = BTreeMap::new();
        for a in &p.actions {
            let plan = &r.plan().actions[&a.name];
            let observation = &projection.actions[&a.name].result;
            let ink_core::logic::Sort::Data(observation) = observation else {
                return Err("action observation sort".into());
            };
            actions.insert(a.name.clone(),json!({"params":a.params.iter().map(|(_,t)|t).collect::<Vec<_>>(),"result":a.result,"execute":plan.execute,"observe":plan.observe,"next":plan.next,"observation":observation}));
        }
        plans.insert(id.clone(),json!({"id":id,"source":source,"encode":r.plan().encode,"decode":r.plan().decode,"types":projection.types,"state":projection.state,"events":projection.events,"event":event,"tables":projection.tables,"actions":actions,"records":p.records,"enums":p.enums,"channels":p.events.iter().collect::<Vec<_>>()}));
    }
    for m in migrations {
        let emission = ink_lowering_js::definition::lower(m.definition())?;
        let id = ink_core::registry::identity(&(
            m.source_identity(),
            m.endpoints(),
            m.function(),
            m.definition().bundle_sha256(),
        ))?;
        let name = format!("__representation_definition_{}", bindings.len());
        output.push_str(&format!(
            "const {name}=(()=>{{{};return invokeNative;}})();\n",
            emission.javascript.replace("export function ", "function ")
        ));
        bindings.push(format!("{}:{name}", serde_json::to_string(&id).unwrap()));
        bridges.insert(id,json!({"source":m.source_identity(),"from":m.endpoints().0,"to":m.endpoints().1,"function":m.function()}));
    }
    let ids = representations
        .iter()
        .map(|r| r.request().representation.clone())
        .collect::<Vec<_>>();
    output.push_str(&format!("const __representationPlans={};const __representationMigrations={};const __representationIds={};const __representationExecutors=Object.freeze({{{}}});\nexport const representationMetadata=Object.freeze({{representations:__representationIds,migrations:Object.keys(__representationMigrations)}});\n",serde_json::to_string(&plans).map_err(|e|e.to_string())?,serde_json::to_string(&bridges).map_err(|e|e.to_string())?,serde_json::to_string(&ids).unwrap(),bindings.join(",")));
    output.push_str(include_str!("../../hosts/representation.mjs"));
    Ok(output)
}
