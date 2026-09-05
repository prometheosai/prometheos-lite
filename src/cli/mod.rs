use clap::{Parser, Subcommand};

pub mod commands;
pub mod runner;
pub mod runtime_builder;

#[derive(Debug, Parser)]
#[command(
    name = "prometheos",
    version,
    about = "Local-first multi-agent coding CLI"
)]
pub struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Run a flow from a JSON or YAML file
    Flow(commands::flow::FlowCommand),
    /// P2-014: Harness V1.6 commands for autonomous/assisted coding
    ///
    /// Run harness on a task, inspect results, dry-run, apply, or rollback.
    /// This is the primary interface for the V1.6 coding harness.
    Harness(commands::harness::HarnessCommand),
    /// Start the API server for the local chat interface
    Serve(commands::serve::ServeCommand),
    /// Run benchmark tasks
    Bench(commands::bench::BenchCommand),
    /// Manage WorkContexts
    Work(commands::work::WorkCommand),
    /// MVP local repo workbench: scan, plan, stage artifacts, require approval, and remember
    #[command(name = "repo", alias = "repo-workbench")]
    RepoWorkbench(commands::repo_workbench::RepoWorkbenchCommand),
    /// Manage domain templates
    Templates(commands::templates::TemplatesCommand),
    /// Approval-controlled patch workflow: propose -> dry-run -> approve -> apply -> report
    Workflow(commands::workflow::WorkflowCommand),
    /// Run provider/system/validation diagnostics
    Diagnostics(commands::diagnostics::DiagnosticsArgs),
}

pub async fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Flow(cmd) => cmd.execute().await,
        Commands::Harness(cmd) => cmd.execute().await,
        Commands::Serve(cmd) => cmd.execute().await,
        Commands::Bench(cmd) => cmd.execute().await,
        Commands::Work(cmd) => cmd.execute().await,
        Commands::RepoWorkbench(cmd) => cmd.execute().await,
        Commands::Templates(cmd) => cmd.execute().await,
        Commands::Workflow(cmd) => cmd.execute().await,
        Commands::Diagnostics(args) => {
            commands::diagnostics::handle_diagnostics_command(args).await
        }
    }
}

#[cfg(test)]
mod cli_contract_tests {
    //! CLI contract integration tests for the top-level `Commands` enum
    //! above. Each test parses a documented invocation through
    //! `clap::Parser::try_parse_from` and asserts the parser shape — it
    //! does NOT execute the command (no runtime needed).
    //!
    //! Acceptance (E6/I01 #130): "CLI contracts are documented and
    //! integration-tested". The `clap::Parser` derives on each
    //! subcommand already carry `#[command(about = "...")]` doc
    //! strings; this module provides the integration-test side of that
    //! contract.
    //!
    //! These tests live inside `src/cli/mod.rs` (as `#[cfg(test)] mod`)
    //! rather than as an integration test in `tests/`, because the `Cli`
    //! enum is a binary-only type — it is not re-exported from the
    //! library crate's public API, so it is not reachable from
    //! `tests/cli_contract_tests.rs`. Putting the tests in-module
    //! avoids a public-API change while still keeping them as proper
    //! `cargo test` cases.

    use super::Cli;
    use clap::Parser;

