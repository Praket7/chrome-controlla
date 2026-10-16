#!/usr/bin/env node
import { readFile } from 'node:fs/promises';
const chunks=[]; for await (const chunk of process.stdin) chunks.push(chunk); const input=JSON.parse(Buffer.concat(chunks).toString());
const base=process.env.PHASE11_FIXTURE_BASE_URL; if(!base) throw new Error('PHASE11_FIXTURE_BASE_URL is required');
const response=await fetch(`${base}/__reset`,{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(input),signal:AbortSignal.timeout(input.deadline_ms??60000)});
if(!response.ok) throw new Error(`fixture reset failed: ${response.status}`); console.log(JSON.stringify(await response.json()));
