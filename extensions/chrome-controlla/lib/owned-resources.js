// Owned browser resources: durable state, not receipt archaeology.
//
// Open receipts serve several jobs at once (idempotency, ownership, tab lookup,
// window lookup, document identity, recovery, reuse). Scanning them in creation
// order and hoping the newest eight cover the live tab breaks as soon as more
// than eight opens happen. This module builds the hot indexes once per call and
// answers reuse questions from them.
//
// Everything here is pure: it reads receipts, never Chrome. Liveness, leases
// and presentation slots are supplied by the caller.

import { logicalResourceKey } from "./resource-keys.js";

export const ReuseKind = Object.freeze({
  EXACT_DOCUMENT: "exact_document",
  SAME_RESOURCE_NAVIGATION: "same_resource_navigation",
  RECYCLED_OWNED_TAB: "recycled_owned_tab",
  NEW_TAB_EXISTING_WINDOW: "new_tab_existing_window",
  NEW_TAB_NEW_WINDOW: "new_tab_new_window",
});

// How long a tab is expected to survive. Probes create ephemeral tabs that may
// be recycled once idle; a client that opens a tab through a shared session is
// holding state it may still reference, so those default to session.
export const TabDurability = Object.freeze({
  EPHEMERAL: "ephemeral",
  SESSION: "session",
  DURABLE: "durable",
});

const COMPLETED = new Set(["complete", "closed"]);

function positiveInt(value) {
  const number = Number(value);
  return Number.isSafeInteger(number) && number > 0 ? number : null;
}

function normalizedDurability(value) {
  return Object.values(TabDurability).includes(value) ? value : null;
}

// One record per live tab, aggregated from every receipt that points at it.
// Alias receipts written when a new idempotency key resolved to an existing tab
// are therefore folded in rather than shadowing the original creation record.
export function indexOwnedResources(receipts, { now = Date.now() } = {}) {
  const byTab = new Map();
  const byKey = new Map();
  const byUrl = new Map();
  const byResource = new Map();
  const byWindow = new Map();

  for (const [key, receipt] of Object.entries(receipts || {})) {
    if (!receipt || typeof receipt !== "object") continue;
    const tabId = positiveInt(receipt.tab_id);
    byKey.set(key, { key, ...receipt });
    if (tabId === null) continue;
    let record = byTab.get(tabId);
    const created = Number.isFinite(receipt.created_ms) ? receipt.created_ms : 0;
    const used = Number.isFinite(receipt.last_used_ms) ? receipt.last_used_ms : created;
    const verified = Number.isFinite(receipt.last_verified_ms) ? receipt.last_verified_ms : created;
    if (!record) {
      record = {
        tab_id: tabId,
        window_id: positiveInt(receipt.window_id),
        url: typeof receipt.url === "string" ? receipt.url : null,
        document_id: typeof receipt.document_id === "string" ? receipt.document_id : null,
        resource_key: logicalResourceKey(receipt.url)?.key ?? null,
        durability: normalizedDurability(receipt.durability)
          ?? (receipt.create_background_window === true ? TabDurability.EPHEMERAL : TabDurability.SESSION),
        status: receipt.status ?? "unknown",
        create_background_window: receipt.create_background_window === true,
        created_ms: created,
        last_used_ms: used,
        last_verified_ms: verified,
        keys: [key],
        alias_of: receipt.alias_of ?? null,
      };
      byTab.set(tabId, record);
    } else {
      record.keys.push(key);
      // A closed receipt wins over a stale complete one: the tab is known gone.
      if (receipt.status === "closed") record.status = "closed";
      else if (COMPLETED.has(receipt.status) && record.status !== "closed") record.status = receipt.status;
      if (created >= record.created_ms) {
        record.created_ms = created;
        record.window_id = positiveInt(receipt.window_id) ?? record.window_id;
        record.url = typeof receipt.url === "string" ? receipt.url : record.url;
        record.document_id = typeof receipt.document_id === "string" ? receipt.document_id : record.document_id;
        record.resource_key = logicalResourceKey(record.url)?.key ?? record.resource_key;
        if (normalizedDurability(receipt.durability)) record.durability = normalizedDurability(receipt.durability);
      }
      record.last_used_ms = Math.max(record.last_used_ms, used);
      record.last_verified_ms = Math.max(record.last_verified_ms, verified);
    }
  }

  for (const record of byTab.values()) {
    if (record.url) {
      if (!byUrl.has(record.url)) byUrl.set(record.url, []);
      byUrl.get(record.url).push(record);
    }
    if (record.resource_key) {
      if (!byResource.has(record.resource_key)) byResource.set(record.resource_key, []);
      byResource.get(record.resource_key).push(record);
    }
    if (record.window_id !== null) {
      if (!byWindow.has(record.window_id)) byWindow.set(record.window_id, []);
      byWindow.get(record.window_id).push(record);
    }
  }

  return { byTab, byKey, byUrl, byResource, byWindow, records: [...byTab.values()], now };
}

// A record is only a *candidate*. Liveness and leases are checked by the caller
// against Chrome, because a receipt is a memory of intent, not a proof of the
// tab's current state.
export function usableRecord(record, { isLeased = () => false, isTabUnavailable = () => false } = {}) {
  if (!record || record.status !== "complete") return false;
  if (!record.url || !record.document_id) return false;
  if (isLeased(record.tab_id)) return false;
  if (isTabUnavailable(record.tab_id)) return false;
  return true;
}

