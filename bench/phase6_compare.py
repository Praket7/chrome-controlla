#!/usr/bin/env python3
"""Deterministic planning fixture; counts only, not browser or performance evidence."""

import json

steps = [
    {"id": "read_account", "deps": ["account"], "effect": "read"},
    {"id": "read_rows", "deps": ["account", "rows"], "effect": "read"},
    {"id": "format", "deps": ["read_rows"], "effect": "read"},
    {"id": "submit", "deps": ["format"], "effect": "external"},
    {"id": "confirm", "deps": ["submit"], "effect": "unknown"},
    {"id": "audit", "deps": ["unrelated_counter"], "effect": "read"},
]

invalid = {"rows"}
while True:
    added = {s["id"] for s in steps if s["id"] not in invalid and invalid.intersection(s["deps"])}
    if not added:
        break
    invalid |= added
affected = [s["id"] for s in steps if s["id"] in invalid]

batches = []
for i, step in enumerate(steps):
    if step["effect"] != "read" or not batches or steps[batches[-1][0]]["effect"] != "read":
        batches.append([i])
    else:
        batches[-1].append(i)

print(json.dumps({
    "evidence": "deterministic local fixture counts; no browser, latency, or model measurement",
    "fixture_steps": len(steps),
    "changed_dependency": "rows",
    "invalidated_steps": affected,
    "strategies": {
        "fixed_batch": {"initial_calls": 1, "recovery_steps": len(steps)},
        "bounded_code": {"initial_calls": 1, "recovery_steps": len(steps)},
        "guarded_compiler": {"initial_batches": len(batches), "recovery_steps": len(affected)},
    },
}, indent=2))
