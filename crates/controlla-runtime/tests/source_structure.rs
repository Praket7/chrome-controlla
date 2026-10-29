//! Source-structure ratchet.
//!
//! The execution core is still being decomposed. Until that work lands, these
//! assertions stop the two production files from silently regrowing and stop a
//! second implementation of the guard, recovery, or program contracts from
//! appearing next to the canonical one. When a file is genuinely decomposed,
//! lower its ceiling in the same change that moves the code out.

use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn read(relative: &str) -> String {
    let path = root().join(relative);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn line_count(relative: &str) -> usize {
    read(relative).lines().count()
}

#[test]
fn production_monoliths_do_not_regrow() {
    // Recorded at 56970d9, raised for the executed recovery ladders in click
    // and type dispatch plus the Docs canvas verification path (2026-10-29).
    // Both files must shrink as subsystems move out; they may never grow past
    // this point.
    assert!(
        line_count("crates/controlla-runtime/src/mcp/v3_full.rs") <= 31_671,
        "v3_full.rs regrew; move implementation into a subsystem module instead of adding lines"
    );
    assert!(
        line_count("extensions/chrome-controlla/background.js") <= 3_322,
        "background.js regrew; move the subsystem into extensions/chrome-controlla/lib"
    );
    assert!(
        line_count("crates/controlla-runtime/src/task_program.rs") <= 900,
        "the shared task program must stay a bounded representation, not a second runtime"
    );
}

#[test]
fn one_module_owns_each_cross_cutting_contract() {
    let owners = [
        (
            "crates/controlla-runtime/src/guard_reason.rs",
            "pub enum GuardReason",
        ),
        (
            "crates/controlla-runtime/src/action_recovery.rs",
            "pub enum RecoveryClass",
        ),
        (
            "crates/controlla-runtime/src/task_program.rs",
            "pub enum TaskStepKind",
        ),
        (
            "crates/controlla-runtime/src/browser_contract.rs",
            "pub struct ExecutionPolicy",
        ),
    ];
    for (owner, declaration) in owners {
        assert!(
            read(owner).contains(declaration),
            "{owner} must keep the canonical {declaration} declaration"
        );
    }
    // A duplicate definition anywhere else is the failure this test exists for.
    for duplicate in [
        "crates/controlla-runtime/src/mcp/v3_full.rs",
        "crates/controlla-runtime/src/mcp.rs",
        "crates/controlla-runtime/src/mcp/v2_full.rs",
    ] {
        let source = read(duplicate);
        for declaration in [
            "pub enum GuardReason",
            "pub enum RecoveryClass",
            "pub enum TaskStepKind",
        ] {
            assert!(
                !source.contains(declaration),
                "{duplicate} redeclares {declaration}; import the canonical module instead"
            );
        }
    }
}

#[test]
fn extension_subsystems_stay_in_lib_modules() {
    let background = read("extensions/chrome-controlla/background.js");
    let mut imports = background
        .lines()
        .filter_map(|line| line.split_once("from \"./lib/"))
        .filter_map(|(_, tail)| tail.split_once('"'))
        .map(|(module, _)| module.to_owned())
        .collect::<Vec<_>>();
    imports.sort_unstable();
    imports.dedup();
    assert!(
        imports.len() >= 6,
        "background.js must compose lib modules, found {imports:?}"
    );
    for module in &imports {
        let relative = format!("extensions/chrome-controlla/lib/{module}");
        assert!(
            root().join(&relative).exists(),
            "background.js imports missing module {relative}"
        );
        assert!(
            line_count(&relative) <= 900,
            "{relative} grew past the subsystem budget; split it"
        );
    }
    // The extension modules are the only owners of these policies.
    assert!(
        read("extensions/chrome-controlla/lib/observation.js")
            .contains("export function semanticDelta")
    );
    assert!(read("extensions/chrome-controlla/lib/editor-adapters.js").contains("export function"));
}

#[test]
fn normal_agent_instructions_stay_short() {
    let skill_path =
        root().join("packages/chrome-controlla/plugin/skills/browser-control/SKILL.md");
    let bytes = fs::metadata(&skill_path).expect("skill file").len();
    assert!(
        bytes <= 4096,
        "the normal skill must stay a fast-path contract, found {bytes} bytes"
    );
    let advanced = fs::metadata(skill_path.with_file_name("references/advanced.md"))
        .expect("advanced reference");
    assert!(
        advanced.len() > bytes as u64,
        "recovery and capability detail belongs in advanced.md"
    );
}
