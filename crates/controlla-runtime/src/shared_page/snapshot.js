function(q) {
  const roots = document.querySelectorAll(q.selector);
  if (roots.length !== 1) throw Error('snapshot scope must match exactly one element');
  const root = roots[0], nodes = [], items = [];
  const protectedSelector = '[data-masked],[data-requires-trusted],input[type="password"],input[type="file"],input[type="hidden"]';
  const readable = e => e && !e.closest(protectedSelector) && !e.querySelector(protectedSelector);
  const visible = e => {
    const s = getComputedStyle(e), b = e.getBoundingClientRect();
    return b.width > 0 && b.height > 0 && s.visibility === 'visible' && s.display !== 'none' && !e.closest('[hidden],[inert]');
  };
  const raw = e => /^(INPUT|TEXTAREA|SELECT)$/.test(e.tagName) ? e.value : (e.innerText ?? '');
  const name = e => {
    const labelled = (e.getAttribute('aria-labelledby') || '').split(/\s+/).filter(Boolean).map(id => { const label=document.getElementById(id); return readable(label) ? label.innerText : ''; }).join(' ').trim();
    return (labelled || e.getAttribute('aria-label') || e.innerText || e.getAttribute('alt') || Array.from(e.labels || []).filter(readable).map(l => l.innerText).join(' ') || e.getAttribute('title') || '').replace(/\s+/g,' ').trim();
  };
  const selector = 'a[href],button,input,textarea,select,summary,[role="button"],[role="link"],[role="menuitem"],[role="checkbox"],[role="radio"],[role="tab"]';
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT);
  let e = root, scanned = 0, truncated = false, bytes = 0;
  const encoder = new TextEncoder();
  while(e) {
    if (++scanned > 65536) { truncated = true; break; }
    if (e.matches(selector) && visible(e) && readable(e)) {
      if (items.length >= q.max_items) { truncated = true; break; }
      const value = raw(e), label = name(e);
      // Oversized controls are omitted explicitly; never give the action tool a clipped expected value.
      if (value.length > 16384 || label.length > 2000) { truncated = true; e = walker.nextNode(); continue; }
      const item = {reference:q.token+':'+items.length,role:e.getAttribute('role') || ({A:'link',BUTTON:'button',INPUT:'input',TEXTAREA:'textbox',SELECT:'combobox',SUMMARY:'button'}[e.tagName] || 'control'),name:label,raw_value:value,visible:true,disabled:e.matches(':disabled') || e.getAttribute('aria-disabled')==='true',href:e instanceof HTMLAnchorElement ? e.href : null,expanded:e.getAttribute('aria-expanded'),selected:e.getAttribute('aria-selected') || e.getAttribute('aria-checked'),controls_selector:e.getAttribute('aria-controls')?.split(/\s+/).filter(Boolean).map(id=>'#'+CSS.escape(id)).join(',') || null};
      const size = encoder.encode(JSON.stringify(item)).length;
      if (bytes + size > q.max_bytes - q.max_text_chars * 6 - 2048) { truncated = true; break; }
      bytes += size; nodes.push(e); items.push(item);
    }
    e = walker.nextNode();
  }
  // Visible text is a page excerpt, never evidence that all assignments or records were enumerated.
  const texts = document.createTreeWalker(root,NodeFilter.SHOW_TEXT);
  let text='', textNode, textScanned=0;
  while ((textNode=texts.nextNode()) && ++textScanned<=65536 && text.length<=q.max_text_chars) {
    const p=textNode.parentElement;
    if (p && !p.closest(protectedSelector) && visible(p)) {
      const part=textNode.textContent.trim(); if (part) text+=(text?'\n':'')+part;
    }
  }
  return {nodes,items,raw,name,visible,public:{url:location.href,title:document.title,ready_state:document.readyState,items,text:text.slice(0,q.max_text_chars),text_truncated:!!textNode||text.length>q.max_text_chars,truncated,coverage:'visible light-DOM controls in this scope; excludes iframe and shadow-root content',scanned_nodes:Math.min(scanned,65536)}};
}
