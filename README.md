# Ink runtime

Shared execution orchestration for [Ink](https://github.com/siliconjungle/ink-lang). Backend-neutral checked-graph execution, scheduling, common decision application, bundle assembly, CPU fallback and host ABIs live here. Search lives in [ink-planner](https://github.com/siliconjungle/ink-planner); semantics and proof authority stay in `ink-core`.

`execution::execute` accepts only the core's immutable checked source-routing witness, captures and validates original inputs before invoking adapters, composes typed results, and falls back by repeating the same pure stage. `execution::Adapter` registers execution providers under opaque package/domain names. Adapter registration, generated code and physical bridges remain host trust decisions; typed output validation is not a backend correctness proof. Stateful/effectful stages are not speculatively replayed.

`emit_gpu`, `emit_selected` and `emit_routed` assemble target artifacts with C CPU fallback and versioned browser/native hosts. WGSL emission and WebGPU/wgpu device operations remain in `ink-lowering-gpu`, which has no dependency on the C lowerer or runtime. The device package supplies adapters for `ink-scalar-pipeline-v1` and `ink-packed-pipeline-v2`; native device code consumes the host's typed ABI, and browser adapters receive its codecs and limits explicitly. The generated bundle pins target/toolchain dependencies and shader/CPU identities.

`hosts/` owns both browser and native scheduling, decision application and fallback. Offline cost observations cover complete calls, including upload, allocation, dispatch and readback. The planner fits scope-bound models; hosts execute one decision from the shared ABI contract. Device loss, unsupported capabilities and driver validation failures retain compiled CPU execution. GPU residency within supported pipelines stays under device-adapter control. Cross-function scalar routes currently transport word/Bool/list values through the host; packed compute pipelines provide resident array composition. Broader bridge protocols must be implemented and checked explicitly.

`toolchain::wasm` invokes the Wasm target configuration from `ink-lowering-wasm`. The Wasm state host adapter also lives here; Rust/LLVM remains the state target compiler. Runtime scheduling never grants proof authority, and measured profitability is not a mathematical premise.

The Ink distribution pins this repository and tests integrated lowering/execution. Standalone `cargo test` also checks backend-neutral composition and pure fallback.

Browser bundles also include trusted `program.mjs` emission from the pinned
`ink-lowering-js` package. Both scalar calls and packed pipelines accept explicit
`javascript`, `cpu` (Wasm), `gpu` or `auto` selection. `backend-selection.mjs` separates execution policy from language semantics.
The default `policy:'adaptive'` uses an optional saved `costProfile` bound to a
`targetKey`, exact emitted-code scope and a declared workload domain. It performs
one chosen execution and no online trials. Without comparable CPU/alternative
profile evidence it retains the compiled CPU baseline. `policy:'static'` fixes
`staticBackend:'cpu'|'javascript'|'gpu'`; explicit backend requests override the
policy. Static JS calls avoid packed transport and Wasm initialization.
Both native and browser hosts use `ink-abi`'s single decision implementation;
browser hosts execute its Rust through Wasm. Workload feature aggregation also
uses that implementation. Online profiling/trial policy has been removed.
Measurements and fitted profiles belong in knowledge and planner tooling.
Profiles bind the target, exact code scope, transport boundary and observed domain.
They never establish correctness, valid types, ranges or source equality.
Native wgpu bundles execute their C CPU fallback. An unavailable candidate or
runtime/device failure retains CPU execution.
Mixed modules expose complete Wasm state execution and explicit JavaScript state instances. Stateful actions never enter speculative selection.

Packed browser pipelines support saved-profile partition/target selection through
an external planner asset. The distribution installs the pinned planner with
`install_pipeline_planner`; standalone runtime assembly contains physical
`pipeline-contract.mjs`, scheduling/workers and profiling tools, with no search.
Explicit partitions work without a planner. `placementProfile`, `parallelism`
(0..16) and `expectedCalls` enable bounded mixed JS/Wasm/WebGPU placement and pure
worker overlap. Contiguous GPU groups retain interiors; whole-GPU feedback retains
its device batch path. Typed bridges preserve captured input and owned output
lifetimes. Failed tasks replay only pure groups on CPU and drain before close.
Cold alternatives require measured setup until ready; unknown domains or absent
CPU comparisons keep baseline. See the distribution's `docs/adaptive-execution.md`
and `reports/adaptive-placement-2026-10-12` for the completed contract and gates.

The JavaScript value ABI trusts the caller, like native compiled code: arguments
must have the declared types and ranges, with U64 supplied as BigInt. Generated
function exports do no input checking, execution-budget counting or defensive
copying for word values. A source identity can return the caller's same array or
record; explicit collection operations remain nonmutating. F32 boundary
conversion preserves rounded numbers and explicit `F32Bits` values.

Browser hosts bind these generated functions once when loading the module.
Explicit JavaScript calls bypass packed transport, generic pipeline construction
and profiling keys, and do not load the Wasm fallback. JavaScript pipelines pass
native values between stages rather than packing, unpacking and recapturing each
intermediate result. They remain serialized, so callers must keep borrowed
JavaScript inputs stable until the returned promise settles. Wasm/GPU calls still
snapshot inputs for transport; flat U32 typed arrays use one bulk clone. CPU
fallback loads lazily when needed. Profiled automatic calls capture native
values before yielding, derive workload descriptors without packing, and let
the shared selector choose before materializing Wasm/GPU transport. Finite F32
inputs round once into native numbers; nonfinite tags retain exact bits. U32
captures also supply their transport words. Packed-host identity results become fresh public
Arrays, and explicit JavaScript identity calls retain their borrowed-input
contract. Typed logical lengths, shared inputs and caller mutation after
submission retain the same captured semantics. Graph configuration, device limits and result
layout invariants remain host responsibilities. The packed host retains its
JSON-compatible U64 output convention (safe Number, otherwise decimal string).

When the shared selector establishes a profile-free or static CPU choice at
engine load, ordinary automatic calls use the direct captured CPU path. They
do not rebuild profiling keys or run workload-feature encoding on every call.
Whole-call profiles use `ink-complete-call-v2` scopes, including the runtime host
identity; earlier transport prices cannot select the changed host implicitly.

Packed compute engines also expose `const call = engine.prepareCall(name,args)`.
Preparation synchronously owns one immutable typed snapshot and checked call
topology. `await call.run(options)` evaluates fresh primals, selects normally,
and returns detached complete results. Selected CPU/GPU transport and workload
descriptors can be retained across runs. `await call.close()` stops new runs,
drains accepted work, and releases captures. Changing model inputs require new
preparation. Prepared calls have their own `ink-prepared-call-v1` profile scope
around the ordinary scope; setup must be measured separately and ordinary-call
profiles cannot price this lifetime. This API is distinct from borrowed-output
`prepareCpuCall` and resident `prepareGpuPipeline`.

Run host checks with `node --test tests/*.mjs`. The distribution browser harness exercises emitted code, real WebGPU and real workers.

Explicit packed CPU calls bind the function directly and marshal its arguments
without building a pipeline or profiling key. Pipeline reference detection
checks reference properties before inspecting object keys, so typed arrays are
never enumerated to distinguish constants from step references.

List results also expose an explicit destination ABI. `callInto(name, args,
destination, {backend:'javascript'|'cpu'})` returns an element count and writes
only that prefix. `prepareCpuCall(name, args, capacityWords)` owns a dedicated
Wasm instance, captures arguments once and returns packed borrowed output views
from `run()`. `writeArgument` updates reserved packed inputs; `close` releases
the call. Capacity, declared types, eight-byte packed alignment, nonaliasing and
borrowed view lifetimes are trusted caller obligations. Scratch allocations are
rewound to a saved mark on every prepared run. Ordinary calls preserve their
existing result behavior. Terminal CPU results decode into owned host values
before the arena resets; their intermediate packed clone is omitted. Pipeline
outputs decode on the final iteration, while only later stage uses and feedback
retain owned packed intermediates across resets. CPU destination decoding writes
directly into the caller's output prefix without an intermediate result Array.
U64 native bridge decoding produces BigInt in one pass; JSON-compatible public
U64 values still use safe Numbers or decimal strings. Generic packing fills its
final typed word allocation directly. These are fixed transport/lifetime rules,
not source-specific rewrites or mathematical proof admission. See the distribution's
`docs/web-engine-storage-and-contexts.md` for the full ownership and trust
boundary; the physical lowering is tested, not certified machine code.

Packed GPU descriptors may include a literal stage graph. The device package
expands supported map/indexed/zip/filter/scan/concat and ordered reductions into
resident stages, retaining dynamic lengths in list headers. No reduction
reassociation or automatic stage fusion is introduced. Graph feedback and
unsupported shapes retain CPU fallback. Actual dispatches, transfers and live
buffers are reported separately from capability metadata.

Generate shared host assets with `cargo run --example host-test-assets`, then run
`node --test tests/backend-selection.mjs`. Native/browser policy and packed ABI
conformance is also tested in the pinned `ink-abi` repository.

`emit_c_program` and `emit_rust_program` assemble complete literal programs with
primitive values, transaction publication, portable snapshot codecs and a shared
native/Wasm handle/buffer ABI. C emission uses shared Rust primitives; generated
programs contain no Ink parser or AST evaluator. `toolchain::complete_program`
builds native executables or Wasm modules. `module` extracts GPU-eligible pure
closures from mixed programs and retains persistent device engines.
The C header is `hosts/state-abi.h`; browser Wasm support is `hosts/state-wasm.mjs`.

`hosts/durable-state.mjs` provides serialized durable calls and bounded retry
receipts over generated JavaScript or the complete Rust Wasm ABI. Storage
adapters are Node's atomic synced file replacement (`durable-file.mjs`, one
writer process) and IndexedDB (transactional compare-and-replace). Ordered
outbox delivery is at least once; receivers deduplicate event coordinates.
An uncertain write requires reopening before further execution.
`durable::{CompiledState, Host, Store, FileStore}` supplies native embedding;
complete Rust projects and mixed native wgpu bundles share this host/CLI and accept
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

