// Task-conditioned semantic observation helpers. Pruning happens before any
// bytes reach model context; deltas are document-scoped, keyed by stable
// identity, and never merge two distinct controls that share a role and name.

const ACTION_ROLES = new Set([
  "button", "link", "textbox", "searchbox", "combobox", "checkbox", "radio",
  "switch", "tab", "menuitem", "option", "slider", "spinbutton",
]);

function tokenize(value) {
  return String(value ?? "")
    .toLowerCase()
    .split(/[^a-z0-9]+/u)
    .filter(token => token.length > 1);
}

function scoreItem(item, request) {
  const role = String(item?.role ?? "").toLowerCase();
  const name = String(item?.name ?? item?.text ?? "");
  const tokens = new Set(tokenize(`${name} ${item?.href ?? ""} ${item?.label ?? ""}`));
  const queryTokens = tokenize(request?.query ?? request?.task_hint ?? "");
  let score = 0;
  if (ACTION_ROLES.has(role)) score += 8;
  if (item?.focused === true) score += 10;
  if (item?.disabled !== true) score += 1;
  if (request?.required_roles?.length && request.required_roles.map(String).map(value => value.toLowerCase()).includes(role)) score += 12;
  for (const token of queryTokens) if (tokens.has(token)) score += 6;
  if (item?.changed === true) score += 5;
  if (item?.in_dialog === true || item?.dialog === true) score += 4;
  if (item?.visible === false) score -= 8;
  if (item?.value !== undefined && item?.value !== null) score += 1;
  return score;
}

function isRequired(item, required) {
  return required.has(String(item?.role ?? "").toLowerCase());
}

export function pruneSemanticItems(items, request = {}) {
  const list = Array.isArray(items) ? items : [];
  const required = new Set((request.required_roles ?? []).map(value => String(value).toLowerCase()));
  const scored = list.map((item, index) => ({ item, index, score: scoreItem(item, request) }));
  const budgetChars = Number.isSafeInteger(request.byte_budget) ? request.byte_budget : 12_000;
  const kept = [];
  let bytes = 0;
  for (const entry of scored.sort((a, b) => b.score - a.score || a.index - b.index)) {
    if (entry.score <= 0 && !isRequired(entry.item, required)) continue;
    const size = JSON.stringify(entry.item).length;
    if (kept.length > 0 && bytes + size > budgetChars) continue;
    kept.push(entry);
    bytes += size;
    if (kept.length >= (request.max_items ?? 40)) break;
  }
  const keptIndexes = new Set(kept.map(entry => entry.index));
  const requiredIndexes = scored.filter(entry => isRequired(entry.item, required)).map(entry => entry.index);
  const requiredRetained = requiredIndexes.filter(index => keptIndexes.has(index)).length;
  return {
    items: kept.map(entry => entry.item),
    pruned_count: list.length - kept.length,
    retained_count: kept.length,
    retained_bytes: bytes,
    // Real retention signal: did pruning drop a role the caller declared
    // required? A constant one was not a measurement.
    required_total: requiredIndexes.length,
    required_retained: requiredRetained,
    required_retention_rate: requiredIndexes.length === 0 ? 1 : requiredRetained / requiredIndexes.length,
    dropped_required: requiredIndexes.length - requiredRetained,
  };
}

// Stable identity is the same hierarchy the target registry uses: a retained
// node identity first, then structural fingerprints, and only then role/name
// with an explicit occurrence ordinal. Two "Save" buttons therefore stay two
// distinct entries instead of colliding on one key.
function identityKeys(items) {
  const seen = new Map();
  return items.map(item => {
    const frame = String(item?.frame_id ?? item?.source_frame ?? "");
    const retained = item?.stable_id ?? item?.retained_reference ?? item?.backend_node_id ?? item?.node_id;
    if (retained !== undefined && retained !== null && retained !== "") {
      return `node:${frame}\u0000${retained}`;
    }
    const fingerprint = item?.dom_path_fingerprint ?? item?.path;
    if (fingerprint) return `path:${frame}\u0000${fingerprint}`;
    if (item?.test_id) return `test:${frame}\u0000${item.test_id}`;
    const base = `rn:${frame}\u0000${item?.role ?? ""}\u0000${item?.name ?? item?.text ?? ""}\u0000${item?.href ?? ""}`;
    const occurrence = seen.get(base) ?? 0;
    seen.set(base, occurrence + 1);
    return `${base}\u0000#${occurrence}`;
  });
}

export function semanticDelta(previous, current, { documentId = null, previousDocumentId = null } = {}) {
  const prior = Array.isArray(previous?.items) ? previous.items : [];
  const next = Array.isArray(current?.items) ? current.items : [];
  if (documentId && previousDocumentId && documentId !== previousDocumentId) {
    return { document_replaced: true, added: next, removed: [], changed: [], unchanged: 0 };
  }
  const priorKeys = identityKeys(prior);
  const nextKeys = identityKeys(next);
  const previousByKey = new Map(priorKeys.map((key, index) => [key, prior[index]]));
  const nextKeySet = new Set(nextKeys);
  const added = [];
  const changed = [];
  const duplicate_keys = nextKeys.length - nextKeySet.size;
  for (let index = 0; index < next.length; index += 1) {
    const before = previousByKey.get(nextKeys[index]);
    if (!before) {
      added.push(next[index]);
      continue;
    }
    if (before.value !== next[index].value || before.checked !== next[index].checked
      || before.selected !== next[index].selected || before.disabled !== next[index].disabled
      || before.focused !== next[index].focused || before.expanded !== next[index].expanded) {
      changed.push(next[index]);
    }
  }
  const removed = prior.filter((_, index) => !nextKeySet.has(priorKeys[index]));
  return {
    document_replaced: false,
    added,
    removed,
    changed,
    unchanged: next.length - added.length - changed.length,
    // Observability for the identity scheme itself: a nonzero count means two
    // live items shared a structural key and were disambiguated by ordinal.
    duplicate_keys,
  };
}
