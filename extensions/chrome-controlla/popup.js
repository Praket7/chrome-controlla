const tabsEl = document.querySelector("#tabs");
const statusEl = document.querySelector("#status");
const nativeStatusEl = document.querySelector("#native-status");
const nativeTabsEl = document.querySelector("#native-tabs");

async function refreshNativeStatus() {
  try {
    const status = await chrome.runtime.sendMessage({ type: "native-status" });
    if (status?.error) {
      nativeStatusEl.textContent = `Native bridge unavailable: ${status.error}`;
    } else {
      nativeStatusEl.textContent = status?.connected
        ? "Native bridge connected"
        : "Native bridge unavailable. Expand Manual fallback to pair with a local WebSocket.";
    }
    const attached = Array.isArray(status?.attached) ? status.attached : [];
    nativeTabsEl.textContent = attached.length
      ? `Attached tab IDs: ${attached.join(", ")}`
      : "No tabs attached through the native bridge.";
  } catch (error) {
    nativeStatusEl.textContent = `Native bridge unavailable: ${error?.message || error}`;
    nativeTabsEl.textContent = "";
  }
}

refreshNativeStatus();
const statusPoll = setInterval(refreshNativeStatus, 1000);
window.addEventListener("unload", () => clearInterval(statusPoll), { once: true });

chrome.tabs.query({}).then(tabs => {
  for (const tab of tabs.filter(item => Number.isInteger(item.id) && item.url?.startsWith("http"))) {
    const label = document.createElement("label");
    const checkbox = document.createElement("input");
    checkbox.type = "checkbox";
    checkbox.value = String(tab.id);
    label.append(checkbox, ` [${tab.id}] ${tab.title || tab.url}`);
    tabsEl.append(label);
  }
});
document.querySelector("#pair").addEventListener("click", async () => {
  const tabIds = [...tabsEl.querySelectorAll("input:checked")].map(input => input.value);
  statusEl.textContent = "Attaching only the selected tabs…";
  const result = await chrome.runtime.sendMessage({
    type: "pair", endpoint: document.querySelector("#endpoint").value.trim(),
    token: document.querySelector("#token").value, tabIds
  });
  statusEl.textContent = result?.ok ? `Paired ${tabIds.length} selected tab(s).` : result?.error || "Pairing failed.";
});
document.querySelector("#release").addEventListener("click", async () => {
  const result = await chrome.runtime.sendMessage({ type: "release" });
  statusEl.textContent = result?.ok ? "Released debugger attachments from the selected tabs." : "Release failed.";
});
