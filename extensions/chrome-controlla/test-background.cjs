const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");

const sockets = [];
const detached = [];
let listener;
class FakeWebSocket {
  static OPEN = 1;
  constructor(url) {
    this.url = url;
    this.readyState = 0;
    sockets.push(this);
    queueMicrotask(() => { this.readyState = 1; this.onopen?.(); });
  }
  send(payload) {
    if (JSON.parse(payload).type === "hello") {
      queueMicrotask(() => this.onmessage?.({ data: JSON.stringify({ type: "ready" }) }));
    }
  }
  close() { this.readyState = 3; this.onclose?.(); }
}
const chrome = {
  runtime: { onMessage: { addListener(callback) { listener = callback; } } },
  debugger: {
    async attach() {},
    async detach({ tabId }) { detached.push(tabId); },
    onDetach: { addListener() {} },
  },
};
vm.runInNewContext(fs.readFileSync(new URL("./background.js", `file://${__filename}`), "utf8"), {
  chrome, WebSocket: FakeWebSocket, URL, Set, Number, Array, String, Error, Promise, JSON, setTimeout,
  clearTimeout,
});

function message(payload) {
  return new Promise(resolve => listener(payload, {}, resolve));
}

(async () => {
  assert.equal((await message({ type: "pair", endpoint: "ws://127.0.0.1:1234/", token: "one", tabIds: [17] })).ok, true);
  const oldSocket = sockets[0];
  assert.equal((await message({ type: "pair", endpoint: "ws://127.0.0.1:1234/", token: "two", tabIds: [18] })).ok, true);
  const detachCountBeforeOldClose = detached.length;
  oldSocket.onclose();
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(detached.length, detachCountBeforeOldClose, "an old pairing must not detach newer tabs");
  console.log("Extension pairing-generation cleanup test passed.");
})().catch(error => { console.error(error); process.exitCode = 1; });
