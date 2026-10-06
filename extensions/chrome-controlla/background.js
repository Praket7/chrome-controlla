let socket;
let attachedTabs = new Set();
let pairingGeneration = 0;

async function detachAll(generation) {
  if (generation !== pairingGeneration) return;
  const toDetach = [...attachedTabs];
  attachedTabs = new Set();
  await Promise.all(toDetach.map(tabId => chrome.debugger.detach({ tabId }).catch(() => {})));
}

chrome.runtime.onMessage.addListener((message, _sender, respond) => {
  if (message?.type === "pair") {
    pair(message.endpoint, message.token, message.tabIds)
      .then(() => respond({ ok: true }))
      .catch(error => respond({ ok: false, error: String(error) }));
    return true;
  }
  if (message?.type === "status") {
    respond({ connected: socket?.readyState === WebSocket.OPEN, attached: [...attachedTabs] });
  }
  if (message?.type === "release") {
    const current = socket;
    const generation = ++pairingGeneration;
    socket = undefined;
    current?.close();
    detachAll(generation).then(() => respond({ ok: true }));
    return true;
  }
});

async function pair(endpoint, token, selectedTabIds) {
  if (!Array.isArray(selectedTabIds) || selectedTabIds.length === 0 || !token || !endpoint) {
    throw new Error("Choose tabs and provide the local pairing endpoint and token.");
  }
  const tabs = [...new Set(selectedTabIds.map(Number))];
  if (tabs.some(id => !Number.isInteger(id) || id < 0)) throw new Error("Invalid tab selection.");
  const endpointUrl = new URL(endpoint);
  if (endpointUrl.protocol !== "ws:" || endpointUrl.hostname !== "127.0.0.1"
      || endpointUrl.username || endpointUrl.password || endpointUrl.search || endpointUrl.hash) {
    throw new Error("The pairing endpoint must use ws://127.0.0.1.");
  }
  const generation = ++pairingGeneration;
  const previousSocket = socket;
  socket = undefined;
  previousSocket?.close();
  await detachAll(generation);
  try {
    for (const tabId of tabs) {
      await chrome.debugger.attach({ tabId }, "1.3");
      if (generation !== pairingGeneration) {
        await chrome.debugger.detach({ tabId }).catch(() => {});
        throw new Error("Pairing was replaced.");
      }
      attachedTabs.add(tabId);
    }
  } catch (error) {
    await detachAll(generation);
    throw error;
  }
  const next = new WebSocket(endpoint);
  socket = next;
  try {
    await new Promise((resolve, reject) => {
      next.onopen = resolve;
      next.onerror = () => reject(new Error("Cannot connect to the loopback provider."));
      next.onclose = () => reject(new Error("Loopback pairing was closed."));
    });
    if (generation !== pairingGeneration) throw new Error("Pairing was replaced.");
    next.send(JSON.stringify({ type: "hello", token, targets: [...attachedTabs].map(String).sort() }));
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("Provider did not confirm pairing.")), 5000);
      next.onmessage = event => {
        const reply = JSON.parse(event.data);
        if (reply.type === "ready") { clearTimeout(timer); resolve(); }
        else { clearTimeout(timer); reject(new Error(reply.error || "Pairing rejected.")); }
      };
    });
  } catch (error) {
    next.close();
    if (socket === next) socket = undefined;
    await detachAll(generation);
    throw error;
  }
  if (generation !== pairingGeneration) {
    next.close();
    return;
  }
  next.onmessage = async event => {
    const request = JSON.parse(event.data);
    if (request.type !== "command" || !attachedTabs.has(Number(request.target_id))) return;
    try {
      const result = await chrome.debugger.sendCommand(
        { tabId: Number(request.target_id) }, request.method, request.params || {}
      );
      next.send(JSON.stringify({ type: "result", id: request.id, result }));
    } catch (error) {
      next.send(JSON.stringify({ type: "result", id: request.id, error: String(error) }));
    }
  };
  next.onclose = () => {
    if (generation !== pairingGeneration) return;
    socket = undefined;
    void detachAll(generation);
  };
  next.onerror = () => {
    if (generation !== pairingGeneration) return;
    socket = undefined;
    void detachAll(generation);
  };
}

chrome.debugger.onDetach.addListener(({ tabId }) => {
  if (Number.isInteger(tabId)) attachedTabs.delete(tabId);
});
