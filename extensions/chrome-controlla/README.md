# Chrome Controlla Shared Tab Bridge

This unpacked Manifest V3 extension pairs a shared session with only the tabs the user checks in its popup. It uses `chrome.debugger` for those tabs and a one-session authenticated WebSocket bound to `127.0.0.1`. The pairing token is supplied by the local Rust `SharedExtensionProvider`; it is not stored by the extension.

Shared-session target identifiers are decimal Chrome `tabs.Tab.id` values. They are a different identity namespace from CDP target IDs used by dedicated direct-CDP sessions. The Rust provider checks exact selected-tab membership and rechecks the live session grant before every command. `Browser.*` and `Target.*` commands are rejected; direct CDP dispatch is not used for shared mode.

For MCP pairing, call `session` with `action: "pair_shared"` and the exact numeric IDs shown beside tabs in this popup. The server returns a loopback endpoint and one-session token. Call `accept_shared`, check the same tabs here, enter both values, and pair. This route uses the extension's `chrome.debugger` permission; it does not require Chrome DevTools remote debugging to be enabled. Then call `list_shared_targets` or `shared_observe` with one selected `chrome_tab_id`. Use `release_shared` to detach. Shared observation checks root frame ID, loader ID, and URL before and after a bounded read; it reports no Direct CDP `TargetRef` or navigation epoch. Shared extraction and guarded input remain unavailable.

For development, load this directory with Chrome's “Load unpacked” extension flow, then use the popup to enter the loopback endpoint and one-session token and check the intended tabs. The popup includes an explicit release action; closing the provider or losing its WebSocket also detaches tabs. Re-pair by checking the intended tabs again and entering a new endpoint/token.

The Rust loopback fixture tests cover authentication, selected-tab binding, command routing, and grant revocation. They do not exercise a real Chrome extension installation, Chrome's native debugger consent UI, browser-version compatibility, or platform-specific focus/foreground behavior. Those live qualification gates remain open.
