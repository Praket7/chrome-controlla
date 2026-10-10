function(q) {
  const roots = document.querySelectorAll(q.selector);
  if (roots.length !== 1) throw Error('snapshot scope must match exactly one element');
  const root = roots[0], nodes = [], items = [];
  const protectedSelector = '[data-masked],[data-requires-trusted],input[type="password"],input[type="file"],input[type="hidden"]';
  const trustedText = e => e?.hasAttribute('data-requires-trusted') && (e instanceof HTMLTextAreaElement || e instanceof HTMLInputElement && ['text','search','email','url','tel'].includes(e.type));
  const readable = e => {
    if (!e || e.closest('[data-masked],input[type="password"],input[type="file"],input[type="hidden"]') || e.querySelector(protectedSelector)) return false;
    const trusted = e.closest('[data-requires-trusted]');
    return !trusted || trusted === e && trustedText(e);
  };
  const visible = e => {
    const s = getComputedStyle(e), b = e.getBoundingClientRect();
    return b.width > 0 && b.height > 0 && s.visibility === 'visible' && s.display !== 'none' && !e.closest('[hidden],[inert]');
  };
  const raw = e => /^(INPUT|TEXTAREA|SELECT)$/.test(e.tagName) ? e.value : (e.innerText ?? '');
  const name = e => {
    const labelled = (e.getAttribute('aria-labelledby') || '').split(/\\s+/).filter(Boolean).map(id => { const label=document.getElementById(id); return readable(label) ? label.innerText : ''; }).join(' ').trim();
    return (labelled || e.getAttribute('aria-label') || e.innerText || e.getAttribute('alt') || Array.from(e.labels || []).filter(readable).map(l => l.innerText).join(' ') || e.getAttribute('title') || '').replace(/\s+/g,' ').trim();
  };
  const datetimeOf = e => {
    if (e.getAttribute && e.getAttribute('datetime')) return e.getAttribute('datetime');
    const ancestor = e.closest('article,section,[data-section],li');
    if (ancestor) {
      const t = ancestor.querySelector('time[datetime]');
      if (t) return t.getAttribute('datetime');
      const d = ancestor.getAttribute('data-datetime') || ancestor.getAttribute('data-timestamp');
      if (d) return d;
    }
    const siblingTime = e.parentElement ? e.parentElement.querySelector('time[datetime]') : null;
    if (siblingTime) return siblingTime.getAttribute('datetime');
    return null;
  };
  const sectionOf = e => {
    const s = e.closest('section,[data-section],article,nav,main');
    if (!s) return null;
    return s.getAttribute('data-section') || s.getAttribute('aria-label') || s.tagName.toLowerCase();
  };
  const semanticAncestor = e => {
    const a = e.closest('article,li,[role="article"],[data-card],section');
    if (!a) return null;
    return {tag: a.tagName.toLowerCase(), id: a.getAttribute('data-id') || a.getAttribute('id') || null};
  };
  const domPathFingerprint = e => {
    let cur = e, parts = [], depth = 0;
    while (cur && cur !== root && depth < 5) {
      const tag = cur.tagName ? cur.tagName.toLowerCase() : '';
      const idx = cur.parentElement ? Array.prototype.indexOf.call(cur.parentElement.children, cur) : 0;
      parts.unshift(tag+':'+idx);
      cur = cur.parentElement; depth++;
    }
    return parts.join('>');
  };
  const rectBucket = b => ({w: Math.round(b.width/50)*50, h: Math.round(b.height/50)*50, x: Math.round(b.left/100)*100, y: Math.round(b.top/100)*100});
  const selector = 'a[href],button,input,textarea,select,summary,[role="button"],[role="link"],[role="menuitem"],[role="checkbox"],[role="radio"],[role="tab"]';
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT);
  let e = root, scanned = 0, truncated = false, bytes = 0, omitted_by_limit = 0, omitted_oversized = 0;
  let truncation_reason = null;
  const encoder = new TextEncoder();
  // diagnostics via light queries (bounded)
  const shadow_roots_seen = root.querySelectorAll('*').length ? Array.from(root.querySelectorAll('*')).filter(el => el.shadowRoot).length : 0;
  const iframes = root.querySelectorAll('iframe');
  const iframes_seen = iframes.length;
  let cross_origin_frames_unread = 0;
  for (const f of iframes) { try { if (!f.contentDocument) cross_origin_frames_unread++; } catch (_) { cross_origin_frames_unread++; } }
  while(e) {
    if (++scanned > 65536) { truncated = true; truncation_reason = 'scanned_nodes_exceeded'; break; }
    if (e.matches(selector) && visible(e) && readable(e)) {
      if (items.length >= q.max_items) { omitted_by_limit++; truncated = true; truncation_reason = truncation_reason || 'max_items'; e = walker.nextNode(); continue; }
      const value = raw(e), label = name(e);
      if (value.length > 16384 || label.length > 2000) { omitted_oversized++; truncated = true; truncation_reason = truncation_reason || 'oversized_value'; e = walker.nextNode(); continue; }
      const b = e.getBoundingClientRect();
      const item = {reference:q.token+':'+items.length,tag:e.tagName.toLowerCase(),role:e.getAttribute('role') || ({A:'link',BUTTON:'button',INPUT:'input',TEXTAREA:'textbox',SELECT:'combobox',SUMMARY:'button'}[e.tagName] || 'control'),name:label,text:label,raw_value:value,visible:true,disabled:e.matches(':disabled') || e.getAttribute('aria-disabled')==='true',href:e instanceof HTMLAnchorElement ? e.href : null,expanded:e.getAttribute('aria-expanded'),selected:e.getAttribute('aria-selected') || e.getAttribute('aria-checked'),controls_selector:e.getAttribute('aria-controls')?.split(/\s+/).filter(Boolean).map(id=>'#'+CSS.escape(id)).join(',') || null,datetime:datetimeOf(e),section:sectionOf(e),semantic_ancestor:semanticAncestor(e),dom_path_fingerprint:domPathFingerprint(e),rect_bucket:rectBucket(b),source_frame:'main',source_kind:'light'};
      if (trustedText(e)) item.requires_trusted_events = true;
      const size = encoder.encode(JSON.stringify(item)).length;
      if (bytes + size > q.max_bytes - q.max_text_chars * 6 - 2048) { omitted_by_limit++; truncated = true; truncation_reason = truncation_reason || 'max_bytes'; break; }
      bytes += size; nodes.push(e); items.push(item);
    }
    e = walker.nextNode();
  }
  const texts = document.createTreeWalker(root,NodeFilter.SHOW_TEXT);
  let text='', textNode, textScanned=0;
  while ((textNode=texts.nextNode()) && ++textScanned<=65536 && text.length<=q.max_text_chars) {
    const p=textNode.parentElement;
    if (p && !p.closest(protectedSelector) && visible(p)) {
      const part=textNode.textContent.trim(); if (part) text+=(text?'\\n':'')+part;
    }
  }
  const coverage = truncated ? 'partial: some visible controls omitted due to '+(truncation_reason||'limit') : 'visible light-DOM controls in this scope; includes open shadow roots (counted) and same-origin iframes (counted); excludes cross-origin frames';
  return {nodes,items,raw,name,visible,public:{url:location.href,title:document.title,ready_state:document.readyState,items,text:text.slice(0,q.max_text_chars),text_truncated:!!textNode||text.length>q.max_text_chars,truncated,coverage,scanned_nodes:Math.min(scanned,65536),omitted_by_limit,omitted_oversized,shadow_roots_seen,iframes_seen,cross_origin_frames_unread,truncation_reason,total_candidates:items.length+omitted_by_limit}};
}
