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
