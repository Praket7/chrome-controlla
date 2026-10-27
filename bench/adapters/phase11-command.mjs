#!/usr/bin/env node
import { readFile, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { tmpdir } from 'node:os';
import { basename, dirname, isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { setup } from '../runners/phase11-public-readonly-pilot.mjs';

const root = resolve(fileURLToPath(new URL('../..', import.meta.url)));
const commandStarted = process.hrtime.bigint();
const adapter = process.argv[2];
const sourceRefs={'controlla-1':'62e6407d0e75b83a1be8d69468476601a6e6da37','controlla-2-pre-stage2':'406b9d9794d6cb59dc75d22c7e6fe7a8554b1e67'}; const source_ref=sourceRefs[adapter] ?? null;
const allowed = new Set(['controlla','controlla-1','controlla-2-pre-stage2','playwright-mcp','chrome-devtools-mcp','agent-browser','browser-use','stagehand']);
if (!allowed.has(adapter)) throw new Error(`unknown adapter ${adapter}`);
const chunks=[]; let inputBytes=0; for await (const chunk of process.stdin) { inputBytes+=chunk.length; if(inputBytes>65536) throw new Error('request too large'); chunks.push(chunk); } const input=JSON.parse(Buffer.concat(chunks).toString());
const id=input.task_id; const mode=input.mode; const executionMode=input.execution_mode;
const defaults={adapter,source_ref,task_id:id,mode,execution_mode:executionMode,concurrency:input.concurrency,success:false,independently_verified_success:false,wall_ms:null,model_round_trips:null,model_tokens:null,mcp_calls:null,browser_ops:null,observation_bytes:null,screenshot_bytes:null,retries:null,recovery_events:null,cache_status:null,verification_ms:null,cpu_ms:null,peak_rss_bytes:null,focus_disruptions:null,clipboard_disruptions:null,human_interventions:null,severe_wrong_target_events:null,authority_violations:null,silent_stale_mutations:null,external_controlla_calls:null,model_turns:null,model_visible_observations:null,internal_model_calls:null,browser_cdp_commands:null,host_round_trips:null,native_host_messages_sent:null,native_host_messages_received:null,schema_bytes:null,prompt_input_bytes:null,tool_input_bytes:null,output_bytes:null,unknown_delivery_count:null,fallback_count:null,timeout_count:null,cleanup_completed:null,cleanup_outcome:null};
const output=(fields={})=>{const result={...defaults,...fields}; if(result.wall_ms===null) result.wall_ms=Number(process.hrtime.bigint()-commandStarted)/1e6; result.metric_evidence=Object.fromEntries(contract.measurement_metric_fields.map(field=>[field,{provenance:contract.measurement_metric_provenance[field],...(result[field]===null||result[field]===undefined?{missing_reason:field==='success'||field==='independently_verified_success'?'unsupported_task_or_uninstrumented_adapter':'not reported by this adapter command'}:{})}])); console.log(JSON.stringify(result));};

const contract=JSON.parse(await readFile(resolve(root,'bench/runners/adapter-contract.json'),'utf8'));
if (executionMode !== 'headless') { output({runner_error:'real adapter smoke currently supports headless execution only; foreground/background bindings require a headed Chrome display harness'}); process.exit(0); }
const fixtureKind = id === 'form-guarded-fill' && input.task_class === 'forms' ? 'form' : id === 'guarded-upload' && input.task_class === 'upload' ? 'upload' : null;
if (['controlla','playwright-mcp'].includes(adapter) && fixtureKind && !input.scenario && mode === 'cold_browser' && input.concurrency === 1) {
  const base = new URL(process.env.PHASE11_FIXTURE_BASE_URL ?? '');
  const url = new URL(input.fixture_url);
  const bytes = Buffer.from('Controlla benchmark\n');
  const expected = fixtureKind === 'form' ? {kind:'form',name:'Benchmark User',notes:'Persisted benchmark form'} : {kind:'upload',filename:'benchmark.txt',size:bytes.length,sha256:createHash('sha256').update(bytes).digest('hex')};
  if (base.protocol !== 'http:' || base.hostname !== '127.0.0.1' || base.username || base.password || url.origin !== base.origin || url.username || url.password || url.pathname !== (fixtureKind === 'form' ? '/registration-form' : '/file-input') || [...url.searchParams.keys()].join(',') !== 'task' || url.searchParams.get('task') !== id || !input.acceptance || Object.keys(input.acceptance).length !== Object.keys(expected).length || Object.entries(expected).some(([key,value])=>input.acceptance[key]!==value)) throw new Error('adapter requires the exact disposable loopback fixture and acceptance');
  const binary = process.env.PHASE11_CONTROLLA_BINARY;
  const chrome = process.env.PHASE11_CHROME_PATH;
  if (!chrome || adapter === 'controlla' && !binary) throw new Error('configured Chrome and Controlla binary paths are required');
  const dir = await mkdtemp(join(tmpdir(), 'controlla-phase11-form-'));
  try {
    const state = join(dir, 'state');
    await mkdir(state);
    const configured = setup(adapter, chrome, binary, state);
    const directCli = adapter === 'playwright-mcp' ? process.env.PHASE11_PLAYWRIGHT_MCP_CLI : undefined;
    if (directCli) {
      if (!isAbsolute(directCli) || basename(directCli) !== 'cli.js') throw new Error('Playwright MCP CLI must be an absolute cli.js path');
      const pkg = JSON.parse(await readFile(join(dirname(directCli), 'package.json'), 'utf8'));
      if (pkg.name !== '@playwright/mcp' || pkg.version !== '0.0.83') throw new Error('Playwright MCP CLI package is not the pinned version');
      configured.config.push('-c',`mcp_servers.playwright-mcp.command=${JSON.stringify(process.execPath)}`,'-c',`mcp_servers.playwright-mcp.args=${JSON.stringify([directCli,'--browser','chrome','--headless','--isolated','--executable-path',chrome])}`);
    }
    const file = join(dir,'benchmark.txt');
    if (fixtureKind === 'upload') await writeFile(file,bytes,{flag:'wx'});
    const goal = fixtureKind === 'form' ? 'fill Name with "Benchmark User" and Notes with "Persisted benchmark form", click Save form, and verify the visible Saved status' : adapter === 'controlla' ? `Register the synthetic bytes with browser action=artifact_register, filename benchmark.txt, and bytes_base64 ${bytes.toString('base64')}. Use the returned artifact_handle in browser action=task with task {"kind":"upload","artifact_handle":"<returned handle>","selector":"#upload","trigger_reference":null,"acceptance":null,"persisted":null,"account_marker":["h1","Controlla Phase 11 · guarded-upload"]}. Selection alone is not application acceptance. After selection, take a fresh snapshot and use the act tool with action="click", reference for Register upload, and outcome {"kind":"text","selector":"#status","text":"Upload saved"}; require a verified result.` : `select and upload the synthetic file benchmark.txt, click Register upload, and verify the visible Upload saved status. The synthetic file path for the browser upload tool is ${file}.`;
    const route = adapter === 'controlla' ? `Start with browser action=launch and mode=headless. Use its returned session_id and target_ids[0] as chrome_tab_id. Navigate using browser action=task with that session_id and chrome_tab_id and task {"kind":"research","urls":["${url.href}"],"selector":"h1"}; then snapshot for fresh references. Browser action=open is only for shared tabs; workflow has no navigate step.` : 'Start an isolated headless browser session.';
    const prompt = `Use only ${configured.server} browser tools. ${route} Navigate to ${url.href}, ${goal}, then close or release the browser session. Do not use shell commands, web search, direct HTTP requests, or other tools. Return exactly DONE if every step succeeded, otherwise return exactly FAILED.`;
    const args = ['exec','--ephemeral','--json','--ignore-user-config','--disable','shell_tool','--disable','unified_exec','--disable','apps','--disable','plugins','--disable','remote_plugin','--approve-for-me','-c','web_search="disabled"','--skip-git-repo-check','-m',process.env.PHASE11_MODEL ?? 'gpt-6-luna','-C',dir,...configured.config,'-c',`mcp_servers.${configured.server}.startup_timeout_sec=120`,prompt];
    const env = Object.fromEntries(['PATH','HOME','USER','SHELL','TMPDIR','TEMP','TMP','LANG','LC_ALL','CODEX_HOME','SSL_CERT_FILE','SYSTEMROOT','USERPROFILE','APPDATA','LOCALAPPDATA'].filter(key=>process.env[key] !== undefined).map(key=>[key,process.env[key]]));
    const events = []; let text = '', oversized = false, timedOut = false;
    const result = await new Promise((resolveRun, reject) => {
      const child = spawn(process.env.PHASE11_CODEX_PATH ?? 'codex', args, {cwd:dir,env,detached:process.platform !== 'win32',stdio:['ignore','pipe','ignore']});
      const stop = () => { try { if (process.platform !== 'win32' && child.pid) process.kill(-child.pid, 'SIGKILL'); else child.kill('SIGKILL'); } catch {} };
      process.once('SIGTERM', stop);
      const timer = setTimeout(() => { timedOut = true; stop(); }, Math.max(1, Math.min(Number(input.deadline_ms) || 60000, 600000) - 1000));
      child.stdout.on('data', chunk => { text += chunk; if (Buffer.byteLength(text) > 4_000_000) { oversized = true; stop(); } });
      child.once('error', error => { clearTimeout(timer); process.removeListener('SIGTERM', stop); reject(error); });
      child.once('close', code => { clearTimeout(timer); process.removeListener('SIGTERM', stop); stop(); resolveRun(code); });
    });
    let malformed = false;
    for (const line of text.split('\n')) { if (!line.trim()) continue; try { events.push(JSON.parse(line)); } catch { malformed = true; } }
    const calls = events.filter(event => event.type === 'item.completed' && event.item?.type === 'mcp_tool_call');
    const decoded = event => {
      const result = event.item?.result;
      let value = result?.structured_content ?? result?.structuredContent;
      if (!value) {
        const text = result?.content?.find?.(entry => entry.type === 'text')?.text;
        if (text) { try { value = JSON.parse(text); } catch {} }
        else if (result && typeof result === 'object' && !Array.isArray(result.content)) value = result;
      }
      return value;
    };
    const selectionOnly = event => {
      const item = event.item, value = decoded(event), receipt = value?.receipts?.[0];
      const selected = receipt?.files?.files?.[0];
      return adapter === 'controlla' && fixtureKind === 'upload' && item?.tool === 'browser' && item.arguments?.task?.kind === 'upload' && value?.status === 'incomplete' && value.receipts?.length === 1 && receipt?.status === 'selected' && receipt.chrome_selected_file === true && selected?.filename === expected.filename && selected?.size === expected.size && selected?.sha256 === expected.sha256;
    };
    const callSucceeded = event => {
      const item = event.item, result = item?.result;
      if (item?.status !== 'completed' || item?.error || !result || typeof result !== 'object' || result.isError || result.is_error) return false;
      if (selectionOnly(event)) return true;
      const value = decoded(event);
      return typeof value?.status !== 'string' || !/(?:error|fail|blocked|not_dispatched|unknown|incomplete|inconclusive|unverified|uncertain)/i.test(value.status);
    };
    const intended = calls.filter(event => event.item?.server === configured.server && callSucceeded(event));
    const unrelated = events.some(event => ['item.started','item.completed'].includes(event.type) && (['command_execution','web_search'].includes(event.item?.type) || event.item?.type === 'mcp_tool_call' && event.item?.server !== configured.server));
    const final = events.filter(event => event.type === 'item.completed' && event.item?.type === 'agent_message').at(-1)?.item?.text?.trim();
    const selectionIndexes = calls.flatMap((event, index) => selectionOnly(event) ? [index] : []);
    const selectionIndex = selectionIndexes[0] ?? -1;
    const selectionArgs = selectionIndex < 0 ? null : calls[selectionIndex].item.arguments;
    const selectionConfirmed = adapter !== 'controlla' || fixtureKind !== 'upload' || Boolean(selectionIndexes.length === 1 && selectionArgs?.session_id && selectionArgs?.chrome_tab_id && calls.slice(selectionIndex + 1).some(event => event.item?.server === configured.server && event.item?.tool === 'act' && event.item?.arguments?.session_id === selectionArgs.session_id && event.item?.arguments?.chrome_tab_id === selectionArgs.chrome_tab_id && event.item?.arguments?.action === 'click' && event.item?.arguments?.outcome?.kind === 'text' && event.item.arguments.outcome.selector === '#status' && event.item.arguments.outcome.text === 'Upload saved' && decoded(event)?.status === 'verified'));
    const success = result === 0 && !oversized && !malformed && intended.length > 0 && intended.length === calls.length && !unrelated && selectionConfirmed && final === 'DONE';
    const failedCall = calls.find(event => !callSucceeded(event))?.item;
    const failedTool = /^[a-zA-Z0-9_-]{1,80}$/.test(failedCall?.tool ?? '') ? failedCall.tool : 'unknown';
    const failure = oversized || malformed ? 'Codex host returned invalid or oversized event output' : timedOut ? 'Codex host timed out' : result !== 0 ? `Codex host exited with code ${result ?? 'unknown'}` : unrelated ? 'Codex host used an unrelated tool' : calls.length === 0 ? 'Codex host made no MCP calls' : intended.length !== calls.length ? `Codex host MCP call failed: ${failedTool}` : 'Codex host did not report task completion';
    output({success, mcp_calls:success ? calls.length : null, runner_error:success ? undefined : failure});
  } finally { await rm(dir,{recursive:true,force:true}); }
  process.exit(0);
}
output({runner_error:`convenience adapter is unqualified for task ${id}: the fixture event log is not an independent task acceptance oracle`});
process.exit(0);
