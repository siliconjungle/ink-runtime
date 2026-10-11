# Ink runtime

Shared execution orchestration for [Ink](https://github.com/siliconjungle/ink-lang). Backend-neutral checked-graph execution, scheduling, profiling, bundle assembly, CPU fallback and host ABIs live here. Search lives in [ink-planner](https://github.com/siliconjungle/ink-planner); semantics and proof authority stay in `ink-core`.

`execution::execute` accepts only the core's immutable checked source-routing witness, captures and validates original inputs before invoking adapters, composes typed results, and falls back by repeating the same pure stage. `execution::Adapter` registers execution providers under opaque package/domain names. Adapter registration, generated code and physical bridges remain host trust decisions; typed output validation is not a backend correctness proof. Stateful/effectful stages are not speculatively replayed.

`emit_gpu`, `emit_selected` and `emit_routed` assemble target artifacts with C CPU fallback and versioned browser/native hosts. WGSL emission and WebGPU/wgpu device operations remain in `ink-lowering-gpu`, which has no dependency on the C lowerer or runtime. The device package supplies adapters for `ink-scalar-pipeline-v1` and `ink-packed-pipeline-v2`; native device code consumes the host's typed ABI, and browser adapters receive its codecs and limits explicitly. The generated bundle pins target/toolchain dependencies and shader/CPU identities.

`hosts/` owns both browser and native scheduling, profiling and fallback. Whole-call/pipeline measurements include upload, allocation, dispatch and readback; setup is amortised over bounded reuse. Device loss, unsupported capabilities and driver validation failures retain compiled CPU execution. GPU residency within supported pipelines stays under device-adapter control. Cross-function scalar routes currently transport word/Bool/list values through the host; packed compute pipelines provide resident array composition. Broader bridge protocols must be implemented and checked explicitly.

`toolchain::wasm` invokes the Wasm target configuration from `ink-lowering-wasm`. The Wasm state host adapter also lives here; Rust/LLVM remains the state target compiler. Runtime scheduling never grants proof authority, and measured profitability is not a mathematical premise.

The Ink distribution pins this repository and tests integrated lowering/execution. Standalone `cargo test` also checks backend-neutral composition and pure fallback.

Browser bundles also include literal `program.mjs` emission from the pinned
`ink-lowering-js` package. Both scalar calls and packed pipelines accept explicit
`javascript`, `cpu` (Wasm), `gpu` or `auto` selection. `backend-selection.mjs`
compares eligible pure candidates with bounded, alternating measurements of
whole execution, including conversions, transfers and result materialisation.
Initial JavaScript/Wasm module setup is charged once and amortised over expected
reuse; later profiles measure warm execution. GPU preparation remains explicit.
Unavailable/failed candidates retain the compiled CPU baseline. Profiling caches
hold at most 32 shapes and recheck after 32 uses by default. Profile receipts are
observations, not proofs. Native wgpu bundles continue to use their C CPU path.
Mixed modules expose complete Wasm state execution and explicit JavaScript state instances. Stateful actions never enter speculative selection. Per-stage adaptive JS/Wasm/GPU placement remains future work.

Run browser policy checks with `node --test tests/backend-selection.mjs`.

`emit_c_program` and `emit_rust_program` assemble complete literal programs with
primitive values, transaction publication, portable snapshot codecs and a shared
native/Wasm handle/buffer ABI. C emission uses shared Rust primitives; generated
programs contain no Ink parser or AST evaluator. `toolchain::complete_program`
builds native executables or Wasm modules. `module` extracts GPU-eligible pure
closures from mixed programs and retains persistent device/profile engines.
The C header is `hosts/state-abi.h`; browser Wasm support is `hosts/state-wasm.mjs`.

`hosts/durable-state.mjs` provides serialized durable calls and bounded retry
receipts over generated JavaScript or the complete C/Rust Wasm ABI. Storage
adapters are Node's atomic synced file replacement (`durable-file.mjs`, one
writer process) and IndexedDB (transactional compare-and-replace). Ordered
outbox delivery is at least once; receivers deduplicate event coordinates.
An uncertain write requires reopening before further execution.
`durable::{CompiledState, Host, Store, FileStore}` supplies native embedding;
complete C/Rust projects and mixed native wgpu bundles share this host/CLI and accept
`SCRIPT.json --durable FILE`. Native receipts and outbox acknowledgements survive
process death after publication but before reply. Complete snapshots are copied
per call; this is a durability baseline, not a performance claim. See the distribution's
[durable host contract](https://github.com/siliconjungle/ink-lang/blob/main/docs/durable-host.md).

Run `node --test tests/durable-state.mjs` for failure-injection and file checks.

`emit_c_program_selected`, `emit_selected` and `module::emit_selected` keep the
core's sealed selection witness during complete/native/Wasm and mixed-module
assembly. Selected C and JavaScript state use `snapshot::selected_layout` from
the core; Rust state code comes from its selected emitter. Baseline checkpoints
and durable files remain usable by checked plans for the same source. Raw
program emission keeps its own namespace, and no identity override is accepted.
CLI `emit-state` uses `COMPLETE_STATE_RUNNER` and `emit_native_state_host`; the
specialised `STATE_RUNNER` remains available for direct state-emitter harnesses.

Mixed native bundles dispatch eligible pure calls independently of durable transactions. GPU calls do not publish state or retry receipts; poisoned hosts reject dispatch until reopened. Both native runners share `hosts/native/state-cli.rs`.

Source-bound representation codecs also transport supported List values through
Nil/Cons datatypes, preserving nested element types and enforcing depth/length
limits. Native definition execution retains its separate resource limits.
See the Ink distribution docs/point-query-delta.md for tested domains and bounds.
