// Cold-open lifecycle for Controlla-owned background tabs.
//
// This is intentionally the *whole* policy for a native open: reuse discovery,
// window presentation slot, tab creation, navigation, identity verification,
// receipt persistence, and normalized telemetry. background.js supplies the
// live Chrome bindings and the process-wide lock; it does not re-implement the
// lifecycle. Keeping it here is also what keeps that file under its
// decomposition ceiling.
//
// Navigation deliberately runs outside the state lock: two independent opens
// must overlap instead of serializing behind one slow page load.

import {
  findReusableOwnedBackgroundCandidate, activateReservedPresentation, findOwnedBackgroundWindow,
  pruneOpenReceipts,
} from "./tab-control.js";
import { PresentationRequirement, presentationProof } from "./presentation-slots.js";
import { ReuseKind, TabDurability, openTelemetry } from "./owned-resources.js";

let deps = null;
const inflight = new Map();

export function configureOwnedTabOpener(next) {
  deps = next;
  return { openOwnedTabFromNative };
}

// Pick an owned automation window whose presentation slot is free and reserve
// it. Windows with an active presenter are skipped, so simultaneous visible
// tasks land in different windows instead of hiding each other. Exported
// because the shared-session open path in background.js uses the same policy.
export async function reserveOwnedBackgroundSlot(receipts) {
  const scanned = new Set();
  for (;;) {
    const found = await findOwnedBackgroundWindow(deps.chrome.windows, receipts, {
      isWindowBusy: windowId => scanned.has(windowId) || Boolean(deps.presentationSlots.owner(windowId)),
    });
    if (!found) return null;
    const slot = await deps.presentationSlots.reserve({
      windowId: found.windowId, requirement: PresentationRequirement.VISIBLE, deadlineMs: 0,
    });
    if (slot.granted) return { ...found, slot };
    scanned.add(found.windowId);
  }
}

