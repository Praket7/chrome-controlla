//! Task-program facade. The shared program representation lives in the
//! library crate so workflows and `browser(action="task")` compile to the same
//! execution model.

pub use controlla_runtime::task_program::{
    TaskProgram, TaskProgramLimits, TaskStep, TaskStepKind, compile_task_program,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facade_reexports_the_shared_program_type() {
        let limits = TaskProgramLimits::default();
        assert_eq!(limits.max_steps, 128);
        let program = TaskProgram {
            steps: vec![TaskStep {
                kind: TaskStepKind::Observe,
                ..TaskStep::default()
            }],
        };
        assert_eq!(program.steps.len(), 1);
        assert!(compile_task_program(&program, limits).is_ok());
    }
}
