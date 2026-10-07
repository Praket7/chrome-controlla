import { isDeepStrictEqual } from 'node:util';

const forbiddenKeys = new Set(['__proto__', 'prototype', 'constructor']);

function pointer(path) {
  if (typeof path !== 'string' || !path.startsWith('/') || path === '/') throw new Error(`invalid JSON pointer: ${path}`);
  const parts = path.slice(1).split('/').map((part) => part.replaceAll('~1', '/').replaceAll('~0', '~'));
  if (parts.some((part) => !part || forbiddenKeys.has(part))) throw new Error(`unsafe JSON pointer: ${path}`);
  return parts;
}

function at(root, path) {
  let node = root;
  for (const part of pointer(path)) {
    if (node === null || typeof node !== 'object' || !Object.hasOwn(node, part)) throw new Error(`missing state path: ${path}`);
    node = node[part];
  }
  return node;
}

function parentAt(root, path) {
  const parts = pointer(path);
  const key = parts.pop();
  let parent = root;
  for (const part of parts) {
    if (parent === null || typeof parent !== 'object' || !Object.hasOwn(parent, part)) throw new Error(`missing state path: ${path}`);
    parent = parent[part];
  }
  if (parent === null || typeof parent !== 'object' || !Object.hasOwn(parent, key)) throw new Error(`missing state path: ${path}`);
  return [parent, key];
}

export function validateFixtureContract(fixture) {
  const errors = [];
  if (fixture?.schema_version !== 1 || fixture?.kind !== 'offline-fixture-only') errors.push('fixture must declare schema_version 1 and offline-fixture-only');
  if (typeof fixture?.id !== 'string' || !fixture.id.trim()) errors.push('fixture id is required');
  if (!fixture?.initial_state || typeof fixture.initial_state !== 'object' || Array.isArray(fixture.initial_state)) errors.push('initial_state must be a JSON object');
  if (fixture?.reset?.strategy !== 'clone-initial-state') errors.push('reset strategy must be clone-initial-state');
  if (!Array.isArray(fixture?.actions) || !Array.isArray(fixture?.predicates) || fixture.predicates.length === 0) errors.push('actions and non-empty predicates are required');
  if (fixture?.before_actions !== undefined && !Array.isArray(fixture.before_actions)) errors.push('before_actions must be an array');
  const ids = new Set();
  for (const [i, action] of (fixture?.actions ?? []).entries()) {
    if (!['set', 'set-if', 'increment', 'append'].includes(action?.op)) errors.push(`action ${i + 1}: unsupported operation`);
    try { pointer(action?.path); } catch (error) { errors.push(`action ${i + 1}: ${error.message}`); }
    if (action?.op === 'increment' && (!Number.isFinite(action.by) || action.by === 0)) errors.push(`action ${i + 1}: increment requires a nonzero finite by`);
    if (['set', 'set-if'].includes(action?.op) && !Object.hasOwn(action, 'value')) errors.push(`action ${i + 1}: set requires value`);
    if (action?.op === 'append' && !Object.hasOwn(action, 'value')) errors.push(`action ${i + 1}: append requires value`);
    if (action?.op === 'set-if' && (!Array.isArray(action.when) || action.when.length === 0)) errors.push(`action ${i + 1}: set-if requires conditions`);
    for (const condition of action?.when ?? []) {
      try { pointer(condition?.path); } catch (error) { errors.push(`action ${i + 1}: ${error.message}`); }
      if (!Object.hasOwn(condition ?? {}, 'equals')) errors.push(`action ${i + 1}: condition requires equals`);
    }
  }
  for (const [i, action] of (fixture?.before_actions ?? []).entries()) {
    if (!['set', 'increment', 'append'].includes(action?.op)) errors.push(`before_action ${i + 1}: unsupported operation`);
    try { pointer(action?.path); } catch (error) { errors.push(`before_action ${i + 1}: ${error.message}`); }
    if (action?.op === 'increment' && (!Number.isFinite(action.by) || action.by === 0)) errors.push(`before_action ${i + 1}: increment requires a nonzero finite by`);
    if (['set', 'append'].includes(action?.op) && !Object.hasOwn(action, 'value')) errors.push(`before_action ${i + 1}: ${action.op} requires value`);
  }
  for (const [i, predicate] of (fixture?.predicates ?? []).entries()) {
    if (typeof predicate?.id !== 'string' || !predicate.id.trim() || ids.has(predicate.id)) errors.push(`predicate ${i + 1}: id must be present and unique`);
    ids.add(predicate?.id);
    if (!['equals', 'unchanged'].includes(predicate?.op)) errors.push(`predicate ${i + 1}: unsupported operation`);
    try { pointer(predicate?.path); } catch (error) { errors.push(`predicate ${i + 1}: ${error.message}`); }
    if (predicate?.op === 'equals' && !Object.hasOwn(predicate, 'expected')) errors.push(`predicate ${i + 1}: equals requires expected`);
  }
  return errors;
}

export function resetFixture(fixture) {
  const errors = validateFixtureContract(fixture);
  if (errors.length) throw new Error(`invalid offline fixture: ${errors.join('; ')}`);
  return structuredClone(fixture.initial_state);
}

function apply(state, action) {
  if (action.op === 'set-if') {
    const applies = action.when.every((condition) => isDeepStrictEqual(at(state, condition.path), condition.equals));
    if (!applies) return false;
    action = { op: 'set', path: action.path, value: action.value };
  }
  const [parent, key] = parentAt(state, action.path);
  if (action.op === 'set') parent[key] = structuredClone(action.value);
  else if (action.op === 'increment') {
    if (typeof parent[key] !== 'number' || !Number.isFinite(parent[key])) throw new Error(`increment target is not numeric: ${action.path}`);
    parent[key] += action.by;
  } else if (action.op === 'append') {
    if (!Array.isArray(parent[key])) throw new Error(`append target is not an array: ${action.path}`);
    parent[key].push(structuredClone(action.value));
  } else throw new Error(`unsupported operation: ${action.op}`);
  return true;
}

export function evaluatePredicates(initialState, readback, predicates) {
  return predicates.map((predicate) => {
    const observed = structuredClone(at(readback, predicate.path));
    const passed = predicate.op === 'equals'
      ? isDeepStrictEqual(observed, predicate.expected)
      : isDeepStrictEqual(observed, at(initialState, predicate.path));
    return { id: predicate.id, path: predicate.path, op: predicate.op, observed, passed };
  });
}

export function runOfflineFixture(fixture) {
  const state = resetFixture(fixture);
  const initialState = structuredClone(state);
  for (const action of fixture.before_actions ?? []) apply(state, action);
  const actionOutcomes = fixture.actions.map((action) => ({ applied: apply(state, action) }));
  const readback = structuredClone(state);
  const predicateResults = evaluatePredicates(initialState, readback, fixture.predicates);
  return {
    kind: 'offline-fixture-validation-only',
    fixture_id: fixture.id,
    final_state: readback,
    action_outcomes: actionOutcomes,
    predicate_results: predicateResults,
    all_predicates_passed: predicateResults.every((result) => result.passed),
  };
}
