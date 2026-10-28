// Task-conditioned semantic observation helpers. Pruning happens before any
// bytes reach model context; deltas are document-scoped and never recycle a
// reference from a replaced document.

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

function scoreItem(item, request, index) {
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
  return { score, index };
}

export function pruneSemanticItems(items, request = {}) {
  const list = Array.isArray(items) ? items : [];
  const required = new Set((request.required_roles ?? []).map(value => String(value).toLowerCase()));
  const scored = list.map((item, index) => ({ item, ...scoreItem(item, request, index) }));
  const budgetChars = Number.isSafeInteger(request.byte_budget) ? request.byte_budget : 12_000;
  const kept = [];
  let bytes = 0;
  for (const entry of scored.sort((a, b) => b.score - a.score || a.index - b.index)) {
    const role = String(entry.item?.role ?? "").toLowerCase();
    if (entry.score <= 0 && !required.has(role)) continue;
    const size = JSON.stringify(entry.item).length;
    if (kept.length > 0 && bytes + size > budgetChars) continue;
    kept.push(entry.item);
    bytes += size;
    if (kept.length >= (request.max_items ?? 40)) break;
  }
  return {
    items: kept,
    pruned_count: list.length - kept.length,
    retained_count: kept.length,
    retained_bytes: bytes,
    recall_floor: kept.length === 0 && list.length > 0 ? 0 : 1,
  };
}

export function semanticDelta(previous, current, { documentId = null, previousDocumentId = null } = {}) {
  const prior = Array.isArray(previous?.items) ? previous.items : [];
  const next = Array.isArray(current?.items) ? current.items : [];
  if (documentId && previousDocumentId && documentId !== previousDocumentId) {
    return { document_replaced: true, added: next, removed: [], changed: [], unchanged: 0 };
  }
  const key = item => `${item?.role ?? ""}\u0000${item?.name ?? item?.text ?? ""}\u0000${item?.frame_id ?? ""}`;
  const previousByKey = new Map(prior.map(item => [key(item), item]));
  const nextKeys = new Set(next.map(key));
  const added = next.filter(item => !previousByKey.has(key(item)));
  const changed = next.filter(item => {
    const before = previousByKey.get(key(item));
    if (!before) return false;
    return before.value !== item.value || before.checked !== item.checked
      || before.selected !== item.selected || before.disabled !== item.disabled
      || before.focused !== item.focused || before.expanded !== item.expanded;
  });
  const removed = prior.filter(item => !nextKeys.has(key(item)));
  return { document_replaced: false, added, removed, changed, unchanged: next.length - added.length - changed.length };
}
