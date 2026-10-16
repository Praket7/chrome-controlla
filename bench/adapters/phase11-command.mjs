#!/usr/bin/env node
import { createHash, randomUUID } from 'node:crypto';
import { spawn } from 'node:child_process';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { setup } from '../runners/phase11-public-readonly-pilot.mjs';

const root = resolve(fileURLToPath(new URL('../..', import.meta.url)));
const adapter = process.argv[2];
const sourceRefs={'controlla-1':'62e6407d0e75b83a1be8d69468476601a6e6da37','controlla-2-pre-stage2':'406b9d9794d6cb59dc75d22c7e6fe7a8554b1e67'}; const source_ref=sourceRefs[adapter] ?? null;
const allowed = new Set(['controlla','controlla-1','controlla-2-pre-stage2','playwright-mcp','chrome-devtools-mcp','agent-browser','browser-use','stagehand']);
if (!allowed.has(adapter)) throw new Error(`unknown adapter ${adapter}`);
const chunks=[]; for await (const chunk of process.stdin) chunks.push(chunk); const input=JSON.parse(Buffer.concat(chunks).toString());
const id=input.task_id; const mode=input.mode; const executionMode=input.execution_mode; const fixture=input.fixture_url; const deadline=input.deadline_ms??60000;
const defaults={adapter,source_ref,task_id:id,mode,execution_mode:executionMode,concurrency:input.concurrency,success:false,independently_verified_success:false,wall_ms:null,model_round_trips:null,model_tokens:null,mcp_calls:null,browser_ops:null,observation_bytes:null,screenshot_bytes:null,retries:0,recovery_events:0,cache_status:null,verification_ms:null,cpu_ms:null,peak_rss_bytes:null,focus_disruptions:0,clipboard_disruptions:0,human_interventions:0,severe_wrong_target_events:0,authority_violations:0,silent_stale_mutations:0,external_controlla_calls:null,model_turns:null,model_visible_observations:null,internal_model_calls:null,browser_cdp_commands:null,host_round_trips:null,native_host_messages_sent:null,native_host_messages_received:null,schema_bytes:null,prompt_input_bytes:null,tool_input_bytes:null,output_bytes:null,unknown_delivery_count:0,fallback_count:0,timeout_count:0,cleanup_completed:null,cleanup_outcome:null};
const output=(fields={})=>{const result={...defaults,...fields}; result.metric_evidence=Object.fromEntries(contract.measurement_metric_fields.map(field=>[field,{provenance:contract.measurement_metric_provenance[field],...(result[field]===null||result[field]===undefined?{missing_reason:'not reported by this adapter command'}:{})}])); console.log(JSON.stringify(result));};

const contract=JSON.parse(await readFile(resolve(root,'bench/runners/adapter-contract.json'),'utf8'));
function evidence(reason){ return Object.fromEntries(contract.measurement_metric_fields.map(field=>[field,{provenance:contract.measurement_metric_provenance[field],...(field==='success'||field==='independently_verified_success'||field==='wall_ms'?{}:{missing_reason:reason})}])); }
if (executionMode !== 'headless') { output({runner_error:'real adapter smoke currently supports headless execution only; foreground/background bindings require a headed Chrome display harness',wall_ms:0}); process.exit(0); }
const codex=process.env.PHASE11_CODEX_PATH ?? '/Users/pcg/.local/bin/codex';
const chrome=process.env.PHASE11_CHROME_PATH ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const binary=process.env.PHASE11_CONTROLLA_BINARY ?? resolve(root,'target/release/controlla-v2');
const parent=await mkdtemp(join(process.env.PHASE11_RUN_DIR ?? tmpdir(),`phase11-${adapter}-`)); const state=join(parent,'state'); await mkdir(state); if(adapter==='stagehand') await mkdir(join(state,'stagehand-tmp'));
let configured=adapter;
if(adapter==='controlla-1'||adapter==='controlla-2-pre-stage2') configured='controlla';
let spec=setup(configured,chrome,binary,state,process.env.PHASE11_BROWSER_USE_EXECUTABLE,process.env.PHASE11_STAGEHAND_SERVER);
let prompt=`Use only the ${configured} browser MCP tools. Open the fixture URL ${fixture}. Perform the benchmark action for task ${id}: interact with the page controls as needed, then click the button whose exact label is "Complete ${id}". Confirm the page shows completion. Use no shell, web search, other browser, or prior knowledge. Return exactly one JSON object: {"completed":true,"task_id":"${id}"}.`;
if(configured==='controlla') prompt=`Use only the chrome-controlla MCP browser tool. Make exactly one call with {"action":"task","mode":"headless","release_after_task":true,"task":{"kind":"research","urls":["${fixture}"],"selector":"#complete","goal":"Read the live benchmark fixture and confirm the completion control for task ${id}."}}. Return exactly {"completed":true,"task_id":"${id}"} after the successful browser result. Do not use any other tool.`;
const config=[...spec.config];
const args=['exec','--ephemeral','--json','--ignore-user-config','--approve-for-me','--skip-git-repo-check','-m',process.env.PHASE11_MODEL??'gpt-6-luna','-C',parent];
for(let i=0;i<config.length;i++) args.push(config[i]); args.push(prompt);
const started=process.hrtime.bigint(); let stdout=''; let stderr=''; let buffer=''; const events=[]; let final='';
try {
 await new Promise(done=>{const child=spawn(codex,args,{cwd:parent,env:process.env,stdio:['ignore','pipe','pipe'],detached:true}); const timer=setTimeout(()=>{try{process.kill(-child.pid,'SIGKILL')}catch{child.kill('SIGKILL')}},deadline); child.stdout.setEncoding('utf8'); child.stderr.setEncoding('utf8'); child.stdout.on('data',c=>{stdout+=c;buffer+=c;let n;while((n=buffer.indexOf('\n'))>=0){const line=buffer.slice(0,n);buffer=buffer.slice(n+1);try{const e=JSON.parse(line);events.push(e);if(e.type==='item.completed'&&e.item?.type==='agent_message')final=e.item.text??'';}catch{}}}); child.stderr.on('data',c=>stderr+=c); child.on('close',code=>{clearTimeout(timer);done(code);});});
 const calls=events.filter(e=>e.type==='item.completed'&&e.item?.type==='mcp_tool_call'); const success=events.some(e=>e.type==='item.completed'&&e.item?.type==='mcp_tool_call'&&e.item.status==='completed'&&!e.item.error)&&/completed|success/i.test(final);
 const elapsed=Number(process.hrtime.bigint()-started)/1e6;
 output({success,wall_ms:elapsed,mcp_calls:calls.length,browser_ops:calls.length,output_bytes:Buffer.byteLength(stdout),prompt_input_bytes:Buffer.byteLength(prompt),tool_input_bytes:calls.reduce((n,e)=>n+Buffer.byteLength(JSON.stringify(e.item.arguments??{})),0),observation_bytes:calls.reduce((n,e)=>n+Buffer.byteLength(JSON.stringify(e.item.result??{})),0),cleanup_completed:configured==='controlla'?calls.some(e=>JSON.stringify(e.item.result??{}).includes('private_session_released')):null,cleanup_outcome:configured==='controlla'?'private_session_released_or_adapter_cleanup':null,runner_error:success?null:((stderr.trim()||'adapter did not produce a successful completion').slice(0,120))});
} finally { await rm(parent,{recursive:true,force:true}); }
