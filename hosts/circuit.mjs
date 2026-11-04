// Evaluate ink-circuit-v1 circuits (`ink emit-circuit`): affine layers, ReLU
// and ReGLU gates over one-hot encodings of finite values. Batches evaluate as
// dense Float32Array blocks, one row per input, so many inputs share one pass.
//
// A learn hole's lookup layer keeps its table in its weights: `table(hole)`
// reads the rows back and `setTable(hole, rows)` writes them, so the same rows
// that `ink learn-rows` lists and `ink gate-edit` admits are the network's
// weights. Writing weights changes only this circuit; the compiler's
// exhaustive check covers circuits as emitted.
export const FORMAT = 'ink-circuit-v1';

export function loadCircuit(circuit) {
  if (circuit?.format !== FORMAT) throw TypeError(`expected an ${FORMAT} circuit`);
  const nodes = circuit.nodes;
  const widths = [];
  for (const n of nodes) {
    widths.push(n.op === 'input' || n.op === 'affine' ? n.width : n.op === 'const' ? n.values.length
      : n.op === 'relu' ? widths[n.input] : widths[n.relu]);
  }
  function run(inputs) {
    const batch = inputs.length, values = [];
    nodes.forEach((n, k) => {
      const w = widths[k], out = new Float32Array(batch * w);
      if (n.op === 'input') {
        for (let b = 0; b < batch; b++) {
          const i = inputs[b][n.param];
          if (!(i >= 0 && i < w)) throw RangeError(`input ${n.param} out of range`);
          out[b * w + i] = 1;
        }
      } else if (n.op === 'const') {
        for (let b = 0; b < batch; b++) out.set(n.values, b * w);
      } else if (n.op === 'affine') {
        const parts = n.inputs.map(i => [values[i], widths[i]]);
        const offsets = []; let total = 0;
        for (const [, pw] of parts) { offsets.push(total); total += pw; }
        // Map each concatenated input column to its source block once.
        const source = new Int32Array(total), column = new Int32Array(total);
        parts.forEach(([, pw], p) => { for (let j = 0; j < pw; j++) { source[offsets[p] + j] = p; column[offsets[p] + j] = j; } });
        for (let b = 0; b < batch; b++) {
          for (let o = 0; o < w; o++) out[b * w + o] = n.bias[o] ?? 0;
          for (const [o, i, weight] of n.terms) {
            const [block, pw] = parts[source[i]];
            out[b * w + o] = Math.fround(out[b * w + o] + Math.fround(weight * block[b * pw + column[i]]));
          }
        }
      } else if (n.op === 'relu') {
        const x = values[n.input];
        for (let j = 0; j < x.length; j++) out[j] = x[j] > 0 ? x[j] : 0;
      } else if (n.op === 'gate') {
        const a = values[n.relu], x = values[n.linear];
        for (let j = 0; j < a.length; j++) out[j] = Math.fround((a[j] > 0 ? a[j] : 0) * x[j]);
      } else throw TypeError(`unknown node ${n.op}`);
      values.push(out);
    });
    return values[circuit.output];
  }
  const index = (p, value) => {
    const k = circuit.params[p].values.indexOf(value);
    if (k < 0) throw RangeError(`${value} is not a value of ${circuit.params[p].name}`);
    return k;
  };
  const decode = (out, b) => {
    const w = circuit.result.length;
    let best = 0;
    for (let j = 1; j < w; j++) if (out[b * w + j] > out[b * w + best]) best = j;
    return circuit.result[best];
  };
  const layer = name => {
    const h = circuit.holes.find(h => h.hole === name);
    if (!h) throw Error(`unknown hole ${name}`);
    return [h, nodes[h.node]];
  };
  const keyOf = (h, i) => h.domains.map(d => { const v = d[i % d.length]; i = Math.floor(i / d.length); return v; });
  return {
    circuit,
    /** Raw output vector for one input given as parameter value indices. */
    evaluate: indices => run([indices]),
    /** Result value names for many inputs, each a list of parameter value names. */
    batch: rows => { const out = run(rows.map(r => r.map((v, p) => index(p, v)))); return rows.map((_, b) => decode(out, b)); },
    call: (...values) => decode(run([values.map((v, p) => index(p, v))]), 0),
    table(name) {
      const [h, n] = layer(name), rows = [];
      for (let e = 0; e < h.entries; e++) {
        const t = n.terms.find(([, i, w]) => i === e && w === 1);
        rows.push({hole: name, key: keyOf(h, e), value: t ? h.values[t[0]] : null});
      }
      return rows;
    },
    setTable(name, rows) {
      const [h, n] = layer(name);
      const byEntry = new Map(n.terms.map(t => [t[1], t]));
      for (const r of rows) {
        let e = 0, stride = 1;
        r.key.forEach((v, j) => { const k = h.domains[j].indexOf(v); if (k < 0) throw RangeError(`${v} is not in the domain`); e += k * stride; stride *= h.domains[j].length; });
        const v = h.values.indexOf(r.value);
        if (v < 0) throw RangeError(`${r.value} is not a value of ${name}`);
        byEntry.set(e, [v, e, 1]);
      }
      n.terms = [...byEntry.values()].sort((a, b) => a[1] - b[1]);
    },
  };
}
