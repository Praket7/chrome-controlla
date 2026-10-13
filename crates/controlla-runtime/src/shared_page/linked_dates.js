async function(urls, expectedDocument) {
  if (expectedDocument?.url !== location.href || expectedDocument?.revision !== String(performance.timeOrigin)) return {snapshot_matches:false,dates:[]};
  if (!Array.isArray(urls) || urls.length > 12) return [];
  const normalized = (value, base = location.href) => {
    const raw = typeof value === 'string' ? value : value?.['@id'];
    if (typeof raw !== 'string') return null;
    try { const url = new URL(raw, base); return url.origin === location.origin ? url.href : null; }
    catch (_) { return null; }
  };
  const listed = new Map();
  for (const script of [...document.querySelectorAll('script[type="application/ld+json"]')].slice(0,16)) {
    if ((script.textContent || '').length > 1000000) continue;
    let data;
    try { data = JSON.parse(script.textContent); } catch (_) { continue; }
    const queue = [data];
    for (let i=0; i<queue.length && i<128; i++) {
      const node = queue[i];
      if (!node || typeof node !== 'object') continue;
      const kind = Array.isArray(node['@type']) ? node['@type'] : [node['@type']];
      const date = node.datePublished;
      if (kind.some(type => typeof type === 'string' && /(?:News)?Article$/.test(type)) && typeof date === 'string' && /^\d{4}-\d{2}-\d{2}T.+(?:Z|[+-]\d{2}:\d{2})$/.test(date)) {
        const time = Date.parse(date);
        if (Number.isFinite(time)) for (const value of [node.url,node.mainEntityOfPage,node['@id']]) {
          const href = normalized(value);
          if (!href) continue;
          const prior = listed.get(href);
          listed.set(href, prior && prior.datetime_utc_ms === time ? prior : listed.has(href) ? null : {href,datetime:date,datetime_utc_ms:time,source:'listing_json_ld'});
        }
      }
      for (const child of Object.values(node)) if (child && typeof child === 'object') queue.push(...(Array.isArray(child) ? child : [child]));
    }
  }
  const dates = await Promise.all(urls.map(async href => {
    try {
      const url = new URL(href, location.href);
      if (url.origin !== location.origin || !/^https?:$/.test(url.protocol)) return {href, error:'cross_origin'};
      if (listed.get(url.href)) return listed.get(url.href);
      const controller = new AbortController();
      const timer = setTimeout(() => controller.abort(), 4000);
      try {
        const response = await fetch(url.href, {credentials:'same-origin', signal:controller.signal});
        if (!response.ok || response.url !== url.href) return {href, error:'http_or_redirect'};
        if (Number(response.headers.get('content-length') || 0) > 1000000) return {href, error:'oversized'};
        const reader = response.body.getReader(), decoder = new TextDecoder();
        let html = '', bytes = 0;
        for (;;) {
          const {done, value} = await reader.read();
          if (done) break;
          bytes += value.byteLength;
          if (bytes > 1000000) return {href, error:'oversized'};
          html += decoder.decode(value, {stream:true});
        }
        html += decoder.decode();
        const scripts = [...html.matchAll(/<script\b[^>]*type=["']application\/ld\+json["'][^>]*>([\s\S]*?)<\/script>/gi)].slice(0,16);
        for (const script of scripts) {
          let data;
          try { data = JSON.parse(script[1]); } catch (_) { continue; }
          const queue = [data];
          for (let i=0; i<queue.length && i<128; i++) {
            const node = queue[i];
            if (!node || typeof node !== 'object') continue;
            const kind = Array.isArray(node['@type']) ? node['@type'] : [node['@type']];
            const date = node.datePublished;
            if (kind.some(type => typeof type === 'string' && /(?:News)?Article$/.test(type)) && typeof date === 'string' && /^\d{4}-\d{2}-\d{2}T.+(?:Z|[+-]\d{2}:\d{2})$/.test(date)) {
              const time = Date.parse(date);
              if (Number.isFinite(time) && [node.url,node.mainEntityOfPage,node['@id']].some(value => normalized(value, response.url) === url.href)) return {href:url.href, datetime:date, datetime_utc_ms:time};
            }
            for (const child of Object.values(node)) if (child && typeof child === 'object') queue.push(...(Array.isArray(child) ? child : [child]));
          }
        }
        return {href, error:'date_missing'};
      } finally { clearTimeout(timer); }
    } catch (_) { return {href, error:'fetch_failed'}; }
  }));
  return {snapshot_matches:true,dates};
}
