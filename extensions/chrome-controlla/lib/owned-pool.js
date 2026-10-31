// Bounded owned-tab pool.
//
// The receipt table may remember more history than Chrome should keep alive.
// Eviction closes only owned ephemeral tabs that are idle, unleased, not
// attached by a debugger, and not currently presented. Refusing when no safe
// victim exists is better than exceeding the bound or closing a live owner.

import { indexOwnedResources, planPoolEviction } from "./owned-resources.js";

export async function evictOwnedPoolTabs({
  receipts,
  limit,
  presentedTabIds,
  liveTabs,
  isLeased,
  isTabUnavailable,
  closeTab,
}) {
  const index = indexOwnedResources(receipts, { now: Date.now() });
  const presented = new Set(presentedTabIds);
  const activeTabs = new Set(
    liveTabs.filter(tab => tab.active === true)
      .map(tab => Number(tab.id))
      .filter(Number.isSafeInteger),
  );
  const plan = planPoolEviction(index, {
    limit,
    now: Date.now(),
    keepTabIds: [],
    isLeased,
    isTabUnavailable,
    isTabProtected: tabId => presented.has(tabId) || activeTabs.has(tabId),
  });
  if (plan.within_limit) return { plan, evicted: [] };
  if (plan.blocked) {
    throw new Error(`Owned tab pool is at its ${limit}-tab bound and no idle ephemeral tab can be evicted safely; close or release an owned tab before opening another.`);
  }
  const evicted = [];
  for (const record of plan.evict) {
    try { await closeTab(record.tab_id); } catch { /* closed between planning and eviction */ }
    for (const key of record.keys) {
      const receipt = receipts[key];
      if (!receipt) continue;
      receipts[key] = {
        ...receipt,
        status: "closed",
        closed_ms: Date.now(),
        closed_reason: "owned_pool_eviction",
      };
    }
    evicted.push({ tab_id: String(record.tab_id), window_id: record.window_id, reason: "owned_pool_eviction" });
  }
  return { plan, evicted };
}
