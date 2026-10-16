#!/usr/bin/env node
import { createServer } from 'node:http';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../../..');
const statePath = resolve(process.env.PHASE11_STATE_FILE ?? resolve(root, '.phase11-state.json'));
const port = Number(process.env.PHASE11_PORT ?? process.argv.find(v => v.startsWith('--port='))?.slice(7) ?? 0);
const tasks = new Map();
const classes = new Map([
  ['forms', ['registration-form','rich-editor-1000-char-fastkeys','slow-media-early-form']],
  ['extraction', ['structured-extraction','50k-node-targeted-control','newest-ranking-50-candidates','on-demand-frame-escalation']],
  ['multitab', ['two-tab-compare','eight-tab-pressure']],
  ['spa', ['spa-document-change','stale-after-navigation']],
  ['visual_canvas', ['canvas-targeted-probe','seven-slide-capture-manifest']],
  ['virtual_list', ['virtual-list-find','virtual-list-1000-rows']],
  ['upload', ['guarded-upload','artifact-register-4mb','artifact-register-10mb']],
  ['repeated_workflow', ['repeat-qualified-flow','five-field-single-workflow']],
  ['interference', ['human-edit-race','react-rerender-target']],
  ['long_workflow', ['bounded-long-flow']],
  ['concurrent_tabs', ['eight-tab-pressure']],
  ['cross_origin_oopif', ['cross-site-oopif-target']],
  ['download_upload', ['download-register-upload']],
]);

function taskState(id) { return tasks.get(id) ?? { task_id: id, events: [], completed: false, reset_at: Date.now() }; }
async function persist() { await mkdir(dirname(statePath), { recursive: true }); await writeFile(statePath, JSON.stringify(Object.fromEntries(tasks), null, 2)); }
async function load() { try { const raw = JSON.parse(await readFile(statePath, 'utf8')); for (const [k,v] of Object.entries(raw)) tasks.set(k,v); } catch {} }
function json(res, status, value) { const body = JSON.stringify(value); res.writeHead(status, {'content-type':'application/json','content-length':Buffer.byteLength(body)}); res.end(body); }
function page(taskId, fixture, scenario) {
  const title = `Controlla Phase 11 · ${taskId}`;
  const rows = Array.from({length: fixture === 'records-table' && scenario?.includes('50k') ? 50000 : 50}, (_, i) => `<tr><td>record-${i + 1}</td><td>${i % 2 ? 'older' : 'current'}</td></tr>`).join('');
  const action = `Complete ${taskId}`;
  return `<!doctype html><html><head><meta charset="utf-8"><title>${title}</title><style>body{font:16px system-ui;margin:32px}#scroll{height:220px;overflow:auto;border:1px solid #888}canvas{border:1px solid #888;width:500px;height:140px}label,button,input{margin:8px;padding:8px}iframe{width:400px;height:100px}</style></head><body>
<h1>${title}</h1><p data-testid="fixture-description">Controlled benchmark fixture for ${fixture}.</p>
<form id="registration-form"><label>Name <input id="name" name="name"></label><label>Notes <textarea id="notes"></textarea></label><button type="submit">Save form</button></form>
<div id="records"><table><tbody>${rows}</tbody></table><button id="extract">Read requested record</button></div>
<div id="tabs"><a href="/fixture/${fixture}?task=${encodeURIComponent(taskId)}&tab=one">Tab one</a> <a href="/fixture/${fixture}?task=${encodeURIComponent(taskId)}&tab=two">Tab two</a></div>
<div id="spa"><button id="route">Navigate SPA route</button><output id="route-output"></output></div>
<canvas id="canvas" width="500" height="140"></canvas><button id="canvas-action">Probe canvas target</button>
<div id="scroll">${Array.from({length:1000},(_,i)=>`<div class="row">virtual-row-${i+1}</div>`).join('')}</div>
<input id="upload" type="file"><button id="upload-action">Register upload</button>
<div id="editor" contenteditable="true">Editable benchmark content</div>
<iframe title="child frame" src="/frame?task=${encodeURIComponent(taskId)}"></iframe>
<button id="complete" data-task="${taskId}">${action}</button><output id="status" aria-live="polite"></output>
<script>
const task=${JSON.stringify(taskId)}; const fixture=${JSON.stringify(fixture)};
async function event(kind, extra={}){document.querySelector('#status').textContent=kind; await fetch('/__event',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({task_id:task,kind,fixture,...extra})});}
document.querySelector('#registration-form').addEventListener('submit',e=>{e.preventDefault(); event('form_saved',{name:document.querySelector('#name').value,notes:document.querySelector('#notes').value});});
document.querySelector('#extract').onclick=()=>event('extracted',{count:1});
document.querySelector('#route').onclick=()=>{history.pushState({},'',location.pathname+'?task='+encodeURIComponent(task)+'&route=next'); document.querySelector('#route-output').textContent='route=next'; event('spa_navigated');};
document.querySelector('#canvas-action').onclick=()=>event('canvas_probed');
document.querySelector('#upload-action').onclick=()=>{const f=document.querySelector('#upload').files[0]; event('upload_registered',f?{filename:f.name,size:f.size,sha256:'client-unavailable'}:{filename:null,size:0});};
document.querySelector('#complete').onclick=()=>event('completed',{completed:true});
window.addEventListener('scroll',()=>event('scrolled',{scrollY:scrollY}));
</script></body></html>`;
}

await load();
const server = createServer(async (req,res) => {
  const url = new URL(req.url, `http://${req.headers.host}`);
  if (req.method === 'GET' && url.pathname === '/__health') return json(res,200,{ok:true,state_file:statePath});
  if (req.method === 'POST' && (url.pathname === '/__reset' || url.pathname === '/__event')) {
    let body=''; for await (const chunk of req) body += chunk; let input={}; try { input=JSON.parse(body||'{}'); } catch { return json(res,400,{ok:false,error:'invalid json'}); }
    const id=input.task_id; if (!id) return json(res,400,{ok:false,error:'task_id required'});
    if (url.pathname === '/__reset') tasks.set(id,{task_id:id,events:[],completed:false,reset_at:Date.now()});
    else { const current=taskState(id); current.events.push({at:Date.now(),kind:input.kind,...input}); if (input.completed===true) current.completed=true; tasks.set(id,current); }
    await persist(); return json(res,200,{ok:true,task_id:id});
  }
  if (req.method === 'GET' && url.pathname === '/__state') return json(res,200,taskState(url.searchParams.get('task_id') ?? ''));
  if (req.method === 'GET' && url.pathname === '/frame') return res.end(`<html><body><p>Independent child frame for ${url.searchParams.get('task') ?? ''}</p></body></html>`);
  if (req.method === 'GET' && url.pathname.startsWith('/fixture/')) return res.end(page(url.searchParams.get('task') ?? url.pathname.split('/').pop(), url.pathname.split('/').pop(), url.searchParams.get('scenario') ?? ''));
  res.writeHead(404); res.end('not found');
});
server.listen(port,'127.0.0.1',()=>{const address=server.address(); console.log(JSON.stringify({ok:true,base_url:`http://127.0.0.1:${address.port}`}));});
