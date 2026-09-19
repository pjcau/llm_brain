//! `brain bench`: run the real-bug suite (`bench/tasks/*.yaml`) with a tool
//! (aider or Claude Code, headless) against a tier, verify with the repo's own
//! tests, and record pass/cost/time per run. See docs/architecture/benchmark.md.

pub mod runner;
pub mod task;
pub mod tool;

pub use runner::{RunOptions, run_suite};
pub use task::load_tasks;
pub use tool::Tool;
