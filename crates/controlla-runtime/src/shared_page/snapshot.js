function(q) {
  const roots = document.querySelectorAll(q.selector);
  if (roots.length !== 1) throw Error('snapshot scope must match exactly one element');
  let root = roots[0];
  if (q.goal?.within) {
    const {tag,id} = q.goal.within;
    if (!/^[a-z][a-z0-9-]{0,31}$/.test(tag) || typeof id !== 'string' || !id || id.length > 256) throw Error('invalid semantic ancestor scope');
    const candidates = Array.from(root.querySelectorAll(tag)).filter(e => e.id === id || e.getAttribute('data-id') === id);
    if (candidates.length !== 1) throw Error('semantic ancestor scope is not unique');
    root = candidates[0];
  }
  const nodes = [], items = [];
  const protectedSelector = '[data-masked],[data-requires-trusted],input[type="password"],input[type="file"],input[type="hidden"]';
  const isCredentialInput = e => e instanceof HTMLInputElement && e.type === 'password';
  const composedClosest = (e, selector) => {
    for (let n=e, depth=0; n && depth<16; n=n.parentElement || n.getRootNode()?.host, depth++) {
      if (n.matches?.(selector)) return n;
    }
    return null;
  };
  const trustedText = e => e?.hasAttribute('data-requires-trusted') && (e instanceof HTMLTextAreaElement || e instanceof HTMLInputElement && ['text','search','email','url','tel'].includes(e.type));
  const richEditor = e => e instanceof HTMLElement && !(e instanceof HTMLInputElement || e instanceof HTMLTextAreaElement) && e.getAttribute('aria-readonly') !== 'true' && e.getAttribute('contenteditable')?.toLowerCase() !== 'false' && (e.isContentEditable || e.getAttribute('role') === 'textbox');
  const editableText = e => { const text=e.innerText ?? e.textContent ?? ''; return e.textContent === '' && /^\s*$/.test(text) ? '' : text.replace(/\r\n/g,'\n').replace(/\u00a0/g,' '); };
  const readable = e => {
    if (!e || composedClosest(e, '[data-masked],input[type="file"],input[type="hidden"]') || e.querySelector(protectedSelector)) return false;
    const trusted = composedClosest(e, '[data-requires-trusted]');
    return !trusted || trusted === e && trustedText(e);
  };
  const presentation = e => {
    const s = e.ownerDocument.defaultView.getComputedStyle(e), b = e.getBoundingClientRect();
    const present = e.isConnected;
    const rendered = present && b.width > 0 && b.height > 0 && s.visibility === 'visible' && s.display !== 'none' && Number.parseFloat(s.opacity || '1') > 0 && !composedClosest(e, '[hidden],[inert]');
    const view = e.ownerDocument.defaultView;
    const inViewport = rendered && b.right > 0 && b.bottom > 0 && b.left < view.innerWidth && b.top < view.innerHeight;
    const disabled = e.matches(':disabled') || e.getAttribute('aria-disabled') === 'true';
    const pointerEventsNone = s.pointerEvents === 'none';
    const actionability = !rendered ? 'not_rendered' : disabled ? 'disabled' : pointerEventsNone ? 'pointer_blocked' : inViewport ? 'candidate' : 'offscreen';
    return {present,rendered,in_viewport:!!inViewport,disabled,pointer_events_none:pointerEventsNone,actionability};
  };
  const raw = e => isCredentialInput(e) ? null : /^(INPUT|TEXTAREA|SELECT)$/.test(e.tagName) ? e.value : richEditor(e) ? editableText(e) : (e.innerText ?? '');
  const name = e => {
    const labelled = (e.getAttribute('aria-labelledby') || '').split(/\s+/).filter(Boolean).map(id => { const label=e.getRootNode().getElementById?.(id) || e.ownerDocument.getElementById(id); return readable(label) ? label.innerText : ''; }).join(' ').trim();
    if (isCredentialInput(e)) {
      const labels = Array.from(e.labels || []).filter(label => !composedClosest(label, '[data-masked]')).map(label => label.innerText).join(' ').trim();
      return (labelled || e.getAttribute('aria-label') || labels || e.getAttribute('title') || '').replace(/\s+/g,' ').trim();
    }
    return (labelled || e.getAttribute('aria-label') || e.innerText || e.getAttribute('alt') || Array.from(e.labels || []).filter(readable).map(l => l.innerText).join(' ') || e.getAttribute('title') || '').replace(/\s+/g,' ').trim();
  };
  const datetimeOf = e => {
    if (e.getAttribute && e.getAttribute('datetime')) return e.getAttribute('datetime');
    const ancestor = e.closest('article,section,[data-section],li');
    if (ancestor) {
      const t = ancestor.querySelector('time[datetime]');
      if (t) return t.getAttribute('datetime');
      const meta = ancestor.querySelector('meta[itemprop="datePublished"][content],meta[property="article:published_time"][content]');
      if (meta) return meta.getAttribute('content');
      const d = ancestor.getAttribute('data-datetime') || ancestor.getAttribute('data-timestamp') || ancestor.getAttribute('data-date') || ancestor.getAttribute('data-published');
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
  const selector = 'a[href],button,input,textarea,select,summary,[contenteditable]:not([contenteditable="false"]),[role="textbox"],[role="button"],[role="link"],[role="menuitem"],[role="checkbox"],[role="radio"],[role="tab"],[role="combobox"],[role="option"]';
  let scanned = 0, truncated = false, bytes = 0, omitted_by_limit = 0, omitted_oversized = 0, early_stopped = false, goal_satisfied = false, goal_score = 0;
  let truncation_reason = null;
  const goalQuery = q.goal?.query?.normalize('NFKC').toLocaleLowerCase().replace(/\s+/g,' ').trim() || '';
  const goalRoles = Array.isArray(q.goal?.roles) ? new Set(q.goal.roles) : null;
  const maxScanNodes = Number.isSafeInteger(q.max_scan_nodes) ? Math.max(1, Math.min(65536, q.max_scan_nodes)) : 65536;
  const encoder = new TextEncoder();
  let shadow_roots_seen = 0, iframes_seen = 0, cross_origin_frames_unread = 0, same_origin_frames_read = 0, same_origin_frames_pending = 0, blank_frames_seen = 0;
  const scopes = [{root, frame:'main', kind:'light', depth:0}];
  for (let scopeIndex = 0; scopeIndex < scopes.length; scopeIndex++) {
    const scope = scopes[scopeIndex], walker = scope.root.ownerDocument.createTreeWalker(scope.root, NodeFilter.SHOW_ELEMENT);
    let e = scope.root.nodeType === Node.ELEMENT_NODE ? scope.root : walker.nextNode();
    while (e) {
    if (scanned >= maxScanNodes) { truncated = true; truncation_reason = 'scanned_nodes_exceeded'; break; }
    scanned++;
    if (e.shadowRoot) {
      shadow_roots_seen++;
      if (scopes.length < 32 && scope.depth < 8) scopes.push({root:e.shadowRoot,frame:scope.frame,kind:'shadow',depth:scope.depth+1});
      else { truncated = true; truncation_reason = truncation_reason || 'nested_scope_limit'; }
    }
    if (e.tagName === 'IFRAME') {
      iframes_seen++;
      try {
        const doc = e.contentDocument;
        if (doc?.body && doc.location.href !== 'about:blank' && scopes.length < 32 && scope.depth < 8) { scopes.push({root:doc.body,frame:scope.frame+'/iframe['+iframes_seen+']',kind:'same_origin_frame',depth:scope.depth+1}); same_origin_frames_read++; }
        else if (doc?.body && doc.location.href === 'about:blank') {
          const src = e.getAttribute('src');
          if (src && src !== 'about:blank' && new URL(src, location.href).origin === location.origin) same_origin_frames_pending++;
          else if (src && src !== 'about:blank') cross_origin_frames_unread++;
          else if (doc.body.children.length && scopes.length < 32 && scope.depth < 8) { scopes.push({root:doc.body,frame:scope.frame+'/iframe['+iframes_seen+']',kind:'same_origin_blank_frame',depth:scope.depth+1}); same_origin_frames_read++; }
          else blank_frames_seen++;
        }
        else if (!doc) cross_origin_frames_unread++;
        else { truncated = true; truncation_reason = truncation_reason || 'nested_scope_limit'; }
      } catch (_) { cross_origin_frames_unread++; }
    }
    if (e.matches(selector) && (!q.href_hint || e.tagName === 'A' && e.href.includes(q.href_hint)) && readable(e)) {
      const state = presentation(e);
      if (!state.rendered) { e = walker.nextNode(); continue; }
      if (items.length >= q.max_items && !q.goal) { omitted_by_limit++; truncated = true; truncation_reason = truncation_reason || 'max_items'; e = walker.nextNode(); continue; }
      const credential = isCredentialInput(e), value = raw(e), label = name(e);
      if (typeof value === 'string' && value.length > 16384 || label.length > 2000) { omitted_oversized++; truncated = true; truncation_reason = truncation_reason || 'oversized_value'; e = walker.nextNode(); continue; }
      const b = e.getBoundingClientRect();
      const datetime = datetimeOf(e);
      const parsedTime = datetime && /^\d{4}-\d{2}-\d{2}(?:T\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?(?:Z|[+-]\d{2}:\d{2}))?$/.test(datetime) ? Date.parse(datetime) : NaN;
      const dateOnly = typeof datetime === 'string' && /^(\d{4})-(\d{2})-(\d{2})$/.exec(datetime);
      // Date-only publication dates have no timezone; UTC-12 can extend 12 hours into the following UTC day.
      const freshnessUpperBound = dateOnly ? Date.UTC(Number(dateOnly[1]), Number(dateOnly[2])-1, Number(dateOnly[3])+1, 12)-1 : null;
      const exactInstant = typeof datetime === 'string' && /T\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?(?:Z|[+-]\d{2}:\d{2})$/.test(datetime);
      const rich = richEditor(e), editable = rich || e instanceof HTMLTextAreaElement || e instanceof HTMLInputElement && ['text','search','email','url','tel'].includes(e.type);
      const item = {reference:q.token+':'+items.length,tag:e.tagName.toLowerCase(),role:e.getAttribute('role') || (rich ? 'textbox' : ({A:'link',BUTTON:'button',INPUT:'input',TEXTAREA:'textbox',SELECT:'combobox',SUMMARY:'button'}[e.tagName] || 'control')),name:label,text:label,raw_value:value,...state,editable_kind:rich?'rich_text':editable?'plain_text':null,credential_input:credential,requires_user_input:credential,multiline:rich||e.tagName==='TEXTAREA'||e.getAttribute('aria-multiline')==='true',href:e.tagName === 'A' ? e.href : null,expanded:e.getAttribute('aria-expanded'),selected:e.getAttribute('aria-selected') || e.getAttribute('aria-checked'),controls_selector:e.getAttribute('aria-controls')?.split(/\s+/).filter(Boolean).map(id=>'#'+CSS.escape(id)).join(',') || null,datetime,datetime_utc_ms:exactInstant && Number.isFinite(parsedTime)?parsedTime:null,datetime_precision:exactInstant?'instant':dateOnly?'date':null,freshness_upper_bound_ms:Number.isFinite(freshnessUpperBound)?freshnessUpperBound:null,section:sectionOf(e),semantic_ancestor:semanticAncestor(e),dom_path_fingerprint:domPathFingerprint(e),rect_bucket:rectBucket(b),source_frame:scope.frame,source_frame_revision:String(e.ownerDocument.defaultView.performance.timeOrigin),source_frame_url:e.ownerDocument.location.href,source_kind:scope.kind};
      let queryScore = 0;
      if (q.goal) {
        const normalize = text => text.normalize('NFKC').toLocaleLowerCase().replace(/\s+/g,' ').trim();
        const candidateText = normalize(label+' '+(typeof value === 'string' ? value : ''));
        queryScore = !goalQuery ? 0 : normalize(label) === goalQuery || typeof value === 'string' && normalize(value) === goalQuery ? 100 : candidateText.includes(goalQuery) ? 60 : 0;
        const roleScore = !goalRoles ? 0 : goalRoles.has(item.role) ? 40 : -1000;
        const hrefScore = !q.goal.href_hint ? 0 : item.href?.includes(q.goal.href_hint) ? 30 : -1000;
        if (goalQuery && queryScore === 0 || roleScore < 0 || hrefScore < 0) { e = walker.nextNode(); continue; }
        goal_score = queryScore + roleScore + hrefScore;
        item.goal_score = goal_score;
        if (!goalQuery || queryScore === 100) goal_satisfied = true;
      }
      if (q.href_hint && e.tagName === 'A') {
        const context = e.closest('article,li,[data-card]') || e.parentElement;
        item.context_text = readable(context) ? (context.innerText || '').replace(/\s+/g,' ').slice(0,320) : null;
        const group = e.closest('section,[role="feed"],[role="list"]');
        const heading = group?.querySelector('h1,h2,h3,[role="heading"]');
        item.group_heading = readable(heading) ? heading.innerText.replace(/\s+/g,' ').slice(0,120) : null;
      }
      if (trustedText(e)) item.requires_trusted_events = true;
      const size = encoder.encode(JSON.stringify(item)).length;
      if (q.goal && items.length >= q.max_items) {
        let lowest = 0;
        for (let i = 1; i < items.length; i++) if ((items[i].goal_score || 0) < (items[lowest].goal_score || 0)) lowest = i;
        if ((item.goal_score || 0) > (items[lowest].goal_score || 0)) {
          const previousSize = encoder.encode(JSON.stringify(items[lowest])).length;
          if (bytes - previousSize + size > q.max_bytes - q.max_text_chars * 6 - 2048) { omitted_by_limit++; truncated = true; truncation_reason = truncation_reason || 'max_bytes'; break; }
          bytes += size - previousSize; items[lowest] = item; nodes[lowest] = e;
        }
        omitted_by_limit++; truncated = true; truncation_reason = truncation_reason || 'max_items';
      } else {
        if (bytes + size > q.max_bytes - q.max_text_chars * 6 - 2048) { omitted_by_limit++; truncated = true; truncation_reason = truncation_reason || 'max_bytes'; break; }
        bytes += size; nodes.push(e); items.push(item);
      }
      const exactTargetScore = (goalQuery ? 100 : 0) + (goalRoles ? 40 : 0) + (q.goal?.href_hint ? 30 : 0);
      // Exact semantic criteria reach the global score ceiling; viewport is actionability metadata, not a ranking bonus.
      if (q.goal && !q.goal.exhaustive && q.max_items === 1 && goal_score >= exactTargetScore) { early_stopped = true; truncated = true; truncation_reason = 'ranked_top_one'; break; }
    }
    e = walker.nextNode();
    }
    if (early_stopped || truncation_reason === 'scanned_nodes_exceeded' || truncation_reason === 'max_bytes') break;
  }
  if (q.goal && items.length > 1) {
    const ranked = items.map((item,index)=>({item,node:nodes[index],index})).sort((a,b)=>(b.item.goal_score||0)-(a.item.goal_score||0)||a.index-b.index);
    items.splice(0,items.length,...ranked.map(entry=>entry.item));
    nodes.splice(0,nodes.length,...ranked.map(entry=>entry.node));
    for (let i=0;i<items.length;i++) items[i].reference=q.token+':'+i;
  }
  let text='', textScanned=0, text_truncated=false;
  const maxTextNodes = q.max_text_chars > 0 ? (q.goal ? Math.min(maxScanNodes, 4096) : 65536) : 0;
  const textScopes = scopes.slice();
  if (q.goal && maxTextNodes) {
    for (const frame of root.querySelectorAll('iframe')) {
      try {
        const doc = frame.contentDocument;
        if (doc?.body && doc.location.href !== 'about:blank' && textScopes.length < 32 && !textScopes.some(scope => scope.root === doc.body)) {
          textScopes.push({root:doc.body,frame:'main/iframe[text]',kind:'same_origin_frame',depth:1});
        } else if (doc?.body && doc.location.href === 'about:blank' && doc.body.children.length && textScopes.length < 32 && !textScopes.some(scope => scope.root === doc.body)) {
          textScopes.push({root:doc.body,frame:'main/iframe[text]',kind:'same_origin_blank_frame',depth:1});
        }
      } catch (_) {}
    }
  }
  const visitedTextNodes = new Set();
  for (const scope of textScopes) {
    if (!maxTextNodes) break;
    const texts=scope.root.ownerDocument.createTreeWalker(scope.root,NodeFilter.SHOW_TEXT);
    let textNode;
    while ((textNode=texts.nextNode())) {
      if (visitedTextNodes.has(textNode)) continue;
      visitedTextNodes.add(textNode);
      if (textScanned >= maxTextNodes) { text_truncated=true; break; }
      textScanned++;
      const p=textNode.parentElement;
      if (p && !composedClosest(p,protectedSelector) && presentation(p).rendered) {
        const part=textNode.textContent.trim();
        if (part) {
          const separator=text?'\n':'';
          const remaining=q.max_text_chars+1-text.length;
          text+=separator+part.slice(0,Math.max(0,remaining-separator.length));
          if (separator.length+part.length>remaining || text.length>q.max_text_chars) { text_truncated=true; break; }
        }
      }
    }
    if (text_truncated) break;
  }
  if (q.goal?.exhaustive) goal_satisfied = items.length > 0;
  const controls_complete = !truncated && omitted_by_limit === 0 && omitted_oversized === 0;
  const frames_complete = cross_origin_frames_unread === 0 && same_origin_frames_pending === 0;
  const text_requested = q.max_text_chars > 0;
  const text_complete = text_requested && !text_truncated && frames_complete;
  const complete = controls_complete && (!text_requested || text_complete) && frames_complete && !early_stopped;
  const incomplete_reasons = [];
  if (text_truncated) incomplete_reasons.push('text_truncated');
  if (early_stopped) incomplete_reasons.push(truncation_reason || 'targeted_early_stop');
  if (omitted_by_limit || truncation_reason === 'max_items' || truncation_reason === 'max_bytes') incomplete_reasons.push('item_or_byte_limit');
  if (omitted_oversized) incomplete_reasons.push('oversized_controls');
  if (truncation_reason === 'scanned_nodes_exceeded') incomplete_reasons.push('scanned_node_limit');
  if (truncation_reason === 'nested_scope_limit') incomplete_reasons.push('nested_scope_limit');
  if (cross_origin_frames_unread) incomplete_reasons.push('cross_origin_frames_unread');
  if (same_origin_frames_pending) incomplete_reasons.push('same_origin_frames_pending');
  const coverage = truncated || cross_origin_frames_unread || same_origin_frames_pending || text_truncated ? 'partial: '+(truncation_reason || (text_truncated ? 'text_truncated' : cross_origin_frames_unread ? 'cross_origin_frames_unread' : 'same_origin_frames_pending')) : (q.href_hint ? 'visible links matching href hint in reachable DOM and frames' : 'visible controls in light DOM, open shadow roots, and loaded same-origin frames in this scope');
  // Cross-origin frames cannot be inspected; they do not truncate the reachable first-party link scope.
  const first_party_complete = !!q.href_hint && !truncated && !same_origin_frames_pending && items.every(item => { try { return new URL(item.href).origin === location.origin; } catch (_) { return false; } });
  const scope_complete = q.goal ? !truncated && !early_stopped && frames_complete : q.href_hint ? first_party_complete : complete;
  const target_uniqueness = scope_complete ? 'scope_complete' : 'unverified';
  for (const item of items) item.target_uniqueness = target_uniqueness;
  return {nodes,items,raw,name,presentation,visible:e=>presentation(e).rendered,public:{url:location.href,title:document.title,ready_state:document.readyState,items,text:text.slice(0,q.max_text_chars),text_requested,text_truncated,truncated:!complete,complete,global_complete:complete,completeness:{controls:controls_complete?'complete':'partial',text:text_complete?'complete':'partial',frames:frames_complete?'complete':'partial'},incomplete_reasons,goal_satisfied,goal_score:goal_satisfied?goal_score:null,scope_complete,coverage,coverage_scope:q.goal?'ranked_target':q.href_hint ? 'matching_first_party_links' : 'all_reachable_controls',first_party_complete,scanned_nodes:Math.min(scanned,maxScanNodes),scanned_text_nodes:textScanned,search_budget_exhausted:truncation_reason==='scanned_nodes_exceeded',omitted_by_limit,omitted_oversized,shadow_roots_seen,iframes_seen,same_origin_frames_read,same_origin_frames_pending,blank_frames_seen,cross_origin_frames_unread,truncation_reason:truncation_reason || (text_truncated ? 'text_truncated' : cross_origin_frames_unread ? 'cross_origin_frames_unread' : same_origin_frames_pending ? 'same_origin_frames_pending' : null),total_candidates:items.length+omitted_by_limit}};
}
