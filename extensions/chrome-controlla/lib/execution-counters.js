// Bounded execution telemetry shared by the extension background.
//
// Counters are never model-visible on ordinary success; they surface only in
// diagnostics or explicit qualification receipts.

export const executionCounters = {
  cdp_commands: 0,
  browser_commands: 0,
  native_host_commands: 0,
  full_ax_reads: 0,
  frame_tree_reads: 0,
  target_rebinds: 0,
  automatic_recoveries: 0,
  user_visible_recoveries: 0,
  auto_attach_calls: 0,
  input_commands: 0,
  verification_reads: 0,
  by_method: Object.create(null),
};

export function resetExecutionCounters() {
  for (const key of Object.keys(executionCounters)) {
    if (key === "by_method") executionCounters.by_method = Object.create(null);
    else executionCounters[key] = 0;
  }
}

export function noteCdpCommand(method) {
  executionCounters.cdp_commands += 1;
  executionCounters.by_method[method] = (executionCounters.by_method[method] || 0) + 1;
  if (method === "Accessibility.getFullAXTree") executionCounters.full_ax_reads += 1;
  else if (method === "Page.getFrameTree") executionCounters.frame_tree_reads += 1;
  else if (method === "Target.setAutoAttach") executionCounters.auto_attach_calls += 1;
  else if (method === "Input.dispatchKeyEvent" || method === "Input.insertText" || method === "Input.imeSetComposition" || method === "Input.dispatchMouseEvent" || method === "Input.dispatchDragEvent" || method === "Input.dispatchTouchEvent") executionCounters.input_commands += 1;
  else if (method === "Runtime.callFunctionOn" || method === "Runtime.evaluate") executionCounters.verification_reads += 1;
}

export function executionCounterReceipt() {
  return { ...executionCounters, by_method: { ...executionCounters.by_method } };
}