Packed compute bundles expose `await prepareGpuPipeline(plan)` for a fixed pure
pipeline without feedback. It snapshots/uploads declared inputs once and retains
GPU buffers and bindings. `run()` executes the active emitted legacy shader or
required graph and returns fresh full owned readback values plus actual counters;
`setup` records upload/live bytes separately, and subsequent `upload_bytes` is
zero. Dynamic graph output lengths remain device headers, never capacities.
`close()` releases storage; engine close/device loss invalidates sessions. Runs
are serialized by the host. This is resident input with owned readback, not a
GPU-only timing boundary.

`await open(plan, {backend})` opens a resident session with feedback: `step(n)`
runs iterations without readback, `read(request)` returns whole outputs, a
length, a slice or a device reduction, `write(input, value)` replaces an input
(or the carried feedback value), and `buffer(stepId)` exposes the step's
`GPUBuffer` for renderers on a device supplied as `loadInk(url, {device})`.
Ineligible plans and GPU failures at open run the same session on the compiled
CPU. Native `Engine` has the same operations (`open`, `step`, `read`, `write`,
`view`, `finish`, `close`, `with_device`). See docs/gpu-compute.md in the
distribution.

Source-bound representation codecs also transport supported List values through
Nil/Cons datatypes, preserving nested element types and enforcing depth/length
limits. Native definition execution retains its separate resource limits.
See the Ink distribution docs/point-query-delta.md for tested domains and bounds.

