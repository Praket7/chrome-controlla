use controlla_runtime::visual_probe::{EvidenceMemory, MAX_PROBES_PER_WORKFLOW, escalation_policy};

#[test]
fn targeted_probe_resolves_ambiguous_semantics_within_budget() {
    let mut mem = EvidenceMemory::new();
    // Simulate semantic insufficient, one targeted crop resolves
    let has_semantics = false;
    let policy = escalation_policy(has_semantics, 0);
    assert_eq!(policy, "targeted visual");
    mem.push("probe-1", vec![0u8; 1024]).unwrap();
    assert_eq!(mem.len(), 1);
}

#[test]
fn unlimited_probes_are_rejected() {
    let mut mem = EvidenceMemory::new();
    for i in 0..MAX_PROBES_PER_WORKFLOW {
        mem.push(format!("e{i}"), vec![0; 100]).unwrap();
    }
    assert!(mem.push("extra", vec![0; 100]).is_err());
}
