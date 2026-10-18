#!/usr/bin/env node
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(fileURLToPath(new URL('../..', import.meta.url)));
const commandStarted = process.hrtime.bigint();
const adapter = process.argv[2];
const sourceRefs={'controlla-1':'62e6407d0e75b83a1be8d69468476601a6e6da37','controlla-2-pre-stage2':'406b9d9794d6cb59dc75d22c7e6fe7a8554b1e67'}; const source_ref=sourceRefs[adapter] ?? null;
const allowed = new Set(['controlla','controlla-1','controlla-2-pre-stage2','playwright-mcp','chrome-devtools-mcp','agent-browser','browser-use','stagehand']);
if (!allowed.has(adapter)) throw new Error(`unknown adapter ${adapter}`);
const chunks=[]; for await (const chunk of process.stdin) chunks.push(chunk); const input=JSON.parse(Buffer.concat(chunks).toString());
const id=input.task_id; const mode=input.mode; const executionMode=input.execution_mode;
const defaults={adapter,source_ref,task_id:id,mode,execution_mode:executionMode,concurrency:input.concurrency,success:false,independently_verified_success:false,wall_ms:null,model_round_trips:null,model_tokens:null,mcp_calls:null,browser_ops:null,observation_bytes:null,screenshot_bytes:null,retries:null,recovery_events:null,cache_status:null,verification_ms:null,cpu_ms:null,peak_rss_bytes:null,focus_disruptions:null,clipboard_disruptions:null,human_interventions:null,severe_wrong_target_events:null,authority_violations:null,silent_stale_mutations:null,external_controlla_calls:null,model_turns:null,model_visible_observations:null,internal_model_calls:null,browser_cdp_commands:null,host_round_trips:null,native_host_messages_sent:null,native_host_messages_received:null,schema_bytes:null,prompt_input_bytes:null,tool_input_bytes:null,output_bytes:null,unknown_delivery_count:null,fallback_count:null,timeout_count:null,cleanup_completed:null,cleanup_outcome:null};
const output=(fields={})=>{const result={...defaults,...fields}; if(result.wall_ms===null) result.wall_ms=Number(process.hrtime.bigint()-commandStarted)/1e6; result.metric_evidence=Object.fromEntries(contract.measurement_metric_fields.map(field=>[field,{provenance:contract.measurement_metric_provenance[field],...(result[field]===null||result[field]===undefined?{missing_reason:field==='success'||field==='independently_verified_success'?'unsupported_task_or_uninstrumented_adapter':'not reported by this adapter command'}:{})}])); console.log(JSON.stringify(result));};

const contract=JSON.parse(await readFile(resolve(root,'bench/runners/adapter-contract.json'),'utf8'));
if (executionMode !== 'headless') { output({runner_error:'real adapter smoke currently supports headless execution only; foreground/background bindings require a headed Chrome display harness'}); process.exit(0); }
output({runner_error:`convenience adapter is unqualified for task ${id}: the fixture event log is not an independent task acceptance oracle`});
process.exit(0);
