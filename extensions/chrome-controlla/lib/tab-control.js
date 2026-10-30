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

// A repeated background open must not mint another tab for a URL the owned
// window already holds. Only a tab in an unfocused window is eligible, and it
// is reselected inside that window (never focusing the browser) so Chrome
// paints it and trusted typing has a visible target.
export async function reusableOwnedBackgroundTab(chromeApi, receipts, url, identityOf) {
  const candidates = Object.values(receipts || {})
    .filter(receipt => receipt?.status === "complete" && receipt.create_background_window === true
      && receipt.url === url && Number.isSafeInteger(Number(receipt.tab_id)) && typeof receipt.document_id === "string")
    .sort((left, right) => (right.created_ms || 0) - (left.created_ms || 0))
    .slice(0, 8);
  for (const receipt of candidates) {
    const tabId = Number(receipt.tab_id);
    const tab = await chromeApi.tabs.get(tabId).catch(() => undefined);
    if (!tab || tab.url !== url) continue;
    const window = await chromeApi.windows.get(tab.windowId).catch(() => undefined);
    if (!window || window.focused !== false || window.state === "minimized") continue;
    const identity = await identityOf(tabId).catch(() => undefined);
    if (!identity || identity.url !== receipt.url || identity.document_id !== receipt.document_id) continue;
    if (!tab.active) await chromeApi.tabs.update(tabId, { active: true });
    const active = await chromeApi.tabs.get(tabId).catch(() => undefined);
    const after = active && await chromeApi.windows.get(active.windowId).catch(() => undefined);
    if (!active?.active || after?.focused !== false) continue;
    return { tab: active, receipt };
  }
  return null;
}

// Bounded receipt retention. The extension keeps one record per open so a
// retry can find the tab it created instead of opening a second one.
export function pruneOpenReceipts(receipts, maxAgeMs, maxCount, now = Date.now()) {
  for (const [key, receipt] of Object.entries(receipts)) {
    if (["complete", "closed"].includes(receipt?.status) && Number.isSafeInteger(receipt.created_ms) && now - receipt.created_ms > maxAgeMs) delete receipts[key];
  }
  const completed = Object.entries(receipts).filter(([, receipt]) => ["complete", "closed"].includes(receipt?.status))
    .sort((a, b) => (a[1].created_ms || 0) - (b[1].created_ms || 0));
  while (Object.keys(receipts).length > maxCount && completed.length) delete receipts[completed.shift()[0]];
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
