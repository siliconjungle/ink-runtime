// Concurrent host for Ink task manifests (`ink emit-tasks`, format ink-tasks-v1).
//
// The compiler fixes each step's dependencies; this host only schedules them.
// Queries, changes and ordered effects wait for every earlier step. Other
// effects and pure values start as soon as the steps whose names they use have
// finished, so independent requests overlap. The result is the one sequential
// execution in source order would produce from the same effect replies:
// a failure ends the task at the earliest failing step in source order, and
// effects that were started speculatively after it are recorded as discarded.
//
// Every value crosses into Ink through generated readonly queries. A handler
// reply that does not decode to the effect's declared type, fails its
// `ensures` predicate, or throws is replaced by the authored `else` value; an
// unguarded failure ends the task with an error. Traces record each request
// and reply so `replayTask` (or `ink replay-task`) can re-run them without
// calling the outside world.
export const TRACE_FORMAT = 'ink-task-trace-v1';
export const MANIFEST_FORMAT = 'ink-tasks-v1';

export class TaskFailed extends Error {
  constructor(message, trace) { super(message); this.name = 'TaskFailed'; this.trace = trace; }
}

// Adapts a compiled state (`createState()`) or anything with invoke(call,args)
// returning {result}. DurableState.invoke takes an operation id first; pass
// `(name,args)=>durable.invoke(id(),name,args).then(r=>r.result)` instead.
export function stateCaller(state) {
  return (name, args) => state.invoke(name, args).result;
}

function task(manifest, name) {
  if (manifest?.format !== MANIFEST_FORMAT) throw TypeError(`expected a ${MANIFEST_FORMAT} manifest`);
  const t = manifest.tasks.find(t => t.name === name);
  if (!t) throw Error(`unknown task ${name}`);
  // Empty lists and false flags are omitted from manifests.
  return {...t, ret_uses: t.ret_uses ?? [], steps: t.steps.map(s => ({...s, args: s.args ?? [], uses: s.uses ?? [], after: s.after ?? [], propagate: s.propagate === true}))};
}

const isErr = v => v !== null && typeof v === 'object' && Object.hasOwn(v, 'Err');
const isOk = v => v !== null && typeof v === 'object' && Object.hasOwn(v, 'Ok');

/**
 * Run one task.
 * @param {object} o
 * @param {object} o.manifest  ink-tasks-v1 manifest
 * @param {Function} o.call    (name, args) => result value (sync or async)
 * @param {string} o.task      task name
 * @param {Array} o.args       task arguments
 * @param {object} [o.effects] handlers by effect name: (...args, context) => reply
 * @param {object} [o.replay]  a recorded trace; replies are taken from it by step
 * @param {number} [o.concurrency] maximum concurrently running steps (default 16)
 * @param {AbortSignal} [o.signal]
 * @returns {Promise<{result:any, trace:object}>}
 */
