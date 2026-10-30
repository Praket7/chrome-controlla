// Window-level presentation leases.
//
// A Chrome window can only show one tab at a time. Tab ownership alone cannot
// stop two concurrent tasks from stealing each other's canvas, so VISIBLE work
// (Google Docs typing, visual grounding, compositor gestures) reserves the
// window's single presentation slot before it reselects a tab. Reads and other
// qualified hidden operations reserve nothing.
//
// This module is a pure state machine with an injected clock. It never calls
// Chrome; callers prove the tab/window identity themselves and hand it in.

export const PresentationRequirement = Object.freeze({
  NONE: "none",
  VISIBLE: "visible",
  FOREGROUND: "foreground",
});

export const PresentationConflict = Object.freeze({
  BUSY: "presentation_slot_busy",
  DEADLINE: "presentation_slot_deadline",
  WINDOW_UNKNOWN: "presentation_window_unknown",
});

function validId(value) {
  return Number.isSafeInteger(value) && value > 0;
}

function normalizedRequirement(value) {
  return Object.values(PresentationRequirement).includes(value) ? value : PresentationRequirement.NONE;
}

export class PresentationSlotManager {
  constructor({ now = () => Date.now(), wait = ms => new Promise(resolve => setTimeout(resolve, ms)), pollMs = 50, staleMs = 60_000 } = {}) {
    this.now = now;
    this.wait = wait;
    this.pollMs = Math.max(1, pollMs);
    this.staleMs = Math.max(1, staleMs);
    // window_id -> slot. The slot is the single source of truth for who may
    // change the active tab in that window.
    this.slots = new Map();
    this.nextToken = 1;
  }

  epoch(windowId) {
    return validId(windowId) ? (this.slots.get(windowId)?.epoch ?? 0) : 0;
  }

  owner(windowId) {
    const slot = validId(windowId) ? this.slots.get(windowId) : undefined;
    if (!slot?.owner) return null;
    return { ...slot.owner, epoch: slot.epoch };
  }