## License

Project code and documentation are [MIT licensed](LICENSE). Third-party dependencies,
vendored projects and research data retain their own licenses and notices.

Ordinary Rust packages include schema-bound portable value entrypoints and an
optional raw binary C/Wasm export. `StateInstance.pureBinary(index,bytes)` calls
that extension without a JSON envelope. Sorted supported signature names define
indices; schema identity and bounded codec checks establish value compatibility.
Actions retain their checked transaction interfaces. Wasm input/output currently
copy across instance memory, and compiled calls materialize target values. No
network protocol or profitability rule is introduced by this transport support.

### Profiled resident sessions

For a complete engine lifetime, the packed browser bundle exports
`await executeSession(base, plan, {iterations:32, costProfile, targetKey})`.
It copies inputs and profile before yielding, creates one engine, opens one
session, executes the requested iterations, reads all declared outputs once,
then closes the session and engine before returning owned values. Cleanup also
runs when opening, execution or reading fails. `backend` accepts `auto`, `cpu`,
`gpu` or `gpu-required`; automatic execution retains CPU without comparable
evidence. No runtime timing probes are performed.

For repeated calls, `const prepared = await prepareCostProfile(profile, targetKey)`
creates an opaque immutable profile token accepted by `costProfile`. Preparation
captures and validates the profile once, retaining a private encoding; later
engines do not copy that model graph or rebuild its decision-cache encoding.
A cache miss still sends the profile through the shared ABI decision validator.
Original profile mutations
cannot change the token. The same shared ABI decision, target checks and bounded
exact-input decision cache still apply. Tokens are local handles, not serialised
profiles or proof authority. Keep the original JSON for persistence. Preparation
owns no device or mutable execution state. Include its one-time cost in the whole
workload and report the number of calls over which that cost is amortised; it is
outside each subsequent `executeSession` lifetime.

