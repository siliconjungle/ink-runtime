//! Host transport for literal independently emitted definition artifacts.
//! Registration is a host trust decision, like any other Executor implementation.
use super::Executor;
use ink_core::{
    definition::CheckedDefinition,
    logic::{Context, Term},
    LangResult,
};
use serde_json::{json, Value};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};
struct Artifact {
    manifest: Value,
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    words: bool,
}
impl Drop for Artifact {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
pub struct Compiled {
    artifacts: RefCell<BTreeMap<String, Artifact>>,
}
impl Compiled {
    pub fn new() -> Self {
        Self {
            artifacts: RefCell::new(BTreeMap::new()),
        }
    }
    pub fn register(
        &mut self,
        manifest: Value,
        command: &mut Command,
        words: bool,
    ) -> LangResult<()> {
        let identity = manifest["executable"]
            .as_str()
            .ok_or("missing compiled executable identity")?
            .to_owned();
        if identity.len() != 64 || manifest["exports"].as_array().is_none() {
            return Err("invalid compiled definition manifest".into());
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| e.to_string())?;
        let input = child.stdin.take().ok_or("compiled stdin")?;
        let output = BufReader::new(child.stdout.take().ok_or("compiled stdout")?);
        if self.artifacts.get_mut().contains_key(&identity) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("duplicate compiled executable".into());
        }
        self.artifacts.get_mut().insert(
            identity,
            Artifact {
                manifest,
                child,
                input,
                output,
                words,
            },
        );
        Ok(())
    }
}
impl Default for Compiled {
    fn default() -> Self {
        Self::new()
    }
}
impl Executor for Compiled {
    fn execute(
        &self,
        definition: &CheckedDefinition,
        _context: &Context,
        function: &str,
        args: Vec<Term>,
    ) -> LangResult<Term> {
        let identity = ink_core::registry::identity(&(
            "ink-sealed-executable-v1",
            definition.bundle_sha256(),
            definition.exports(),
            definition.choices(),
        ))?;
        let mut artifacts = self.artifacts.borrow_mut();
        let artifact = artifacts
            .get_mut(&identity)
            .ok_or("compiled executor does not contain pinned definition")?;
        let exports = artifact.manifest["exports"]
            .as_array()
            .ok_or("compiled export table")?;
        let index = exports
            .iter()
            .position(|id| id.as_str() == Some(function))
            .ok_or("unexported compiled definition")?;
        if artifact.words {
            let input = encode_words(&artifact.manifest, &args)?;
            artifact
                .input
                .write_all(&(index as u32).to_le_bytes())
                .and_then(|_| {
                    artifact
                        .input
                        .write_all(&(input.len() as u32).to_le_bytes())
                })
                .map_err(|e| e.to_string())?;
            for word in &input {
                artifact
                    .input
                    .write_all(&word.to_le_bytes())
                    .map_err(|e| e.to_string())?;
            }
            artifact.input.flush().map_err(|e| e.to_string())?;
            let mut length = [0; 4];
            artifact
                .output
                .read_exact(&mut length)
                .map_err(|e| e.to_string())?;
            let length = u32::from_le_bytes(length) as usize;
            if length == 0 || length > 4 * 1024 * 1024 {
                return Err("compiled definition execution/resource failure".into());
            }
            let mut out = vec![0u32; length];
            for word in &mut out {
                let mut bytes = [0; 4];
                artifact
                    .output
                    .read_exact(&mut bytes)
                    .map_err(|e| e.to_string())?;
                *word = u32::from_le_bytes(bytes);
            }
            decode_words(&artifact.manifest, &out)
        } else {
            let wire = args.iter().map(term_wire).collect::<LangResult<Vec<_>>>()?;
            serde_json::to_writer(
                &mut artifact.input,
                &json!({"function":function,"args":wire}),
            )
            .map_err(|e| e.to_string())?;
            artifact
                .input
                .write_all(b"\n")
                .and_then(|_| artifact.input.flush())
                .map_err(|e| e.to_string())?;
            let mut text = String::new();
            if artifact
                .output
                .by_ref()
                .take(16 * 1024 * 1024)
                .read_line(&mut text)
                .map_err(|e| e.to_string())?
                == 0
            {
                return Err("compiled definition closed its channel".into());
            }
            let response: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            if let Some(error) = response["error"].as_str() {
                return Err(error.into());
            }
            wire_term(&response["ok"], 0, &mut 100000)
        }
    }
}
pub fn term_wire(t: &Term) -> LangResult<Value> {
    Ok(match t {
        Term::Bool(x) => json!({"Bool":x}),
        Term::U64(x) => json!({"U64":x.to_string()}),
        Term::Construct {
            datatype,
            constructor,
            arguments,
        } => {
            json!({"Data":{"datatype":datatype,"constructor":constructor,"arguments":arguments.iter().map(term_wire).collect::<LangResult<Vec<_>>>()?}})
        }
        _ => return Err("definition transport requires ground values".into()),
    })
}
pub fn wire_term(v: &Value, depth: usize, budget: &mut usize) -> LangResult<Term> {
    if depth > 64 || *budget == 0 {
        return Err("definition value resource limit".into());
    }
    *budget -= 1;
    let object = v.as_object().ok_or("definition value object")?;
    if object.len() != 1 {
        return Err("definition value tag".into());
    }
    if let Some(v) = object.get("Bool") {
        return Ok(Term::Bool(v.as_bool().ok_or("definition Bool")?));
    }
    if let Some(v) = object.get("U64") {
        return Ok(Term::U64(
            v.as_str()
                .ok_or("exact definition U64")?
                .parse()
                .map_err(|_| "definition U64 overflow")?,
        ));
    }
    let data = object
        .get("Data")
        .and_then(Value::as_object)
        .ok_or("definition datatype")?;
    if data.len() != 3 {
        return Err("definition datatype fields".into());
    }
    let datatype = data["datatype"].as_str().ok_or("datatype identity")?;
    let constructor = data["constructor"]
        .as_u64()
        .and_then(|x| usize::try_from(x).ok())
        .ok_or("definition constructor")?;
    let arguments = data["arguments"].as_array().ok_or("definition arguments")?;
    if datatype.len() != 64 || arguments.len() > 64 {
        return Err("definition constructor limits".into());
    }
    Ok(Term::Construct {
        datatype: datatype.into(),
        constructor,
        arguments: arguments
            .iter()
            .map(|v| wire_term(v, depth + 1, budget))
            .collect::<LangResult<_>>()?,
    })
}
pub fn encode_words(manifest: &Value, args: &[Term]) -> LangResult<Vec<u32>> {
    let ids = manifest["datatypes"]
        .as_array()
        .ok_or("compiled datatype table")?;
    let mut body = Vec::new();
    let mut count = 0u32;
    fn node(
        t: &Term,
        ids: &[Value],
        body: &mut Vec<u32>,
        count: &mut u32,
        depth: usize,
    ) -> LangResult<u32> {
        if depth > 64 || *count >= 100000 {
            return Err("definition input resource limit".into());
        }
        let (tag, bits, datatype, constructor, arguments) = match t {
            Term::Bool(v) => (0, u64::from(*v), 0, 0, vec![]),
            Term::U64(v) => (1, *v, 0, 0, vec![]),
            Term::Construct {
                datatype,
                constructor,
                arguments,
            } => {
                if arguments.len() > 64 {
                    return Err("definition field limit".into());
                }
                let id = ids
                    .iter()
                    .position(|x| x.as_str() == Some(datatype))
                    .ok_or("unknown input datatype")?;
                let arguments = arguments
                    .iter()
                    .map(|a| node(a, ids, body, count, depth + 1))
                    .collect::<LangResult<Vec<_>>>()?;
                (
                    2,
                    0,
                    id as u32,
                    u32::try_from(*constructor).map_err(|_| "constructor index")?,
                    arguments,
                )
            }
            _ => return Err("definition transport requires ground values".into()),
        };
        if *count >= 100000 {
            return Err("definition input node limit".into());
        }
        body.extend([
            tag,
            bits as u32,
            (bits >> 32) as u32,
            datatype,
            constructor,
            arguments.len() as u32,
        ]);
        body.extend(arguments);
        let index = *count;
        *count += 1;
        Ok(index)
    }
    if args.len() > 64 {
        return Err("definition argument limit".into());
    }
    let roots = args
        .iter()
        .map(|a| node(a, ids, &mut body, &mut count, 0))
        .collect::<LangResult<Vec<_>>>()?;
    let mut out = vec![count, args.len() as u32];
    out.extend(roots);
    out.extend(body);
    if out.len() > 4 * 1024 * 1024 {
        return Err("definition word limit".into());
    }
    Ok(out)
}
pub fn decode_words(manifest: &Value, input: &[u32]) -> LangResult<Term> {
    let ids = manifest["datatypes"]
        .as_array()
        .ok_or("compiled datatype table")?;
    if input.len() < 3 || input[1] != 1 || input[0] > 100000 || input[2] >= input[0] {
        return Err("definition response header".into());
    }
    let mut values: Vec<Option<Term>> = Vec::new();
    let mut depths: Vec<usize> = Vec::new();
    let mut position = 3;
    for i in 0..input[0] as usize {
        let header = input
            .get(position..position + 6)
            .ok_or("definition response node")?;
        let count = header[5] as usize;
        if count > 64 {
            return Err("definition response fields".into());
        }
        let children = input
            .get(position + 6..position + 6 + count)
            .ok_or("definition response children")?;
        if children.iter().any(|x| *x as usize >= i) {
            return Err("forward definition response child".into());
        }
        let bits = u64::from(header[1]) | (u64::from(header[2]) << 32);
        let depth = children
            .iter()
            .map(|x| depths[*x as usize] + 1)
            .max()
            .unwrap_or(0);
        if depth > 64 {
            return Err("definition response depth".into());
        }
        let value = match header[0] {
            0 if bits <= 1 && count == 0 && header[3] == 0 && header[4] == 0 => {
                Term::Bool(bits != 0)
            }
            1 if count == 0 && header[3] == 0 && header[4] == 0 => Term::U64(bits),
            2 if bits == 0 => Term::Construct {
                datatype: ids
                    .get(header[3] as usize)
                    .and_then(Value::as_str)
                    .ok_or("definition response datatype")?
                    .into(),
                constructor: header[4] as usize,
                arguments: children
                    .iter()
                    .map(|x| {
                        values[*x as usize]
                            .take()
                            .ok_or("aliased definition response child".into())
                    })
                    .collect::<LangResult<_>>()?,
            },
            _ => return Err("definition response node tag".into()),
        };
        values.push(Some(value));
        depths.push(depth);
        position += 6 + count;
    }
    if position != input.len() {
        return Err("definition response trailing words".into());
    }
    let root = values
        .get_mut(input[2] as usize)
        .and_then(Option::take)
        .ok_or("invalid definition response root")?;
    if values.iter().any(Option::is_some) {
        return Err("unused definition response nodes".into());
    }
    Ok(root)
}
pub const C_RUNNER: &str = include_str!("../../hosts/definition-c-runner.c");
pub const JAVASCRIPT_RUNNER: &str = include_str!("../../hosts/definition-js-runner.mjs");
pub const WASM_RUNNER: &str = include_str!("../../hosts/definition-wasm-runner.mjs");

