#!/usr/bin/env node
import { evaluateAcceptance } from './phase11-verifier-state.mjs';
const chunks=[]; let bytes=0; for await (const chunk of process.stdin) {bytes+=chunk.length;if(bytes>65536)throw new Error('request too large');chunks.push(chunk);}
const request=JSON.parse(Buffer.concat(chunks).toString()); const input=request.input??request;
const base=process.env.PHASE11_FIXTURE_BASE_URL; if(!base)throw new Error('PHASE11_FIXTURE_BASE_URL is required');
let state=null;
if(process.env.PHASE11_ORACLE_TOKEN){
 const response=await fetch(`${base}/__state?task_id=${encodeURIComponent(input.task_id)}&run_id=${encodeURIComponent(input.run_id??'')}`,{headers:{authorization:`Bearer ${process.env.PHASE11_ORACLE_TOKEN}`},signal:AbortSignal.timeout(Math.min(input.deadline_ms??60000,60000))});
 if(response.ok)state=await response.json();
}
console.log(JSON.stringify(evaluateAcceptance(input,state)));
