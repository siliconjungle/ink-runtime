import test from 'node:test';
import assert from 'node:assert/strict';
import {learnRows, applyEdit, gatedEdit, proposeEdit, EDIT_FORMAT} from '../hosts/learn-rows.mjs';

const holes = {format: 'ink-holes-v1', objectives: ['loss'], holes: [
  {name: 'pick', family: 'choice', field: 'pick', parameters: 4, choices: 2, domains: [['A.X', 'A.Y'], ['false', 'true']], values: ['B.L', 'B.R'], admission: 'error-path', declaration: 'f'},
  {name: 'w', family: 'mlp', field: 'w', parameters: 2, choices: 0, shape: [1, 1], admission: 'guarded', declaration: 'g'},
]};
// A fake state: loss counts table entries that are not B.R plus |w0 - 1|.
function fake() {
  let params = {pick: [0, 0, 0, 0], w: [0, 0]}, commits = 0;
  const call = (name, args) => {
    if (name === '__learn_get') return structuredClone(params);
    if (name === '__learn_set') { if (args[0].pick.some(v => v > 1)) return {Err: 'shape'}; params = structuredClone(args[0]); commits++; return {Ok: null}; }
    if (name === '__objective_loss') return params.pick.filter(v => v !== 1).length + Math.abs(params.w[0] - 1);
    throw Error(name);
  };
  return {call, commits: () => commits};
}

test('rows are keyed by argument values and indices', () => {
  const rows = learnRows(holes, {pick: [0, 1, 0, 1], w: [0.5, 0]});
  assert.deepEqual(rows[1], {hole: 'pick', key: ['A.Y', 'false'], value: 'B.R'});
  assert.deepEqual(rows[2], {hole: 'pick', key: ['A.X', 'true'], value: 'B.L'});
  assert.deepEqual(rows[4], {hole: 'w', index: 0, value: 0.5});
  assert.deepEqual(applyEdit(holes, {pick: [0, 1, 0, 1], w: [0.5, 0]}, rows), {next: {pick: [0, 1, 0, 1], w: [0.5, 0]}, changed: 0});
  assert.throws(() => applyEdit(holes, {pick: [0, 0, 0, 0], w: [0, 0]}, [{hole: 'pick', key: ['A.Z', 'true'], value: 'B.L'}]), /not one of/);
  assert.throws(() => applyEdit(holes, {pick: [0, 0, 0, 0], w: [0, 0]}, [{hole: 'w', index: 2, value: 1}]), /out of range/);
  assert.throws(() => applyEdit(holes, {pick: [0, 0, 0, 0], w: [0, 0]}, [{hole: 'w', index: 0, value: 1}, {hole: 'w', index: 0, value: 2}]), /twice/);
});

test('the gate admits improvements and rolls back everything else', async () => {
  const {call, commits} = fake();
  const edit = rows => ({format: EDIT_FORMAT, rows});
  const good = await gatedEdit({call, holes, objective: 'loss', data: [], edit: edit([{hole: 'pick', key: ['A.X', 'false'], value: 'B.R'}, {hole: 'w', index: 0, value: 1}])});
  assert.deepEqual([good.admitted, good.before, good.after, good.changed], [true, 5, 3, 2]);
  const bad = await gatedEdit({call, holes, objective: 'loss', data: [], edit: edit([{hole: 'w', index: 0, value: 3}])});
  assert.equal(bad.admitted, false);
  assert.deepEqual(call('__learn_get', []), {pick: [1, 0, 0, 0], w: [1, 0]});
  assert.equal(commits(), 3); // admit, then the rejected edit and its rollback
  const neutral = await gatedEdit({call, holes, objective: 'loss', data: [], allowEqual: true, edit: edit([{hole: 'w', index: 1, value: 7}])});
  assert.equal(neutral.admitted, true);
  await assert.rejects(gatedEdit({call, holes, objective: 'other', data: [], edit: edit([])}), /unknown objective/);
});

test('a scratch state keeps rejected edits away from the live state', async () => {
  const live = fake(), scratch = fake();
  const r = await gatedEdit({call: live.call, scratch: scratch.call, holes, objective: 'loss', data: [], edit: {format: EDIT_FORMAT, rows: [{hole: 'w', index: 0, value: -4}]}});
  assert.equal(r.admitted, false);
  assert.equal(live.commits(), 0);
  const ok = await gatedEdit({call: live.call, scratch: scratch.call, holes, objective: 'loss', data: [], edit: {format: EDIT_FORMAT, rows: [{hole: 'w', index: 0, value: 1}]}});
  assert.equal(ok.admitted, true);
  assert.equal(live.commits(), 1);
});

test('a model proposal is parsed into an edit', async () => {
  let seen;
  const edit = await proposeEdit({holes, rows: learnRows(holes, {pick: [0, 0, 0, 0], w: [0, 0]}), failures: [{a: 'A.X'}],
    complete: async p => { seen = p; return '{"reason": "r", "rows": [{"hole": "pick", "key": ["A.X", "false"], "value": "B.R"}]}'; }});
  assert.match(seen, /pick\(A\.X, false\) = B\.L/);
  assert.match(seen, /pick: values B\.L, B\.R/);
  assert.deepEqual(edit, {format: EDIT_FORMAT, author: 'model', reason: 'r', rows: [{hole: 'pick', key: ['A.X', 'false'], value: 'B.R'}]});
  await assert.rejects(proposeEdit({holes, rows: [], complete: async () => 'no'}), SyntaxError);
});