/// Target identity is explicit; registration never substitutes reference execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    C,
    Rust,
    JavaScript,
    CWasm,
    RustWasm,
}
impl Target {
    pub fn name(self) -> &'static str {
        match self {
            Self::C => "c",
            Self::Rust => "rust",
            Self::JavaScript => "javascript",
            Self::CWasm => "c-wasm",
            Self::RustWasm => "rust-wasm",
        }
    }
    pub fn parse(name: &str) -> LangResult<Self> {
        match name {
            "c" => Ok(Self::C),
            "rust" => Ok(Self::Rust),
            "javascript" => Ok(Self::JavaScript),
            "c-wasm" => Ok(Self::CWasm),
            "rust-wasm" => Ok(Self::RustWasm),
            _ => {
                Err("representation target must be c, rust, javascript, c-wasm or rust-wasm".into())
            }
        }
    }
}
/// Compile immutable literal definitions and register their persistent process.
/// All commands use argv, and no source or database bytes select a command.
pub fn prepare(
    executor: &mut Compiled,
    definition: &CheckedDefinition,
    target: Target,
    directory: &std::path::Path,
    zig: Option<&str>,
) -> LangResult<()> {
    use std::fs;
    let identity = ink_core::registry::identity(&(
        "ink-sealed-executable-v1",
        definition.bundle_sha256(),
        definition.exports(),
        definition.choices(),
    ))?;
    if let Some(artifact) = executor.artifacts.get_mut().get(&identity) {
        return if artifact.manifest["host_target"] == target.name() {
            Ok(())
        } else {
            Err("definition is already registered for a different target".into())
        };
    }
    let directory = directory.join(target.name());
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let directory = directory.canonicalize().map_err(|e| e.to_string())?;
    let stem = directory.join(identity);
    fn run(command: &mut Command) -> LangResult<()> {
        let output = command.output().map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).into_owned());
        }
        Ok(())
    }
    let (mut manifest, words, mut command) = match target {
        Target::JavaScript => {
            let e = ink_lowering_js::portable_definition::lower(definition)?;
            fs::write(stem.with_extension("mjs"), e.javascript).map_err(|e| e.to_string())?;
            let runner = directory.join("definition-js-runner.mjs");
            fs::write(&runner, JAVASCRIPT_RUNNER).map_err(|e| e.to_string())?;
            let mut command = Command::new("node");
            command.arg(runner).arg(stem.with_extension("mjs"));
            (e.manifest, false, command)
        }
        Target::C | Target::CWasm => {
            let e = ink_lowering_c::portable_definition::lower(definition)?;
            let source = stem.with_extension("c");
            fs::write(
                &source,
                if target == Target::C {
                    e.c + C_RUNNER
                } else {
                    e.c
                },
            )
            .map_err(|e| e.to_string())?;
            let mut command;
            if target == Target::C {
                run(Command::new("cc")
                    .args(["-std=c11", "-O2"])
                    .arg(&source)
                    .arg("-o")
                    .arg(&stem))?;
                command = Command::new(&stem);
            } else {
                run(
                    Command::new(zig.ok_or("C-Wasm representation needs --zig PATH")?)
                        .args([
                            "cc",
                            "-target",
                            "wasm32-freestanding",
                            "-O2",
                            "-nostdlib",
                            "-Wl,--no-entry",
                            "-Wl,--export=ink_definition_input",
                            "-Wl,--export=ink_definition_result",
                            "-Wl,--export=ink_definition_call",
                        ])
                        .arg(&source)
                        .arg("-o")
                        .arg(stem.with_extension("wasm")),
                )?;
                let runner = directory.join("definition-wasm-runner.mjs");
                fs::write(&runner, WASM_RUNNER).map_err(|e| e.to_string())?;
                command = Command::new("node");
                command.arg(runner).arg(stem.with_extension("wasm"));
            }
            (e.manifest, true, command)
        }
        Target::Rust | Target::RustWasm => {
            let (e, manifest) = ink_lowering_rust::definition_native::lower_portable(definition)?;
            let source = stem.with_extension("rs");
            fs::write(
                &source,
                if target == Target::Rust {
                    e.source + include_str!("../../hosts/definition-rust-runner.rs")
                } else {
                    e.source
                },
            )
            .map_err(|e| e.to_string())?;
            let mut build = Command::new("rustc");
            build
                .args([
                    "--edition=2021",
                    "-O",
                    "--crate-name",
                    "ink_literal_definition",
                ])
                .arg(source);
            let mut command;
            if target == Target::Rust {
                build.arg("-o").arg(&stem);
                run(&mut build)?;
                command = Command::new(&stem);
            } else {
                build
                    .args(["--crate-type=cdylib", "--target", "wasm32-unknown-unknown"])
                    .arg("-o")
                    .arg(stem.with_extension("wasm"));
                run(&mut build)?;
                let runner = directory.join("definition-wasm-runner.mjs");
                fs::write(&runner, WASM_RUNNER).map_err(|e| e.to_string())?;
                command = Command::new("node");
                command.arg(runner).arg(stem.with_extension("wasm"));
            }
            (manifest, true, command)
        }
    };
    manifest["host_target"] = json!(target.name());
    fs::write(
        stem.with_extension("manifest.json"),
        serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    executor.register(manifest, &mut command, words)
}