    /// Parse an arg list (with `argv[0]` being the program name) and
    /// return the parsed `Cli`. The tests use `expect` /
    /// `expect_err` so a failure is explicit.
    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        let mut full = Vec::with_capacity(args.len() + 1);
        full.push("prometheos");
        full.extend_from_slice(args);
        Cli::try_parse_from(full)
    }

    #[test]
    fn cli_parses_flow_run_with_minimum_args() {
        let cli = parse(&["flow", "run", "fixtures/flows/ok.yaml"]).expect("parse must succeed");
        let _ = cli;
    }

    #[test]
    fn cli_rejects_flow_run_without_required_path() {
        let err = parse(&["flow", "run"]).expect_err("parse must fail without the path arg");
        let msg = err.to_string();
        assert!(
            msg.contains("required") || msg.contains("FLOW_FILE") || msg.contains("<FLOW_FILE>"),
            "expected clap to name the missing arg, got: {msg}"
        );
    }

    #[test]
    fn cli_parses_harness_run_with_required_task() {
        let cli = parse(&["harness", "run", "fix failing tests"]).expect("parse must succeed");
        let _ = cli;
    }

    #[test]
    fn cli_rejects_harness_run_without_required_task() {
        let err = parse(&["harness", "run"]).expect_err("parse must fail without task");
        let msg = err.to_string();
        assert!(
            msg.contains("required") || msg.contains("TASK") || msg.contains("<TASK>"),
            "expected clap to name the missing task arg, got: {msg}"
        );
    }

    #[test]
    fn cli_parses_serve() {
        let cli = parse(&["serve"]).expect("parse must succeed");
        let _ = cli;
    }

    #[test]
    fn cli_parses_bench_run_with_required_task() {
        // `prometheos bench run` takes a `task` flag, not a path
        // positional. The default `task` is `all`, so `bench run`
        // alone parses.
        let cli = parse(&["bench", "run", "--task", "planning"]).expect("parse must succeed");
        let _ = cli;
    }

    #[test]
    fn cli_rejects_bench_run_with_unknown_subcommand() {
        let err = parse(&["bench", "frobnicate"]).expect_err("parse must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("invalid") || msg.contains("unrecognized") || msg.contains("Usage"),
            "expected clap to name the unknown bench subcommand, got: {msg}"
        );
    }

    #[test]
    fn cli_parses_work_with_subcommand() {
        let cli = parse(&["work", "list"]).expect("work list must parse");
        let _ = cli;
    }

    #[test]
    fn cli_parses_repo_workbench_alias() {
        // The `repo` and `repo-workbench` aliases must both be accepted
        // with a subcommand. `status` requires a positional `<ID>`; the
        // value is irrelevant for parse-only tests.
        let cli1 = parse(&["repo", "status", "wf-1"]).expect("`repo status wf-1` must parse");
        let cli2 = parse(&["repo-workbench", "status", "wf-1"])
            .expect("`repo-workbench status wf-1` must parse");
        let _ = (cli1, cli2);
    }

    #[test]
    fn cli_parses_templates_list() {
        let cli = parse(&["templates", "list"]).expect("templates list must parse");
        let _ = cli;
    }

    #[test]
    fn cli_parses_workflow_propose() {
        // `prometheos workflow` is the approval-controlled patch flow.
        // All subcommands require flags; we use `report` (--repo +
        // positional ID) as the smallest happy-path parse. The exact
        // flag shape is intentionally not asserted here.
        let cli = parse(&["workflow", "report", "--repo", ".", "wf-1"])
            .expect("workflow report must parse");
        let _ = cli;
    }

    #[test]
    fn cli_parses_diagnostics_provider() {
        // `diagnostics` requires a subcommand (provider / system /
        // validation / full).
        let cli = parse(&["diagnostics", "provider"]).expect("diagnostics provider must parse");
        let _ = cli;
    }

    #[test]
    fn cli_parses_work_inspect() {
        // E6/I02 Slice A (R7): the read-only run inspector.
        // `prometheos work inspect <id>` accepts an optional `--json`
        // flag. The inspect command itself is a thin shell that
        // delegates to the repo-workbench loader; the load is
        // exercised by the unit tests in `src/cli/commands/work.rs`.
        let cli1 = parse(&["work", "inspect", "wf-1"]).expect("work inspect wf-1 must parse");
        let _ = cli1;
        let cli2 = parse(&["work", "inspect", "wf-1", "--json"])
            .expect("work inspect wf-1 --json must parse");
        let _ = cli2;
    }

    #[test]
    fn cli_rejects_unknown_subcommand() {
        let err = parse(&["flow", "frobnicate"]).expect_err("parse must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("invalid") || msg.contains("unexpected") || msg.contains("unrecognized"),
            "expected clap to name the unknown subcommand, got: {msg}"
        );
    }

    #[test]
    fn cli_rejects_unknown_top_level_command() {
        let err = parse(&["frobnicate"]).expect_err("parse must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("invalid") || msg.contains("unrecognized") || msg.contains("Usage"),
            "expected clap to name the unknown command, got: {msg}"
        );
    }
}