This operation uses `ink-resident-engine-v1` scopes. Ordinary `loadInk`/`open`
sessions retain their separate `ink-resident-session-v2` scopes, so their profiles
cannot be substituted for one another. Both bind the same compiled host and
actual preparation context. The result includes the exact cost context and
post-disposal buffer counters. A caller-supplied GPU device remains externally
owned; its earlier creation is outside this call's lifetime.

The planner's `engine_profiles.fit_records` imports complete-engine observations
from at least three distinct training processes and uses the existing bounded
cost fitter and database feature-product basis. It rejects session scopes,
evaluation records, fallback-device measurements, changed provenance and missing
case coverage. The distribution's `engine-profile-browser.mjs` acquires complete
calls; `fit-engine-profile.py` retains their immutable identities. Browser startup,
common module import and input generation are outside acquisition. Shared caches
can be warm. This models the measured engine operation, not a cold browser or
total process memory, and requires independent held-out profitability evaluation.

`await engine.describeSession(plan, {iterations:32, readbacks:1})` returns the
scope, boundary and workload features for an expected complete session:
open/capture/upload, 32 iterations, completion, one full owned output read and
close. `engine.open(plan, {workload:{iterations:32, readbacks:1}})` uses that
context with the shared saved-profile decision contract. The forecast is a cost
hint, not a correctness assumption or restriction on later session operations.
If actual usage includes writes or partial reads, this model is not comparable;
omit the forecast or use an explicit backend until that lifecycle is measured.

The scope binds compiled host/shader identity, topology, input slots, output
identities, simultaneous feedback and full-readback count. It is distinct from
single-call and prepared-input profiles. `plan.iterations` is not a forecast of
future `session.step` calls. An unknown horizon or absent/out-of-domain profile
retains the compiled CPU baseline; automatic sessions previously preferred GPU
without consulting cost evidence. Explicit `gpu`/`gpu-required` remain available,
and applications that require `session.buffer()` must explicitly require GPU.
There are no runtime timing probes. A device failure retains the existing CPU
fallback, except for `gpu-required`.

