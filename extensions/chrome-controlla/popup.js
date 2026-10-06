const tabsEl = document.querySelector("#tabs");
const statusEl = document.querySelector("#status");
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