async function openOwnedTabFromNative(request) {
  const url = deps.validateUrl(request.url);
  const key = `ownerless:${request.idempotency_key}`;
  if (typeof request.idempotency_key !== "string" || request.idempotency_key.length < 8 || request.idempotency_key.length > 256
    || /[\u0000-\u001f\u007f]/.test(request.idempotency_key)) throw new Error("Native tab creation requires a bounded idempotency key.");
  const active = request.active === true;
  if (typeof request.active !== "boolean") throw new Error("Native tab creation requires an explicit active boolean.");
  const backgroundWindow = request.create_background_window === true;
  const targetWindowId = request.target_window_id == null ? undefined : Number(request.target_window_id);
  const targetWindowPolicy = targetWindowId === undefined ? undefined : request.target_window_policy;
  if ((targetWindowId !== undefined && (!Number.isSafeInteger(targetWindowId) || targetWindowId <= 0 || backgroundWindow))
    || (backgroundWindow && active)
    || (targetWindowId !== undefined && !["existing_window", "existing_automation_window"].includes(targetWindowPolicy)))
    throw new Error("Native tab creation window policy is invalid.");
  const now = Math.floor(Date.now() / 1000);
  if (!Number.isSafeInteger(request.expires_at_unix) || request.expires_at_unix < now || request.expires_at_unix > now + 30) throw new Error("Native tab creation request expired.");
  const previous = inflight.get(key);
  if (previous && previous.url === url && previous.active === active && previous.backgroundWindow === backgroundWindow
    && previous.targetWindowId === targetWindowId && previous.targetWindowPolicy === targetWindowPolicy) return previous.promise;
  const operation = (async () => {
    const reservation = await deps.withStateLock(async () => {
      const saved = await deps.chrome.storage.local.get(deps.receiptsKey);
      const receipts = saved[deps.receiptsKey] || {};
      const receipt = receipts[key];
      if (receipt && (receipt.requested_url !== url || receipt.active !== active || receipt.create_background_window !== backgroundWindow || receipt.target_window_id !== targetWindowId || receipt.target_window_policy !== targetWindowPolicy))
        throw new Error("Idempotency key is already bound to a different tab creation request.");
      if (receipt?.status === "complete") {
        const tab = await deps.chrome.tabs.get(Number(receipt.tab_id)).catch(() => undefined);
        const identity = tab && await deps.identityOf(tab.id).catch(() => undefined);
        if (!identity || identity.document_id !== receipt.document_id || identity.url !== receipt.url)
          throw new Error(`Owned tab ${receipt.tab_id} changed after creation; inspect it before retrying.`);
        if (targetWindowId !== undefined && tab.windowId !== targetWindowId)
          throw new Error(`Owned tab ${receipt.tab_id} moved from the requested window; inspect it before retrying.`);
        const window = await deps.chrome.windows.get(tab.windowId);
        return {
          done: {
            tab_id: receipt.tab_id, window_id: tab.windowId, url: identity.url, document_id: identity.document_id,
            idempotent_reuse: true, created_by_controlla: true, created_background_window: backgroundWindow,
            tab_active: !!tab.active, window_focused: !!window.focused,
            ...openTelemetry({ created: false, windowCreated: false, reuseKind: ReuseKind.EXACT_DOCUMENT }),
          },
        };
      }
      const markerUrl = `${deps.chrome.runtime.getURL("open-recovery.html")}#${encodeURIComponent(key)}`;
      let tab = receipt?.tab_id && await deps.chrome.tabs.get(Number(receipt.tab_id)).catch(() => undefined);
      if (receipt && !tab) throw new Error(`Owned tab ${receipt.tab_id} disappeared during creation; no duplicate was opened.`);
      if (tab && tab.url !== markerUrl && tab.url !== url) throw new Error("Owned recovery tab changed before completion; no duplicate was opened.");
      if (targetWindowPolicy === "existing_automation_window") await deps.assertAutomationWindow(targetWindowId);
      else if (targetWindowId !== undefined) await deps.chrome.windows.get(targetWindowId);

      let reusedWindowId;
      let reusedTab = null;
      let slotToken = null;
      let createdWindow = false;
      if (!tab && backgroundWindow && targetWindowId === undefined) {
        // A dead owner must not keep a window busy forever.
        await deps.presentationSlots.prune(tabId => deps.chrome.tabs.get(tabId).then(() => true).catch(() => false));
        const candidate = await findReusableOwnedBackgroundCandidate(deps.chrome, receipts, url, deps.identityOf, {
          isLeased: tabId => deps.isTabLeased(tabId),
          allowResourceNavigation: true,
          // Recycling is opt-in: only a caller that says an idle ephemeral tab
          // may be repurposed gets one.
          allowRecycle: request.recycle_idle_tabs === true,
          minIdleMs: 60_000,
        });
        if (candidate) {
          const slot = await deps.presentationSlots.reserve({
            windowId: candidate.window_id, tabId: candidate.tab.id,
            requirement: PresentationRequirement.VISIBLE, deadlineMs: 0,
          });
          if (slot.granted) {
            const activation = await activateReservedPresentation(deps.chrome, candidate);
            if (activation.ok) {
              tab = activation.tab;
              slotToken = slot.token;
              reusedTab = {
                kind: candidate.kind, navigate: candidate.navigate,
                from_url: candidate.current_url, document_id: candidate.document_id,
              };
            } else {
              await deps.presentationSlots.release(slot.token);
            }
          }
        }
      }
      if (!tab && backgroundWindow) {
        // Reuse an owned window whose presentation slot is free. A window that
        // another task is presenting in is skipped, never stolen; when every
        // owned window is busy, concurrency decides the new window.
        const reserved = await reserveOwnedBackgroundSlot(receipts);
        if (reserved) {
          tab = await deps.chrome.tabs.create({ windowId: reserved.windowId, url: markerUrl, active: true });
          reusedWindowId = reserved.windowId;
          slotToken = reserved.slot.token;
        } else {
          const window = await deps.chrome.windows.create({ url: markerUrl, focused: false, type: "normal" });
          tab = window?.tabs?.find(candidate => Number.isSafeInteger(candidate.id));
          if (!tab || !Number.isSafeInteger(window?.id)) throw new Error("Chrome did not return the new unfocused Controlla window and tab.");
          createdWindow = true;
          const slot = await deps.presentationSlots.reserve({
            windowId: window.id, tabId: tab.id, requirement: PresentationRequirement.VISIBLE, deadlineMs: 0,
          });
          slotToken = slot.granted ? slot.token : null;
        }
        if (slotToken) await deps.presentationSlots.bind(slotToken, tab.id);
      } else if (!tab && targetWindowId !== undefined) {
        tab = await deps.chrome.tabs.create({ windowId: targetWindowId, url: markerUrl, active });
      } else if (!tab) {
        tab = await deps.chrome.tabs.create({ url: markerUrl, active });
      }
      if (!tab || !Number.isSafeInteger(tab.id)) throw new Error("Chrome did not return a new owned tab.");
      const nowMs = Date.now();
      receipts[key] = {
        status: "creating", tab_id: String(tab.id), window_id: tab.windowId,
        target_window_id: targetWindowId, target_window_policy: targetWindowPolicy,
        url, requested_url: url, marker_url: markerUrl, active, create_background_window: backgroundWindow,
        reused_background_window: reusedWindowId !== undefined, created_ms: receipt?.created_ms || nowMs,
        durability: backgroundWindow ? TabDurability.EPHEMERAL : TabDurability.SESSION,
        last_used_ms: nowMs, last_verified_ms: nowMs,
      };
      await deps.chrome.storage.local.set({ [deps.receiptsKey]: receipts });
      return { tab, slotToken, createdWindow, reusedWindowId, reusedTab };
    });
    if (reservation.done) return reservation.done;
    const { tab, slotToken, reusedTab } = reservation;
    // An open's reservation covers the presentation change itself. It is
    // released when the open returns; long-lived presentation (typing into a
    // canvas editor) takes its own lease for the duration of the transaction.
    try {
      // Navigation and readiness live outside the state lock: another open may
      // reserve its own window meanwhile, and two slow loads overlap.
      if (tab.url !== url) await deps.chrome.tabs.update(tab.id, { url });
      await deps.waitForReady(tab.id, null);
      let identity = await deps.identityOf(tab.id);
      let redirectedFrom;
      if (identity.url !== url) {
        // A same-origin server redirect is a normal creation flow: an app hands
        // out a new document under its own URL. Re-bind to the settled document
        // once it finishes loading instead of failing an open that worked, and
        // never follow the redirect off the requested origin.
        let requestedOrigin;
        try { requestedOrigin = new URL(url).origin; } catch { requestedOrigin = null; }
        if (!requestedOrigin || new URL(identity.url).origin !== requestedOrigin)
          throw new Error("Created tab navigated away from the requested origin before identity verification.");
        await deps.waitForReady(tab.id, null);
        identity = await deps.identityOf(tab.id);
        if (new URL(identity.url).origin !== requestedOrigin)
          throw new Error("Created tab left the requested origin during its redirect.");
        if (identity.url !== url) redirectedFrom = url;
      }
      const [current, window] = await Promise.all([deps.chrome.tabs.get(tab.id), deps.chrome.windows.get(tab.windowId)]);
      if (targetWindowId !== undefined && current.windowId !== targetWindowId) throw new Error("Selected window changed during tab creation.");
      if (targetWindowPolicy === "existing_automation_window" && (window.focused || !current.active)) throw new Error("Selected automation window changed during tab creation.");
      if (reservation.reusedTab && window.state === "minimized")
        throw new Error("The automation window was minimized before the reused tab could be presented.");
      if (reservation.reusedTab && window.focused === true)
        throw new Error("The automation window became focused while the reused tab was being verified.");
      const latest = (await deps.chrome.storage.local.get(deps.receiptsKey))[deps.receiptsKey] || {};
      const reuseKind = reusedTab?.kind ?? null;
      const nowMs = Date.now();
      latest[key] = {
        ...(latest[key] || {}), status: "complete", url: identity.url, requested_url: url,
        document_id: identity.document_id, window_id: current.windowId,
        redirected_from: redirectedFrom ?? null,
        durability: latest[key]?.durability ?? (backgroundWindow ? TabDurability.EPHEMERAL : TabDurability.SESSION),
        last_used_ms: nowMs, last_verified_ms: nowMs,
        reused_open_tab: Boolean(reusedTab), reuse_kind: reuseKind,
      };
      await deps.chrome.storage.local.set({ [deps.receiptsKey]: latest });
      const proof = presentationProof({ tab: current, window, epoch: deps.presentationSlots.epoch(current.windowId) });
      return {
        tab_id: String(tab.id), window_id: current.windowId, url: identity.url, document_id: identity.document_id,
        redirected_from: redirectedFrom ?? null,
        idempotent_reuse: Boolean(reusedTab), created_by_controlla: true,
        created_background_window: backgroundWindow,
        reused_background_window: reservation.reusedWindowId !== undefined || reservation.createdWindow === false,
        reused_open_tab: Boolean(reusedTab),
        tab_active: !!current.active, window_focused: !!window.focused,
        presentation: { ...proof, visibility_deferred: true },
        ...openTelemetry({
          created: !reusedTab,
          windowCreated: reservation.createdWindow === true,
          reuseKind: reuseKind ?? (reservation.createdWindow ? ReuseKind.NEW_TAB_NEW_WINDOW : ReuseKind.NEW_TAB_EXISTING_WINDOW),
        }),
      };
    } catch (error) {
      const latest = (await deps.chrome.storage.local.get(deps.receiptsKey))[deps.receiptsKey] || {};
      latest[key] = { ...(latest[key] || {}), status: "failed" };
      await deps.chrome.storage.local.set({ [deps.receiptsKey]: latest });
      throw new Error(`Owned tab ${tab.id} remains open for inspection after creation failed: ${deps.safeError(String(error))}`);
    } finally {
      if (slotToken) await deps.presentationSlots.release(slotToken);
    }
  })();
  inflight.set(key, { url, active, backgroundWindow, targetWindowId, targetWindowPolicy, promise: operation });
  try {
    return await operation;
  } finally {
    if (inflight.get(key)?.promise === operation) inflight.delete(key);
  }
}

