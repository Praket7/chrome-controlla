// Tab identity and safe reuse helpers. These do not grant authority; callers
// still verify the paired document and lease before any mutation.
//
// Discovery is deliberately read-only. Finding a reusable tab must never change
// the active tab, because another task may already own that window's
// presentation; activation is a separate, reserved step.

import { logicalResourceKey } from "./resource-keys.js";
import { indexOwnedResources, selectReuse, ReuseKind, TabDurability } from "./owned-resources.js";

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

// A window is eligible to present Controlla work when it is a normal,
// non-incognito, unfocused window that Controlla owns. Minimized is excluded
// for VISIBLE work: Chrome does not paint a minimized window, so a canvas
// editor there cannot confirm what was typed. `state`/`type` are optional in
// mocks but always present in Chrome.
export function windowPresentationEligibility(window) {
  if (!window || !Number.isSafeInteger(window.id)) return { eligible: false, reason: "window_missing" };
  if (window.focused === true) return { eligible: false, reason: "window_focused" };
  if (window.state === "minimized") return { eligible: false, reason: "window_minimized" };
  if (window.type !== undefined && window.type !== "normal") return { eligible: false, reason: "window_not_normal" };
  if (window.incognito === true) return { eligible: false, reason: "window_incognito" };
  const tabs = Array.isArray(window.tabs) ? window.tabs : [];
  if (tabs.some(tab => tab.incognito === true)) return { eligible: false, reason: "window_incognito" };
  return { eligible: true, reason: null };
}

export function windowPresentationProof(window) {
  return {
    window_id: Number.isSafeInteger(window?.id) ? window.id : null,
    window_focused: window?.focused === true,
    window_state: ["normal", "minimized", "maximized", "fullscreen"].includes(window?.state) ? window.state : "unknown",
    window_minimized: window?.state === "minimized",
    window_type: typeof window?.type === "string" ? window.type : "unknown",
    tab_count: Array.isArray(window?.tabs) ? window.tabs.length : null,
  };
}

// A repeated background open must not mint another tab for a URL the owned
// window already holds. This is discovery only: it proves the tab still exists,
// still carries the expected document, and lives in an eligible window, then
// returns a candidate. It never touches `active`.
//
// Priority comes from the owned-resource index, not from the newest eight
// receipts: exact document, same logical resource, recyclable idle tab.
export async function findReusableOwnedBackgroundCandidate(chromeApi, receipts, url, identityOf, {
  isLeased = () => false,
  allowResourceNavigation = true,
  allowRecycle = false,
  minIdleMs = 60_000,
} = {}) {
  const index = indexOwnedResources(receipts);
  const unavailable = new Set();
  for (;;) {
    const selection = selectReuse(index, {
      url,
      isLeased,
      isTabUnavailable: tabId => unavailable.has(tabId),
      allowResourceNavigation,
      allowRecycle,
      minIdleMs,
    });
    if (!selection) return null;
    const { record, kind, navigate } = selection;
    const tab = await chromeApi.tabs.get(record.tab_id).catch(() => undefined);
    if (!tab || (kind !== ReuseKind.RECYCLED_OWNED_TAB && tab.url !== record.url)) {
      unavailable.add(record.tab_id);
      continue;
    }
    const window = await chromeApi.windows.get(tab.windowId).catch(() => undefined);
    const eligibility = windowPresentationEligibility(window);
    if (!eligibility.eligible) {
      unavailable.add(record.tab_id);
      continue;
    }
    // Recycling may navigate the tab, so only its stable document identity is
    // required; exact and same-resource reuse must still match the receipt.
    const identity = await identityOf(tab.id).catch(() => undefined);
    if (!identity?.document_id) {
      unavailable.add(record.tab_id);
      continue;
    }
    if (!navigate && (identity.url !== record.url || identity.document_id !== record.document_id)) {
      unavailable.add(record.tab_id);
      continue;
    }
    return {
      tab,
      window,
      record,
      kind,
      navigate,
      match: kind === ReuseKind.EXACT_DOCUMENT ? "exact_document" : kind === ReuseKind.SAME_RESOURCE_NAVIGATION ? "same_resource" : "recycled",
      url: record.url,
      document_id: identity.document_id,
      current_url: identity.url,
      resource_key: logicalResourceKey(url)?.key ?? null,
      window_id: tab.windowId,
      presentation: windowPresentationProof(window),
    };
  }
}