  // A slot is stale when its owner stopped revalidating it. Staleness is
  // measured from the last reservation or revalidation, so a long typing
  // transaction that never touches the slot again would look stale mid-run.
  // Callers revalidate per chunk; this bound only reclaims abandoned slots.
  #stale(slot, now) {
    return !slot.owner || now - slot.touched_ms > this.staleMs;
  }

  snapshot() {
    return [...this.slots.entries()].map(([windowId, slot]) => ({
      window_id: windowId,
      epoch: slot.epoch,
      owner: slot.owner ? { ...slot.owner } : null,
      touched_ms: slot.touched_ms,
    }));
  }

  async reserve({ windowId, tabId = null, leaseId = null, requirement = PresentationRequirement.VISIBLE, deadlineMs = 1500 } = {}) {
    const wanted = normalizedRequirement(requirement);
    if (wanted === PresentationRequirement.NONE) {
      return { granted: true, token: null, requirement: wanted, epoch: this.epoch(windowId) };
    }
    if (!validId(windowId)) {
      return { granted: false, reason: PresentationConflict.WINDOW_UNKNOWN, requirement: wanted };
    }
    const started = this.now();
    const deadline = started + Math.max(0, Number.isFinite(deadlineMs) ? deadlineMs : 1500);
    for (;;) {
      const now = this.now();
      let slot = this.slots.get(windowId);
      if (!slot) {
        slot = { epoch: 0, owner: null, touched_ms: now };
        this.slots.set(windowId, slot);
      }
      const current = slot.owner;
      const sameTask = current && current.lease_id !== null && leaseId !== null
        ? current.lease_id === leaseId
        : current !== null && leaseId === null && Number(tabId) === current.tab_id;
      if (!current || sameTask || this.#stale(slot, now)) {
        // Taking a slot is a presentation change, so it advances the epoch.
        // Every reservation (not only a change of owner) also touches the slot
        // so a long-lived owner is not reclaimed while it is still working.
        if (!current || !sameTask) slot.epoch += 1;
        slot.owner = {
          token: `presentation-${this.nextToken++}`,
          tab_id: validId(Number(tabId)) ? Number(tabId) : null,
          lease_id: leaseId ?? null,
          requirement: wanted,
          reserved_ms: current && sameTask ? current.reserved_ms : now,
        };
        slot.touched_ms = now;
        return {
          granted: true,
          token: slot.owner.token,
          requirement: wanted,
          window_id: windowId,
          epoch: slot.epoch,
          tab_id: slot.owner.tab_id,
          reclaimed: Boolean(current) && !sameTask,
        };
      }
      if (now >= deadline) {
        return {
          granted: false,
          reason: PresentationConflict.BUSY,
          requirement: wanted,
          window_id: windowId,
          epoch: slot.epoch,
          owner: { ...current },
        };
      }
      await this.wait(Math.min(this.pollMs, Math.max(1, deadline - now)));
    }
  }

  // Revalidation is generation-fenced: a token whose epoch was superseded by a
  // newer reservation no longer owns the window and must not dispatch input.
  async revalidate(token) {
    if (!token) return { valid: true, reason: null };
    for (const [windowId, slot] of this.slots) {
      if (slot.owner?.token !== token) continue;
      const now = this.now();
      if (now - slot.touched_ms > this.staleMs) return { valid: false, reason: "presentation_slot_stale", window_id: windowId, epoch: slot.epoch };
      slot.touched_ms = now;
      return { valid: true, window_id: windowId, epoch: slot.epoch, tab_id: slot.owner.tab_id };
    }
    // A superseded or released token is not the owner any more. Because a
    // takeover replaces the owner record, the old token cannot dispatch input
    // even though the epoch it carried no longer matches anything.
    return { valid: false, reason: "presentation_token_unknown" };
  }

  // Bind a reserved window slot to the concrete tab that was just created or
  // selected. The token stays valid; only the tab identity changes.
  async bind(token, tabId) {
    if (!token) return { bound: false, reason: "presentation_token_missing" };
    for (const [windowId, slot] of this.slots) {
      if (slot.owner?.token !== token) continue;
      slot.owner = { ...slot.owner, tab_id: validId(Number(tabId)) ? Number(tabId) : slot.owner.tab_id };
      slot.touched_ms = this.now();
      return { bound: true, window_id: windowId, epoch: slot.epoch, tab_id: slot.owner.tab_id };
    }
    return { bound: false, reason: "presentation_token_unknown" };
  }

  async release(token) {
    if (!token) return { released: false, reason: "presentation_token_missing" };
    for (const [windowId, slot] of this.slots) {
      if (slot.owner?.token !== token) continue;
      slot.owner = null;
      slot.touched_ms = this.now();
      slot.epoch += 1;
      return { released: true, window_id: windowId, epoch: slot.epoch };
    }
    return { released: false, reason: "presentation_token_unknown" };
  }

  // Administrative release for a closed tab or window. This is deliberately
  // separate from token release: Chrome can close a tab under any task, and a
  // dead owner must not block the window forever.
  async releaseWindow(windowId, { reason = "window_released" } = {}) {
    const slot = this.slots.get(windowId);
    if (!slot?.owner) return { released: false, reason: "slot_vacant" };
    const previous = { ...slot.owner };
    slot.owner = null;
    slot.touched_ms = this.now();
    slot.epoch += 1;
    return { released: true, reason, window_id: windowId, epoch: slot.epoch, previous_owner: previous };
  }

  // Reclaim slots whose tab is gone. Called before reserving so a crashed or
  // closed task never leaves a window permanently busy.
  async prune(isTabLive) {
    const reclaimed = [];
    for (const [windowId, slot] of this.slots) {
      if (!slot.owner || slot.owner.tab_id === null) continue;
      const live = await Promise.resolve(isTabLive(slot.owner.tab_id)).catch(() => false);
      if (live) continue;
      reclaimed.push({ window_id: windowId, owner: { ...slot.owner } });
      slot.owner = null;
      slot.touched_ms = this.now();
      slot.epoch += 1;
    }
    return reclaimed;
  }
}

// A reservation is only as good as the presentation it describes. These are
// the checks each requirement needs before dispatch, evaluated against live
// Chrome state and the page's own renderer report.
export function presentationProof({ tab, window, renderer = {}, epoch = 0 } = {}) {
  const visibility = ["visible", "hidden", "prerender"].includes(renderer.visibility_state) ? renderer.visibility_state : null;
  return {
    tab_active: tab?.active === true,
    window_focused: window?.focused === true,
    window_minimized: window?.state === "minimized",
    window_state: ["normal", "minimized", "maximized", "fullscreen"].includes(window?.state) ? window.state : "unknown",
    tab_frozen: tab?.frozen === true,
    tab_discarded: tab?.discarded === true || renderer.discarded === true,
    document_visibility: visibility,
    document_has_focus: typeof renderer.document_has_focus === "boolean" ? renderer.document_has_focus : null,
    renderer_responsive: renderer.renderer_unavailable !== true && renderer.prerendering !== true,
    presentation_epoch: epoch,
  };
}

export function presentationSatisfies(proof, requirement = PresentationRequirement.VISIBLE) {
  const wanted = normalizedRequirement(requirement);
  if (wanted === PresentationRequirement.NONE) return { ok: proof?.renderer_responsive !== false, missing: proof?.renderer_responsive === false ? ["renderer_responsive"] : [] };
  const missing = [];
  if (proof?.renderer_responsive === false) missing.push("renderer_responsive");
  if (proof?.tab_discarded !== false) missing.push("tab_not_discarded");
  if (proof?.tab_frozen === true) missing.push("tab_not_frozen");
  if (proof?.tab_active !== true) missing.push("tab_active");
  if (proof?.window_minimized !== false) missing.push("window_not_minimized");
  if (proof?.document_visibility !== "visible") missing.push("document_visible");
  if (wanted === PresentationRequirement.FOREGROUND && proof?.window_focused !== true) missing.push("window_focused");
  return { ok: missing.length === 0, missing };
}
