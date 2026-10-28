#[path = "../mcp/v3/mod.rs"]
pub mod v3_execution;

pub use controlla_runtime::{
    apps, artifacts, browser_contract, browser_workflow, cache, canva, capability,
    capability_registry, capability_router, capcut_recipe, compact_catalog, desktop_fallback,
    desktop_platform, doctor, evidence_store, execution_cost_router, guard_reason, http_auth,
    http_server, jobs, metrics, native_host, native_setup, observation_router, page_action_memory,
    process_lease, reference_reacquire, relevance, runtime_observer, semantic_state, site_profiles,
    skill_induction, skill_runtime, skill_store, skills, slides_deck, task_checkpoints, v3,
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
    if let Err(error) = legacy_mcp::run_compact().await {
        eprintln!("Controlla v3 MCP server failed: {error}");
        std::process::exit(1);
    }
}
