#[path = "../mcp/v3/mod.rs"]
pub mod v3_execution;

pub use controlla_runtime::{
    action_recovery, apps, artifacts, browser_contract, browser_workflow, cache, canva, capability,
    capability_registry, capability_router, capcut_recipe, compact_catalog, desktop_fallback,
    desktop_platform, doctor, document_text, evidence_store, execution_cost_router, execution_plan,
    guard_reason, http_auth, http_server, jobs, metrics, native_host, native_setup,
    observation_router, page_action_memory, process_lease, progressive_observation, recovery_exec,
    reference_reacquire, reference_rebase, relevance, runtime_observer, semantic_state,
    site_profiles, skill_induction, skill_runtime, skill_store, skills, slides_deck,
    target_registry, task_checkpoints, task_observation, task_program, text_conflict, v3,
    v3_runtime, v3_tasks, verifier, visual_probe, visual_stability, widget_adapters, workflow,
    workflow_dag, workflow_vm,
};

#[allow(dead_code, clippy::unnecessary_sort_by)]
mod legacy_mcp {
    include!("../mcp.rs");

    mod v2 {
        include!("../mcp/v2.rs");

        pub(super) mod full {
            include!("../mcp/v2_full.rs");
        }

        pub(super) mod v3 {
            include!("../mcp/v3_full.rs");
        }

        pub(super) mod recovery {
            include!("../mcp/v3_recovery.rs");
        }

        pub(super) async fn run_v3() -> Result<(), String> {
            v3::run().await
        }
    }

    pub(crate) async fn run_compact() -> Result<(), String> {
        v2::run_v3().await
    }
}

#[tokio::main]
async fn main() {
    // Chrome launches the registered native host with the calling extension
    // origin as its only argument. This binary is documented as the MCP
    // endpoint, but a copy of it can end up registered as the native host
    // (for example after a release swap). Serving the host role here keeps
    // such a mismatch working instead of leaving Chrome with a process that
    // never publishes `ready` and silently kills the whole bridge.
    let mut args = std::env::args().skip(1);
    if let Some(origin) = args.next()
        && origin.starts_with("chrome-extension://")
        && args.next().is_none()
    {
        let origin = origin.strip_suffix('/').unwrap_or(&origin);
        let id = &origin["chrome-extension://".len()..];
        if id.len() != 32 || !id.bytes().all(|byte| (b'a'..=b'p').contains(&byte)) {
            eprintln!("invalid native host origin");
            std::process::exit(2);
        }
        // Already inside the tokio runtime this async main owns.
        if let Err(error) = controlla_runtime::native_host::run().await {
            eprintln!("Chrome native host failed: {error}");
            std::process::exit(1);
        }
        return;
    }
    if let Err(error) = legacy_mcp::run_compact().await {
        eprintln!("Controlla v3 MCP server failed: {error}");
        std::process::exit(1);
    }
}
