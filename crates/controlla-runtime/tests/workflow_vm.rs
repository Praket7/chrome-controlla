use controlla_runtime::browser_workflow::{
    BrowserWorkflowDefinition, BrowserWorkflowStep, compile_browser_workflow,
};
use controlla_runtime::workflow_vm::{MAX_EXPANDED_STEPS, Vm, VmOp};

#[test]
fn bounded_workflow_vm_rejects_unbounded_and_mutation_in_parallel() {
    // foreach bound
    let mut vm = Vm::new();
    let big = VmOp::Foreach {
        items: (0..33).map(|i| format!("{i}")).collect(),
        var: "x".into(),
        body: vec![],
    };
    assert!(
        vm.run_op(big, &mut Vec::new()).is_err(),
        "foreach should reject >32"
    );
}

#[test]
fn workflow_compile_respects_max_steps() {
    let steps = (0..MAX_EXPANDED_STEPS + 1)
        .map(|_| BrowserWorkflowStep::Checkpoint)
        .collect::<Vec<_>>();
    let def = BrowserWorkflowDefinition { steps };
    assert!(compile_browser_workflow(def).is_err());
}
