// A language model as an Ink effect handler.
//
// `modelEffect(complete)` turns any text-completion function into a handler
// for an effect declared in Ink, for example
//
//   effect classify(text: String, labels: List<Label>) -> Label;
//   let label = classify(t, labels) ensures label != Label.Other else rule(t);
//
// The prompt states the effect's signature, the declarations of the types it
// reaches (from the task manifest) and the JSON encoding Ink accepts. The
// model's reply is parsed as JSON and returned unchanged: the task host then
// passes it through the effect's generated accept query, so a reply that is
// not a value of the declared type, or that fails the step's `ensures`
// predicate, is replaced by the authored `else` value. Nothing the model says
// is trusted beyond the declared type and contract. Recorded task traces keep
// the model's replies, so a run replays without calling the model again.
import {stringifyExactJSON} from './state-wasm.mjs';

const ENCODING = [
  'Encode the reply as JSON in Ink\'s portable form:',
  '- records are objects with every declared field; lists are arrays; String is a JSON string;',
  '- enum values are strings "Enum.Variant", for example "Label.Bug";',
  '- Option is {"Some": value} or {"None": null}; Result is {"Ok": value} or {"Err": error};',
  '- Unit is null; Bool is true or false; integer and float types are JSON numbers.',
].join('\n');

/** The prompt sent for one effect request. */
export function effectPrompt(effect, args, instructions = '') {
  const named = effect.params.map(([name], i) => `${name} = ${stringifyExactJSON(args[i])}`).join('\n');
  return [
    instructions,
    `You are the implementation of this function:\n  ${effect.signature}`,
    effect.declarations ? `It uses these declarations:\n${effect.declarations}` : '',
    `Arguments:\n${named || '(none)'}`,
    ENCODING,
    'Reply with only the JSON value of the result type, without explanation or code fences.',
  ].filter(Boolean).join('\n\n');
}

/** Parse a model reply as one JSON value, ignoring surrounding code fences. */
export function replyValue(text) {
  if (typeof text !== 'string') throw TypeError('model reply must be text');
  const trimmed = text.trim().replace(/^```(?:json)?\s*/i, '').replace(/```$/, '').trim();
  // Integers beyond the exact Number range keep their digits as BigInt.
  return JSON.parse(trimmed, (_, value, context) =>
    typeof value === 'number' && !Number.isSafeInteger(value) && /^-?[0-9]+$/.test(context?.source ?? '') ? BigInt(context.source) : value);
}

/**
 * @param {(prompt: string, options: {effect: string, signal?: AbortSignal}) => Promise<string>} complete
 * @param {{instructions?: string}} [options]
 */
export function modelEffect(complete, {instructions = ''} = {}) {
  if (typeof complete !== 'function') throw TypeError('modelEffect needs a completion function');
  return async (...values) => {
    const context = values.pop();
    if (!context?.effect) throw TypeError('modelEffect handlers run inside runTask');
    const prompt = effectPrompt(context.effect, values, instructions);
    return replyValue(await complete(prompt, {effect: context.effect.name, signal: context.signal}));
  };
}