export async function runTask({manifest, call, task: name, args, effects = {}, replay, concurrency = 16, signal}) {
  const t = task(manifest, name);
  if (!Array.isArray(args) || args.length !== t.params.length) throw TypeError(`task ${name} takes ${t.params.length} arguments`);
  if (!(concurrency >= 1)) throw RangeError('concurrency must be at least 1');
  const recorded = new Map();
  if (replay) {
    if (replay.format !== TRACE_FORMAT || replay.task !== name) throw Error('replay trace does not match this task');
    for (const r of replay.effects) { if (recorded.has(r.step)) throw Error('trace records a step twice'); recorded.set(r.step, r); }
  }
  const declared = new Map(manifest.effects.map(e => [e.name, e]));
  const ctx = new Map(t.params.map(([n], i) => [n, args[i]]));
  const n = t.steps.length, status = new Array(n).fill('waiting'), records = [];
  let failedAt = Infinity, failure = null;
  const used = s => s.uses.map(u => ctx.get(u));
  const fail = (i, f) => { if (i < failedAt) { failedAt = i; failure = f; } };

  async function perform(i, s, values) {
    const effect = declared.get(s.callee);
    const record = {step: i, effect: s.callee, args: values, outcome: 'accepted'};
    let reply, failed = false;
    if (replay) {
      const r = recorded.get(i);
      if (!r || r.effect !== s.callee) throw Error(`trace has no reply for step ${i + 1}`);
      if (Object.hasOwn(r, 'error')) { failed = true; record.error = r.error; } else { reply = r.reply; record.reply = reply; }
    } else {
      const handler = effects[s.callee];
      try {
        if (typeof handler !== 'function') throw Error(`no handler for effect ${s.callee}`);
        reply = await handler(...values, {effect, step: i, task: name, signal});
        record.reply = reply;
      } catch (e) { failed = true; record.error = String(e?.message ?? e); }
    }
    let accepted = false;
    if (!failed) {
      try { accepted = (await call(s.accept, [...used(s), reply])) === true; } catch { accepted = false; }
    }
    records.push(record);
    if (accepted) return reply;
    if (s.fallback) { record.outcome = 'fallback'; return call(s.fallback, used(s)); }
    record.outcome = 'failed';
    throw new TaskFailed(failed ? `effect ${s.callee} failed: ${record.error}` : `effect ${s.callee} returned a value its declared type or contract rejects`);
  }

  async function step(i) {
    const s = t.steps[i];
    if (s.kind === 'value') return call(s.callee, used(s));
    const values = [];
    for (const q of s.args) values.push(await call(q, used(s)));
    return s.kind === 'effect' ? perform(i, s, values) : call(s.callee, values);
  }

  const running = new Set();
  for (;;) {
    if (signal?.aborted) fail(-1, {error: signal.reason ?? Error('aborted')});
    for (let i = 0; i < n && i < failedAt && running.size < concurrency; i++) {
      if (status[i] !== 'waiting' || !t.steps[i].after.every(j => status[j] === 'done')) continue;
      status[i] = 'running';
      const s = t.steps[i];
      const p = Promise.resolve().then(() => step(i)).then(value => {
        if (s.propagate) {
          if (isErr(value)) { status[i] = 'failed'; fail(i, {result: value}); return; }
          if (!isOk(value)) throw TypeError(`step ${i + 1}: expected a Result`);
          value = value.Ok;
        }
        if (s.bind !== undefined) ctx.set(s.bind, value);
        status[i] = 'done';
      }, error => { status[i] = 'failed'; fail(i, {error}); }).finally(() => running.delete(p));
      running.add(p);
    }
    if (running.size === 0) break;
    await Promise.race(running);
  }
  for (const r of records) if (r.step > failedAt) r.outcome = 'discarded';
  records.sort((a, b) => a.step - b.step);
  const trace = {format: TRACE_FORMAT, task: name, args, effects: records};
  if (failure?.error) {
    const error = failure.error;
    if (!(error instanceof TaskFailed)) throw error;
    trace.error = error.message;
    error.trace = trace;
    throw error;
  }
  trace.result = failure ? failure.result : await call(t.ret, t.ret_uses.map(u => ctx.get(u)));
  return {result: trace.result, trace};
}

/** Re-run a recorded trace without calling handlers; the result must match. */
export async function replayTask({manifest, call, trace}) {
  let replayed;
  try { replayed = (await runTask({manifest, call, task: trace.task, args: trace.args, replay: trace})).trace; }
  catch (e) { if (!(e instanceof TaskFailed)) throw e; replayed = e.trace; }
  const same = (a, b) => JSON.stringify(a, (_, v) => typeof v === 'bigint' ? `${v}n` : v) === JSON.stringify(b, (_, v) => typeof v === 'bigint' ? `${v}n` : v);
  const kept = trace.effects.filter(r => r.outcome !== 'discarded').map(r => [r.step, r.outcome]);
  if (!same(replayed.effects.filter(r => r.outcome !== 'discarded').map(r => [r.step, r.outcome]), kept)
    || !same(replayed.result, trace.result) || Boolean(replayed.error) !== Boolean(trace.error)) {
    throw Error('replay differs from the recorded trace');
  }
  return replayed;
}