// Selecting the tab is the only step that changes presentation, and it is only
// reached after the caller has reserved the window's presentation slot. The
// tab is selected inside its own unfocused window; the browser is never focused.
export async function activateReservedPresentation(chromeApi, candidate) {
  if (!candidate?.tab) return { ok: false, reason: "candidate_missing" };
  const tabId = candidate.tab.id;
  const before = await chromeApi.tabs.get(tabId).catch(() => undefined);
  if (!before) return { ok: false, reason: "tab_closed" };
  const windowBefore = await chromeApi.windows.get(before.windowId).catch(() => undefined);
  const eligibility = windowPresentationEligibility(windowBefore);
  if (!eligibility.eligible) return { ok: false, reason: eligibility.reason };
  if (!before.active) await chromeApi.tabs.update(tabId, { active: true });
  const after = await chromeApi.tabs.get(tabId).catch(() => undefined);
  const windowAfter = after && await chromeApi.windows.get(after.windowId).catch(() => undefined);
  if (!after?.active) return { ok: false, reason: "tab_not_active_after_activation" };
  if (windowAfter?.focused !== false) return { ok: false, reason: "window_focused_during_activation" };
  if (windowAfter?.state === "minimized") return { ok: false, reason: "window_minimized_during_activation" };
  return {
    ok: true,
    tab: after,
    window: windowAfter,
    activated: before.active !== true,
    proof: { ...windowPresentationProof(windowAfter), tab_active: true, tab_id: tabId },
  };
}

// The live, unfocused automation window an open may place a tab in. Windows are
// pooled so sequential work reuses one window while simultaneous visible work
// gets a window of its own; a window whose presentation slot is taken is not
// offered to a second task.
export async function findOwnedBackgroundWindow(windowsApi, receipts, { isWindowBusy = () => false, index } = {}) {
  const resources = index ?? indexOwnedResources(receipts);
  const candidates = resources.records
    .filter(record => record.create_background_window === true && record.window_id !== null)
    .sort((left, right) => right.created_ms - left.created_ms);
  const seen = new Set();
  for (const record of candidates) {
    if (seen.has(record.window_id) || isWindowBusy(record.window_id)) continue;
    seen.add(record.window_id);
    const window = await windowsApi.get(record.window_id, { populate: true }).catch(() => undefined);
    if (!windowPresentationEligibility(window).eligible) continue;
    return { windowId: window.id, proof: windowPresentationProof(window) };
  }
  return undefined;
}

// Bounded receipt retention. The extension keeps one record per open so a
// retry can find the tab it created instead of opening a second one. Eviction
// never removes the only receipt that still names an open tab, and it removes
// now-unreachable records before live ones.
export function pruneOpenReceipts(receipts, maxAgeMs, maxCount, now = Date.now()) {
  for (const [key, receipt] of Object.entries(receipts)) {
    if (["complete", "closed"].includes(receipt?.status) && Number.isSafeInteger(receipt.created_ms) && now - receipt.created_ms > maxAgeMs) delete receipts[key];
  }
  const completed = Object.entries(receipts)
    .filter(([, receipt]) => ["complete", "closed"].includes(receipt?.status))
    .sort((a, b) => (a[1].created_ms || 0) - (b[1].created_ms || 0));
  while (Object.keys(receipts).length > maxCount && completed.length) {
    const [key] = completed.shift();
    delete receipts[key];
  }
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

