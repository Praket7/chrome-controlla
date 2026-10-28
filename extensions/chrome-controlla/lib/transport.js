// Bounded Chrome DevTools Protocol transport.
// The service worker owns authorization and target identity; this module only
// enforces a monotonic deadline and reports whether dispatch may have occurred.

export function createBoundedTransport({
  sendCommand,
  maxTimeoutMs = 60000,
  onCommand = () => {},
} = {}) {
  if (typeof sendCommand !== "function") throw new Error("sendCommand is required");
  const limit = Number.isFinite(maxTimeoutMs) ? Math.max(1, Math.min(120000, maxTimeoutMs)) : 60000;

  async function send(target, method, params, timeoutMs = 30000) {
    const boundedTimeoutMs = Number.isFinite(timeoutMs)
      ? Math.max(1, Math.min(limit, timeoutMs))
      : Math.min(limit, 30000);
    try {
      onCommand(method, target, params);
    } catch {
      // Telemetry must never change dispatch semantics.
    }
    let timer;
    try {
      return await Promise.race([
        sendCommand(target, method, params),
        new Promise((_, reject) => {
          timer = setTimeout(
            () =>
              reject(
                Object.assign(
                  new Error(`CDP command ${method} exceeded its ${boundedTimeoutMs}ms deadline.`),
                  { reason_code: "cdp_command_timeout", dispatch_may_have_occurred: true },
                ),
              ),
            boundedTimeoutMs,
          );
        }),
      ]);
    } finally {
      clearTimeout(timer);
    }
  }

  return { send, maxTimeoutMs: limit };
}
