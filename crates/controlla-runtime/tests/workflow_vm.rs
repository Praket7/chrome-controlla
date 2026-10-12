use controlla_runtime::browser_workflow::{
    BrowserWorkflowDefinition, BrowserWorkflowStep, compile_browser_workflow,
};
use controlla_runtime::workflow_vm::MAX_EXPANDED_STEPS;

#[test]
fn workflow_compile_respects_max_steps() {
    let steps = (0..MAX_EXPANDED_STEPS + 1)
        .map(|_| BrowserWorkflowStep::Checkpoint)
        .collect::<Vec<_>>();
    let def = BrowserWorkflowDefinition { steps };
    assert!(compile_browser_workflow(def).is_err());
}
