// Learned parameters as rows, and gated edits to them (see ink-lang
// docs/learn-rows.md). Uses only a module's generated `__learn_get`,
// `__learn_set` and `__objective_NAME` entry points and the hole layout from
// `ink emit-holes` (format ink-holes-v1).
//
// A choice entry becomes a row keyed by its argument values,
//   {hole: 'recover', key: ['State.Quoted', 'Class.Quote'], value: 'Step.End'},
// and an mlp weight a row keyed by index, {hole: 'adjust', index: 3, value: 0.25}.
// An edit is a list of rows from anyone, including a language model. A gate
// admits it only if the exact objective on the gate's data improves (or does
// not worsen, with allowEqual), then applies it through the learn change.
export const ROWS_FORMAT = 'ink-learn-rows-v1';
export const EDIT_FORMAT = 'ink-learn-edit-v1';

const ordinal = value => (typeof value === 'bigint' ? Number(value) : value);

function layout(holes) {
  if (holes?.format !== 'ink-holes-v1') throw TypeError('expected an ink-holes-v1 layout');
  return holes.holes;
}
function keyOf(h, i) {
  return h.domains.map(d => { const v = d[i % d.length]; i = Math.floor(i / d.length); return v; });
}
function indexOf(h, key) {
  if (!Array.isArray(key) || key.length !== h.domains.length) throw Error(`${h.name}: key needs ${h.domains.length} values`);
  let index = 0, stride = 1;
  key.forEach((v, j) => {
    const k = h.domains[j].indexOf(v);
    if (k < 0) throw Error(`${h.name}: ${v} is not one of ${h.domains[j].join(', ')}`);
    index += k * stride; stride *= h.domains[j].length;
  });
  return index;
}

/** Rows for a parameter record returned by `__learn_get`. */
export function learnRows(holes, params) {
  const rows = [];
  for (const h of layout(holes)) {
    const xs = params[h.field];
    if (!Array.isArray(xs)) throw Error(`missing parameters for ${h.name}`);
    xs.forEach((x, i) => rows.push(h.family === 'choice'
      ? {hole: h.name, key: keyOf(h, i), value: h.values[ordinal(x)]}
      : {hole: h.name, index: i, value: x}));
  }
  return rows;
}

/** Apply rows to a parameter record. Returns {next, changed}. */
export function applyEdit(holes, params, rows) {
  if (!Array.isArray(rows) || rows.length > 16384) throw Error('an edit is a list of at most 16384 rows');
  const hs = layout(holes), next = structuredClone(params), seen = new Set();
  let changed = 0;
  for (const r of rows) {
    const h = hs.find(h => h.name === r.hole);
    if (!h) throw Error(`unknown hole ${r.hole}`);
    let i, v;
    if (h.family === 'choice') {
      if (r.index !== undefined) throw Error(`${h.name}: choice rows are addressed by key`);
      i = indexOf(h, r.key);
      v = h.values.indexOf(r.value);
      if (v < 0) throw Error(`${h.name}: ${r.value} is not one of ${h.values.join(', ')}`);
    } else {
      if (r.key !== undefined) throw Error(`${h.name}: mlp rows are addressed by index`);
      i = r.index; v = r.value;
      if (!Number.isInteger(i) || i < 0 || i >= h.parameters || typeof v !== 'number' || !Number.isFinite(v)) throw Error(`${h.name}: index ${i} or value ${v} is out of range`);
    }
    if (seen.has(`${h.name}:${i}`)) throw Error(`${h.name}: an edit addresses parameter ${i} twice`);
    seen.add(`${h.name}:${i}`);
    if (ordinal(next[h.field][i]) !== v) changed++;
    next[h.field][i] = v;
  }
  return {next, changed};
}

const isOk = v => v !== null && typeof v === 'object' && Object.hasOwn(v, 'Ok');

/**
 * Admit an edit only if the exact objective improves on `data`.
 * `call(name, args)` reaches the live state. Pass `scratch` (a state restored
 * from the same parameters) to evaluate there, so a rejected edit never
 * touches the live state; without it, a rejected edit is rolled back with a
 * second learn commit.
 */
export async function gatedEdit({call, scratch = call, holes, edit, objective, data, allowEqual = false}) {
  if (edit?.format !== EDIT_FORMAT) throw TypeError(`expected an ${EDIT_FORMAT} edit`);
  if (!layout(holes).length || !holes.objectives.includes(objective)) throw Error(`unknown objective ${objective}`);
  const score = async () => Number(await scratch(`__objective_${objective}`, data));
  const current = await scratch('__learn_get', []);
  const {next, changed} = applyEdit(holes, current, edit.rows);
  const before = await score();
  if (!isOk(await scratch('__learn_set', [next]))) return {admitted: false, before, changed, rejection: 'the parameter record failed its shape check'};
  const after = await score();
  const admitted = changed > 0 && (after < before || (allowEqual && after <= before));
  if (!admitted) {
    await scratch('__learn_set', [current]);
    return {admitted, before, after, changed, rejection: changed ? 'the objective did not improve on the gate data' : 'the edit changes no parameter'};
  }
  if (scratch !== call && !isOk(await call('__learn_set', [next]))) throw Error('admitted parameters were rejected by the live state');
  return {admitted, before, after, changed};
}

/**
 * Ask a language model for an edit. `complete(prompt) -> Promise<string>`.
 * The reply is only a proposal; pass it to gatedEdit.
 */
export async function proposeEdit({complete, holes, rows, failures = [], instructions = ''}) {
  const shown = rows.map(r => r.key ? `${r.hole}(${r.key.join(', ')}) = ${r.value}` : `${r.hole}[${r.index}] = ${r.value}`);
  const choices = layout(holes).filter(h => h.family === 'choice').map(h => `${h.name}: values ${h.values.join(', ')}`);
  const prompt = [
    instructions,
    'These are learned parameters of a program, one per line:',
    shown.join('\n'),
    choices.length ? `Allowed values:\n${choices.join('\n')}` : '',
    failures.length ? `Cases the program currently gets wrong:\n${failures.map(f => JSON.stringify(f)).join('\n')}` : '',
    'Propose a change. Reply with only JSON: {"reason": "...", "rows": [{"hole": "...", "key": [...], "value": "..."}]} for table entries, or {"hole": "...", "index": N, "value": number} for weights.',
  ].filter(Boolean).join('\n\n');
  const text = (await complete(prompt)).trim().replace(/^```(?:json)?\s*/i, '').replace(/```$/, '').trim();
  const reply = JSON.parse(text);
  if (!reply || !Array.isArray(reply.rows)) throw Error('the model did not propose rows');
  return {format: EDIT_FORMAT, author: 'model', reason: typeof reply.reason === 'string' ? reply.reason : undefined, rows: reply.rows};
}