`bench/gpu/resident-profile-browser.mjs` in the distribution checks independent
complete outputs on an actual browser adapter. Training and held-out runs use
different sizes and iteration horizons. Timings cover the complete lifecycle;
step-only timings are retained separately. Profiles bind CPU initialization,
device readiness and the preparation of every required shader. Warm evidence
cannot select a cold backend: missing comparable startup evidence retains CPU.
The harness records warm sessions and separate `cold-train`/`cold-test` sessions
on fresh engines; cold timings include backend and shader startup in `open`.
The common `loadInk` import is outside both measurements. Mixed CPU-warm/GPU-cold
and CPU-cold/GPU-warm cases have their own contexts and measurements. Resident
session counters expose live and peak allocated GPU buffer bytes, including
readback staging; CPU memory is reported as unknown (`null`). These counters
exclude host copies and driver allocations. No memory-profitability claim
follows from these timing models.
Backend selectors retain at most 128 decisions for identical eligible backends,
compiled scopes, observation boundaries and workload features. Options are copied
when the selector is created, request inputs are captured before asynchronous
loading, and returned decisions never share mutable objects with the cache.
`clear()` discards these decisions. This memoizes the shared ABI decision; it does
not measure execution, cache program results or allow out-of-domain evidence.


## Cooperative action scheduling

`hosts/scheduled-actions.mjs` exports `ScheduledActions({host, plan, onResult,
idleMs})` beside emitted durable hosts. `plan(host)` returns null (idle) or an
explicit `{id,call,args,delayMs}`; the host invokes one compiled action and calls
`onResult`. Queue contents, objective, model versions and bounded work/publication
rules remain source code. This helper does not infer a learner or retry uncertain
storage writes. Use it with `DurableState` to retain accepted action progress.

`run({signal})` returns the service lifetime promise. `wake()` interrupts idle
waiting; `pause()` waits for the current plan/action/callback, and `stop()` waits
for shutdown. From a plan/result callback, use synchronous `setPaused(true)` or
`requestStop()`; awaiting your own turn's quiescence would deadlock. `resume()`
permits new work. A pending plan is invalidated by pause/stop; source version checks
must independently reject stale candidates. Every turn yields to the event loop,
even if a callback repeatedly wakes it. One synchronous compiled action can still
block serving until it returns: enforce work limits in source. This is cooperative
single-host execution, not parallel training or a real-time deadline guarantee.

The service does not close its shared host. On an error it stops and rejects its
lifetime promise; close/reopen an uncertain durable host before explicitly retrying.
Observation identity/progress should live in source state, independently of the
host's bounded retry-receipt retention. Run
`node --test tests/scheduled-actions.mjs tests/durable-state.mjs` for lifecycle and
failure checks. The distribution also tests a real compiled queued-learning
application with exact reference checkpoints and offline CLI bootstrap.


## Explicit deployment initialization

Complete Rust projects now expose `initial_state()`. The default returns the
original source state; `emit_initial_state(project, Some(checkpoint))` installs an
embedded deployment checkpoint after independent source/typed validation in the
distribution. Generated native runners use that factory. `Host::open_initialized`
calls an explicit fallible factory only when storage is empty; recovery always
uses the stored checkpoint/receipts. Failed factories publish nothing.

`StatefulModule.instantiate(wasm,{initialSnapshot})` captures and validates private
checkpoint bytes. `create()` restores that seed; `createSourceState()` explicitly
constructs original source state. Pure-call holders use source state because their
checked functions cannot observe deployed application data. Mixed `loadInk` hosts
read a bound initial-state descriptor, restore CPU state and support an awaited
initialized JavaScript factory. State transactions and learning remain on CPU.
`initial-state.mjs` bounds/streams local or HTTP assets and checks their size/hash;
compiled restore separately validates source/schema/invariants. This is integrity
and startup machinery, not authenticity, equality or approximation authority.

Generated-code compilation can use `INK_COMPILED_TARGET_DIR` outside a published
package. An OS file lock spans both Cargo execution and output copying, preventing
another same-named crate from replacing the shared result between those operations.
Default builds keep their per-project target directories. The distribution's
`build-model` captures dependency locks and hashes the package before publication;
use locked builds and external caches to retain its integrity inventory.
