import test from 'node:test';
import assert from 'node:assert/strict';
import {runTask, replayTask, TaskFailed} from '../hosts/tasks.mjs';
import {modelEffect, effectPrompt, replyValue} from '../hosts/model-effect.mjs';

// A hand-written manifest with fake generated queries: `a` and `b` depend only
// on the parameter, `c` uses both, `write` is a barrier.
const manifest = {
  format: 'ink-tasks-v1',
  effects: [
    {name: 'get', params: [['x', 'U32']], result: 'U32', signature: 'get(x: u32) -> u32'},
    {name: 'pick', params: [['x', 'U32']], result: {Named: 'Choice'}, signature: 'pick(x: u32) -> Choice', declarations: 'enum Choice { Left, Right, }'},
  ],
  tasks: [{
    name: 't', params: [['n', 'U32']], result: {Result: ['U32', 'String']},
    steps: [
      {bind: 'a', kind: 'effect', callee: 'get', args: ['arg'], uses: ['n'], accept: 'accept'},
      {bind: 'b', kind: 'effect', callee: 'get', args: ['arg'], uses: ['n'], accept: 'small', fallback: 'zero'},
      {bind: 'c', kind: 'value', callee: 'sum', uses: ['a', 'b'], after: [0, 1]},
      {bind: 'w', kind: 'change', callee: 'write', args: ['argc'], uses: ['c'], propagate: true, after: [0, 1, 2]},
      {bind: 'd', kind: 'effect', callee: 'get', args: ['arg'], uses: ['n'], accept: 'accept'},
    ],
    ret: 'ok', ret_uses: ['w', 'd'],
  }],
};
function program() {
  const log = [];
  const call = (name, args) => {
    log.push(name);
    switch (name) {
      case 'arg': case 'argc': return args[0];
      case 'accept': if (typeof args[1] !== 'number') throw TypeError('u32'); return true;
      case 'small': if (typeof args[1] !== 'number') throw TypeError('u32'); return args[1] < 100;
      case 'zero': return 0;
      case 'sum': return args[0] + args[1];
      case 'write': return args[0] === 13 ? {Err: 'unlucky'} : {Ok: args[0] * 2};
      case 'ok': return {Ok: args[0] + args[1]};
    }
    throw Error(`unknown ${name}`);
  };
  return {call, log};
}
const delay = ms => new Promise(r => setTimeout(r, ms));

test('independent effects overlap, barriers wait and the result is sequential', async () => {
  let live = 0, peak = 0;
  const get = async x => { live++; peak = Math.max(peak, live); await delay(10); live--; return x + 1; };
  const {call, log} = program();
  const {result, trace} = await runTask({manifest, call, task: 't', args: [4], effects: {get}});
  assert.deepEqual(result, {Ok: 2 * (5 + 5) + 5});
  assert.equal(peak, 3); // a, b and the speculative d all ran together
  assert(log.indexOf('write') > log.lastIndexOf('sum'));
  assert.deepEqual(trace.effects.map(r => [r.step, r.outcome]), [[0, 'accepted'], [1, 'accepted'], [4, 'accepted']]);
  const once = await runTask({manifest, call, task: 't', args: [4], effects: {get}, concurrency: 1});
  assert.deepEqual(once.result, result);
  assert.deepEqual(await replayTask({manifest, call, trace}).then(t => t.result), result);
});

test('rejected replies fall back; the earliest failure in source order wins', async () => {
  const {call} = program();
  // b's reply is too large for its contract and falls back to 0.
  const big = await runTask({manifest, call, task: 't', args: [1], effects: {get: async x => x * 1000}});
  assert.deepEqual(big.trace.effects.map(r => r.outcome), ['accepted', 'fallback', 'accepted']);
  assert.deepEqual(big.result, {Ok: 2 * 1000 + 1000});
  // An unguarded reply of the wrong type fails the task.
  const typed = await runTask({manifest, call, task: 't', args: [1], effects: {get: async () => 'x'}}).catch(e => e);
  assert(typed instanceof TaskFailed && /rejects/.test(typed.message));
  // `?` on the change's Err ends the task; d ran speculatively and is discarded.
  const replies = [6, 7, 1];
  const unlucky = await runTask({manifest, call, task: 't', args: [0], effects: {get: async () => replies.shift()}});
  assert.deepEqual(unlucky.result, {Err: 'unlucky'});
  assert.deepEqual(unlucky.trace.effects.map(r => [r.step, r.outcome]), [[0, 'accepted'], [1, 'accepted'], [4, 'discarded']]);
  assert.deepEqual(await replayTask({manifest, call, trace: unlucky.trace}).then(t => t.result), {Err: 'unlucky'});
  // a fails slowly while d fails fast: the task reports a's failure.
  let calls = 0;
  const slowFirst = async () => { const k = calls++; await delay(k === 0 ? 20 : 0); throw Error(`fail ${k}`); };
  const failed = await runTask({manifest, call, task: 't', args: [1], effects: {get: slowFirst}}).catch(e => e);
  assert(failed instanceof TaskFailed);
  assert.match(failed.message, /fail 0/);
  assert.equal(failed.trace.effects.find(r => r.step === 4).outcome, 'discarded');
});

test('speculative effects after a failure are aborted', async () => {
  const {call} = program();
  let aborted = null;
  // a fails at once; d (step 4) is slow and should see its signal abort.
  const get = async (x, context) => {
    if (context.step === 0) throw Error('down');
    if (context.step === 4) return new Promise((resolve, reject) => { context.signal.addEventListener('abort', () => { aborted = context.signal.reason?.message; reject(context.signal.reason); }); setTimeout(() => resolve(1), 500); });
    return x;
  };
  const failed = await runTask({manifest, call, task: 't', args: [1], effects: {get}}).catch(e => e);
  assert(failed instanceof TaskFailed);
  assert.equal(aborted, 'an earlier step ended the task');
  assert.equal(failed.trace.effects.find(r => r.step === 4).outcome, 'discarded');
});

test('replay detects changed replies and needs no handlers', async () => {
  const {call} = program();
  const {trace} = await runTask({manifest, call, task: 't', args: [2], effects: {get: async x => x}});
  const forged = structuredClone(trace);
  forged.effects[0].reply = 50;
  await assert.rejects(replayTask({manifest, call, trace: forged}), /differs/);
  await assert.rejects(runTask({manifest, call, task: 't', args: [2], replay: {...trace, effects: trace.effects.slice(1)}}), /no reply/);
});

test('a model effect prompts with the declared signature and parses JSON', async () => {
  const prompt = effectPrompt(manifest.effects[1], [3]);
  assert.match(prompt, /pick\(x: u32\) -> Choice/);
  assert.match(prompt, /enum Choice \{ Left, Right, \}/);
  assert.match(prompt, /x = 3/);
  assert.deepEqual(replyValue('```json\n{"Some": 12345678901234567890}\n```'), {Some: 12345678901234567890n});
  assert.throws(() => replyValue('Left'), SyntaxError);
  const seen = [];
  const handler = modelEffect(async (p, o) => { seen.push(o.effect); return '"Choice.Left"'; });
  assert.equal(await handler(3, {effect: manifest.effects[1]}), 'Choice.Left');
  assert.deepEqual(seen, ['pick']);
});
