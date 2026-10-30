// Tab identity and safe reuse helpers. These do not grant authority; callers
// still verify the paired document and lease before any mutation.

export function normalizeTabUrl(value) {
  if (typeof value !== "string" || !value) return null;
  try {
    const url = new URL(value);
    if (!(url.protocol === "http:" || url.protocol === "https:")) return null;
    if (!url.hostname || url.username || url.password) return null;
    return url.href;
  } catch {
    return null;
  }
}

export function canonicalUrl(value) {
  const normalized = normalizeTabUrl(value);
  if (!normalized) return null;
  const url = new URL(normalized);
  url.hash = "";
  if (url.pathname === "/") url.pathname = "/";
  return url.href;
}

export function findReusableTab(tabs, { url, documentId, requestedTabId } = {}) {
  const candidates = Array.isArray(tabs) ? tabs : [];
  if (requestedTabId !== undefined) {
    return candidates.find(tab => String(tab?.id) === String(requestedTabId)) ?? null;
  }
  const target = canonicalUrl(url);
  if (!target) return null;
  return candidates.find(tab => {
    if (documentId && tab?.documentId && tab.documentId !== documentId) return false;
    return canonicalUrl(tab?.url) === target;
  }) ?? null;
}

// A background automation window is durable state, not a per-open resource.
// Minting a window for every open strands one window per tab and buries the
// user's real windows, so an open reuses the live unfocused window Controlla
// already owns. A focused or missing window is never reused: the caller asked
// for an unfocused presentation and user windows stay untouched.
export async function reusableOwnedBackgroundWindow(windowsApi, receipts) {
  const candidates = Object.values(receipts || {})
    .filter(receipt => receipt?.status === "complete" && receipt.create_background_window === true
      && Number.isSafeInteger(receipt.window_id) && typeof receipt.document_id === "string")
    .sort((left, right) => (right.created_ms || 0) - (left.created_ms || 0))
    .slice(0, 8);
  for (const receipt of candidates) {
    const window = await windowsApi.get(receipt.window_id, { populate: true }).catch(() => undefined);
    if (!window || window.focused !== false || (window.type !== undefined && window.type !== "normal")) continue;
    if (Array.isArray(window.tabs) && window.tabs.some(tab => tab.incognito)) continue;
    return window.id;
  }
  return undefined;
}

export function tabIdentity(tab) {
  return {
    tab_id: tab?.id ?? null,
    window_id: tab?.windowId ?? null,
    group_id: Number.isSafeInteger(tab?.groupId) && tab.groupId >= 0 ? tab.groupId : null,
    url: tab?.url ?? null,
    status: tab?.status ?? null,
    frozen: tab?.frozen === true,
  };
}
