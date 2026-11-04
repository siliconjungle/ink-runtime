import test from 'node:test';
import assert from 'node:assert/strict';
import {loadCircuit} from '../hosts/circuit.mjs';

// and(x, y) over Bool one-hots: relu(x_true + y_true - 1), then [1 - t, t];
// plus a 2-entry lookup layer `h(x)` whose weights hold its table.
const circuit = {
  format: 'ink-circuit-v1', module: 'm', function: 'f',
  params: [{name: 'x', values: ['false', 'true']}, {name: 'y', values: ['false', 'true']}],
  result: ['false', 'true'],
  nodes: [
    {op: 'input', param: 0, width: 2},
    {op: 'input', param: 1, width: 2},
    {op: 'affine', inputs: [0, 1], width: 1, terms: [[0, 1, 1], [0, 3, 1]], bias: [-1]},
    {op: 'relu', input: 2},
    {op: 'affine', inputs: [3], width: 2, terms: [[0, 0, -1], [1, 0, 1]], bias: [1, 0]},
    {op: 'affine', inputs: [0], width: 2, terms: [[0, 0, 1], [0, 1, 1]], bias: [0, 0]},
    {op: 'gate', relu: 4, linear: 5},
  ],
  output: 4,
  holes: [{hole: 'h', node: 5, entries: 2, domains: [['false', 'true']], values: ['false', 'true']}],
};

test('batches evaluate exactly and match single calls', () => {
  const c = loadCircuit(structuredClone(circuit));
  const inputs = [['false', 'false'], ['true', 'false'], ['false', 'true'], ['true', 'true']];
  assert.deepEqual(c.batch(inputs), ['false', 'false', 'false', 'true']);
  assert.deepEqual(inputs.map(x => c.call(...x)), ['false', 'false', 'false', 'true']);
  assert.deepEqual([...c.evaluate([1, 1])], [0, 1]);
  assert.throws(() => c.call('maybe', 'true'), /not a value/);
});

test('hole weights read and write as rows', () => {
  const c = loadCircuit(structuredClone(circuit));
  assert.deepEqual(c.table('h'), [{hole: 'h', key: ['false'], value: 'false'}, {hole: 'h', key: ['true'], value: 'false'}]);
  c.setTable('h', [{key: ['true'], value: 'true'}]);
  assert.deepEqual(c.table('h').map(r => r.value), ['false', 'true']);
  assert.throws(() => c.setTable('h', [{key: ['true'], value: 'maybe'}]), /not a value/);
  assert.throws(() => loadCircuit({format: 'other'}), /expected/);
});
