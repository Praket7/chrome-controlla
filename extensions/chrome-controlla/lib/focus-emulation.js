// Chrome focus emulation for automation tabs.
//
// A background tab does not run focused page code. Emulation.setFocusEmulationEnabled
// makes document.hasFocus()/visibility handlers coherent without activating the
// tab or raising the user's window. It is enabled lazily for commands that can
// observe focus, and disabled before detach.

const FOCUS_NEUTRAL_METHODS = new Set([
  "DOM.getDocument", "DOM.querySelector", "DOM.querySelectorAll", "DOM.enable",
  "DOM.describeNode", "DOM.resolveNode", "DOM.getFrameOwner",
  "DOM.setFileInputFiles",
]);

export function commandNeedsPageFocus(method) {
  if (FOCUS_NEUTRAL_METHODS.has(method)) return false;
  return method.startsWith("Runtime.")
    || method.startsWith("DOM.")
    || method.startsWith("Input.")
    || method.startsWith("Accessibility.")
    || method === "Controlla.fastType"
    || method === "Controlla.humanTypeCharacter"
    || method === "Controlla.typeTextTransaction"
    || method === "Controlla.securePasswordFill"
    || method === "Controlla.checkIdentityMarker"
    || method === "Controlla.fileInputSnapshot"
    || method === "Controlla.findFileInput"
    || method === "Controlla.setFrameFileInputFiles"
    || method.startsWith("Controlla.frame")
    || method.startsWith("Controlla.pointer")
    || method.startsWith("Controlla.scroll");
}

export function createFocusEmulation() {
  const tabs = new Set();
  return {
    has: tabId => tabs.has(tabId),
    forget: tabId => tabs.delete(tabId),
    async ensure(tabId) {
      if (tabs.has(tabId)) return true;
      try {
        await chrome.debugger.sendCommand({ tabId }, "Emulation.setFocusEmulationEnabled", { enabled: true });
        tabs.add(tabId);
        return true;
      } catch {
        tabs.delete(tabId);
        return false;
      }
    },
    async disable(tabId) {
      if (!tabs.has(tabId)) return;
      try { await chrome.debugger.sendCommand({ tabId }, "Emulation.setFocusEmulationEnabled", { enabled: false }); } catch {}
      tabs.delete(tabId);
    },
  };
}