// Reuse priority: exact document, same logical resource, recyclable idle owned
// tab. The caller then verifies generation and reserves presentation.
export function selectReuse(index, {
  url,
  isLeased = () => false,
  isTabUnavailable = () => false,
  allowResourceNavigation = true,
  allowRecycle = true,
  minIdleMs = 60_000,
  now = index.now,
} = {}) {
  if (typeof url !== "string" || !url) return null;
  const usable = record => usableRecord(record, { isLeased, isTabUnavailable });

  const exact = (index.byUrl.get(url) ?? [])
    .filter(usable)
    .sort((left, right) => right.last_used_ms - left.last_used_ms)[0];
  if (exact) return { kind: ReuseKind.EXACT_DOCUMENT, record: exact, navigate: false };

  if (allowResourceNavigation) {
    const resource = logicalResourceKey(url);
    if (resource) {
      const sameResource = (index.byResource.get(resource.key) ?? [])
        .filter(usable)
        .sort((left, right) => right.last_used_ms - left.last_used_ms)[0];
      if (sameResource && sameResource.url !== url) {
        return { kind: ReuseKind.SAME_RESOURCE_NAVIGATION, record: sameResource, navigate: true };
      }
    }
  }

  if (allowRecycle) {
    const idle = index.records
      .filter(record => record.durability === TabDurability.EPHEMERAL)
      .filter(record => record.status === "complete")
      .filter(record => record.url !== url)
      .filter(record => !isLeased(record.tab_id))
      .filter(record => !isTabUnavailable(record.tab_id))
      .filter(record => now - record.last_used_ms >= minIdleMs)
      .sort((left, right) => left.last_used_ms - right.last_used_ms)[0];
    if (idle) return { kind: ReuseKind.RECYCLED_OWNED_TAB, record: idle, navigate: true };
  }

  return null;
}

// A bounded pool needs an eviction policy that is safe to run unattended.
// Only ephemeral, completed, unleased, unprotected tabs that have been idle for
// the whole grace window are candidates. A missing candidate refuses the open
// instead of silently exceeding the bound or closing user-visible work.
export const DEFAULT_OWNED_TAB_POOL_LIMIT = 12;
export const MIN_EVICTION_IDLE_MS = 120_000;

export function planPoolEviction(index, {
  limit = DEFAULT_OWNED_TAB_POOL_LIMIT,
  isLeased = () => false,
  isTabUnavailable = () => false,
  isTabProtected = () => false,
  keepTabIds = [],
  now = index.now,
  minIdleMs = MIN_EVICTION_IDLE_MS,
} = {}) {
  const keep = new Set(keepTabIds.map(Number).filter(Number.isSafeInteger));
  const live = index.records.filter(record => record.window_id !== null || record.status === "complete");
  const over = live.length - limit;
  if (over < 0) return { within_limit: true, live_tabs: live.length, limit, evict: [], blocked: false };
  const candidates = live
    .filter(record => record.durability === TabDurability.EPHEMERAL)
    .filter(record => record.status === "complete")
    .filter(record => !keep.has(record.tab_id))
    .filter(record => !isLeased(record.tab_id))
    .filter(record => !isTabUnavailable(record.tab_id))
    .filter(record => !isTabProtected(record.tab_id))
    .filter(record => now - record.last_used_ms >= minIdleMs)
    .sort((left, right) => left.last_used_ms - right.last_used_ms);
  const needed = over + 1;
  const evict = candidates.slice(0, needed);
  if (evict.length < needed) {
    return {
      within_limit: false,
      live_tabs: live.length,
      limit,
      evict,
      blocked: true,
      reason: "no_safe_eviction_candidate",
      needed,
      available: candidates.length,
    };
  }
  return { within_limit: false, live_tabs: live.length, limit, evict, blocked: false, needed };
}

// Windows that currently hold a completed Controlla-owned tab. Eligibility is
// re-proved live by the caller (unfocused, not minimized, no foreign tabs need
// checking here; ownership is what this answers).
export function ownedWindows(index) {
  const windows = new Map();
  for (const record of index.records) {
    if (record.window_id === null) continue;
    const entry = windows.get(record.window_id) ?? { window_id: record.window_id, tabs: 0, last_used_ms: 0, created_background_window: false };
    entry.tabs += 1;
    entry.last_used_ms = Math.max(entry.last_used_ms, record.last_used_ms);
    entry.created_background_window ||= record.create_background_window;
    windows.set(record.window_id, entry);
  }
  return [...windows.values()].sort((left, right) => right.last_used_ms - left.last_used_ms);
}

// Current-call telemetry. Historical receipt booleans describe the tab's whole
// life, which is not what a caller needs to know about this call.
export function openTelemetry({ created = false, windowCreated = false, reuseKind = null } = {}) {
  const reused = !created;
  return {
    tab_created_this_call: created,
    tab_reused_this_call: reused,
    window_created_this_call: windowCreated,
    window_reused_this_call: !windowCreated,
    reuse_kind: reuseKind ?? (created ? ReuseKind.NEW_TAB_EXISTING_WINDOW : ReuseKind.EXACT_DOCUMENT),
  };
}

// Receipts used for reuse must be written before the result is returned, so a
// client or worker restart retries the same idempotency key against the same
// tab instead of creating a second one.
export function aliasReceipt(existing, { key, url, active = false, createdMs = Date.now() } = {}) {
  return {
    status: "complete",
    tab_id: String(existing.tab_id),
    window_id: existing.window_id,
    url: existing.url ?? url,
    requested_url: url,
    document_id: existing.document_id,
    active: active === true,
    create_background_window: existing.create_background_window === true,
    reused_background_window: true,
    reused_open_tab: true,
    alias_of: existing.keys?.[0] ?? null,
    durability: existing.durability ?? TabDurability.EPHEMERAL,
    created_ms: createdMs,
    last_used_ms: createdMs,
    last_verified_ms: createdMs,
  };
}
