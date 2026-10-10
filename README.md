# Ink runtime

Shared execution orchestration for [Ink](https://github.com/siliconjungle/ink-lang). Backend-neutral checked-graph execution, scheduling, profiling, bundle assembly, CPU fallback and host ABIs live here. Search lives in [ink-planner](https://github.com/siliconjungle/ink-planner); semantics and proof authority stay in `ink-core`.

`execution::execute` accepts only the core's immutable checked source-routing witness, captures and validates original inputs before invoking adapters, composes typed results, and falls back by repeating the same pure stage. `execution::Adapter` registers execution providers under opaque package/domain names. Adapter registration, generated code and physical bridges remain host trust decisions; typed output validation is not a backend correctness proof. Stateful/effectful stages are not speculatively replayed.

`emit_gpu`, `emit_selected` and `emit_routed` assemble target artifacts with C CPU fallback and versioned browser/native hosts. WGSL emission and WebGPU/wgpu device operations remain in `ink-lowering-gpu`, which has no dependency on the C lowerer or runtime. The device package supplies adapters for `ink-scalar-pipeline-v1` and `ink-packed-pipeline-v2`; native device code consumes the host's typed ABI, and browser adapters receive its codecs and limits explicitly. The generated bundle pins target/toolchain dependencies and shader/CPU identities.

`hosts/` owns both browser and native scheduling, profiling and fallback. Whole-call/pipeline measurements include upload, allocation, dispatch and readback; setup is amortised over bounded reuse. Device loss, unsupported capabilities and driver validation failures retain compiled CPU execution. GPU residency within supported pipelines stays under device-adapter control. Cross-function scalar routes currently transport word/Bool/list values through the host; packed compute pipelines provide resident array composition. Broader bridge protocols must be implemented and checked explicitly.

`toolchain::wasm` invokes the Wasm target configuration from `ink-lowering-wasm`. The Wasm state host adapter also lives here; Rust/LLVM remains the state target compiler. Runtime scheduling never grants proof authority, and measured profitability is not a mathematical premise.

The Ink distribution pins this repository and tests integrated lowering/execution. Standalone `cargo test` also checks backend-neutral composition and pure fallback.
