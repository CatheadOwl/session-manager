//! CLI adapter for the Q&A export capability (ADR 0005, workunit
//! 20260909-2109). Dual-mode binary: this module runs only when the first
//! CLI argument is a known subcommand — otherwise `run()` launches the GUI
//! unchanged. The CLI is a THIN adapter: it parses flags, resolves the time
//! window, and delegates all filtering/distilling/rendering to
//! `session_manager::export_*` — the same core the Tauri command uses.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use crate::session_manager;
use crate::session_manager::SessionScope;

/// Time-window resolution: `--days N` or explicit `--from`/`--to` (epoch
/// milliseconds, inclusive — the app-wide timestamp unit).
#[derive(Debug)]
struct TimeWindow {
    from: i64,
    to: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AgentArg {
    Claude,
    Codex,
    Gemini,
    Hermes,
    Openclaw,
    Opencode,
    Pi,
    Qoder,
}

impl AgentArg {
    fn provider_id(self) -> &'static str {
        match self {
            AgentArg::Claude => "claude",
            AgentArg::Codex => "codex",
            AgentArg::Gemini => "gemini",
            AgentArg::Hermes => "hermes",
            AgentArg::Openclaw => "openclaw",
            AgentArg::Opencode => "opencode",
            AgentArg::Pi => "pi",
            AgentArg::Qoder => "qoder",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FormatArg {
    Json,
    Markdown,
}

impl From<FormatArg> for session_manager::QaExportFormat {
    fn from(value: FormatArg) -> Self {
        match value {
            FormatArg::Json => Self::Json,
            FormatArg::Markdown => Self::Markdown,
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum CliCommand {
    /// Export Q&A-distilled sessions to stdout or a file
    Export {
        /// Time window in days back from now
        #[arg(long, conflicts_with_all = ["from", "to"])]
        days: Option<u32>,
        /// Window start, epoch milliseconds (inclusive)
        #[arg(long, conflicts_with = "days", requires = "to")]
        from: Option<i64>,
        /// Window end, epoch milliseconds (inclusive)
        #[arg(long, conflicts_with = "days", requires = "from")]
        to: Option<i64>,
        /// Restrict to these agents (repeatable; default: all)
        #[arg(long = "agent", value_enum)]
        agents: Vec<AgentArg>,
        /// Output format
        #[arg(long, value_enum, default_value_t = FormatArg::Json)]
        format: FormatArg,
        /// Omit provenance metadata (source tracing)
        #[arg(long, default_value_t = false)]
        no_metadata: bool,
        /// Write to this file instead of stdout (refuses to overwrite)
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// List available agent (provider) ids
    Agents,
}

/// True when the process should run in CLI mode: the first argument is a
/// known subcommand. Anything else (including stray flags) falls through to
/// the GUI so GUI launches never break on unexpected args.
pub fn is_cli_invocation() -> bool {
    matches!(
        std::env::args().nth(1).as_deref(),
        Some("export") | Some("agents")
    )
}

/// Parse and run the CLI path. Returns the process exit code.
pub fn run_cli() -> i32 {
    let Cli { command } = Cli::parse();
    match command {
        Some(CliCommand::Export {
            days,
            from,
            to,
            agents,
            format,
            no_metadata,
            out,
        }) => run_export(days, from, to, agents, format.into(), no_metadata, out),
        Some(CliCommand::Agents) => run_agents(),
        // Parser-level guard: is_cli_invocation() means a subcommand was
        // present; clap with subcommand_required would not get here.
        None => {
            eprintln!("no subcommand given");
            2
        }
    }
}

fn resolve_window(days: Option<u32>, from: Option<i64>, to: Option<i64>) -> TimeWindow {
    if let Some(days) = days {
        let now = chrono::Utc::now().timestamp_millis();
        TimeWindow {
            from: now - i64::from(days) * 86_400_000,
            to: now,
        }
    } else {
        TimeWindow {
            from: from.unwrap_or(i64::MIN),
            to: to.unwrap_or(i64::MAX),
        }
    }
}

fn run_export(
    days: Option<u32>,
    from: Option<i64>,
    to: Option<i64>,
    agents: Vec<AgentArg>,
    format: session_manager::QaExportFormat,
    no_metadata: bool,
    out: Option<PathBuf>,
) -> i32 {
    let window = resolve_window(days, from, to);
    let providers: Option<Vec<String>> = if agents.is_empty() {
        None
    } else {
        Some(agents.iter().map(|a| a.provider_id().to_string()).collect())
    };

    let registry = session_manager::build_provider_registry();
    let batch = session_manager::export_qa_sessions(
        &registry,
        &SessionScope::Active,
        window.from,
        window.to,
        providers.as_deref(),
    );

    for skipped in &batch.skipped {
        eprintln!(
            "skipped provider={} session={}: {}",
            skipped.provider_id, skipped.session_id, skipped.error
        );
    }

    let content = match session_manager::render_export(
        &batch,
        window.from,
        window.to,
        format,
        !no_metadata,
    ) {
        Ok(content) => content,
        Err(err) => {
            eprintln!("error: {err}");
            return 1;
        }
    };

    match out {
        Some(path) => {
            if let Err(err) = session_manager::write_export_file(&path, &content, false) {
                eprintln!("error: {err}");
                return 1;
            }
            eprintln!(
                "exported {} session(s) to {}",
                batch.sessions.len(),
                path.display()
            );
        }
        None => {
            println!("{content}");
        }
    }
    0
}

fn run_agents() -> i32 {
    let registry = session_manager::build_provider_registry();
    for id in registry.ids() {
        println!("{id}");
    }
    0
}

/// CLI argument root. Subcommand is optional at the type level because the
/// GUI path never parses; `run_cli` relies on `is_cli_invocation()` having
/// seen a known subcommand first.
#[derive(Debug, Parser)]
#[command(
    name = "session-manager",
    about = "Browse, search, inspect, archive, and clean up local AI coding-agent sessions",
    subcommand_negates_reqs = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse(args: &[&str]) -> Result<Option<CliCommand>, String> {
        let mut full = vec!["session-manager"];
        full.extend_from_slice(args);
        let Cli { command } =
            Cli::try_parse_from(full).map_err(|e| e.to_string())?;
        Ok(command)
    }

    #[test]
    fn days_window_parses() {
        let Some(CliCommand::Export { days: Some(7), from, to, agents, format, .. }) =
            parse(&["export", "--days", "7"]).expect("parse")
        else {
            panic!("expected export subcommand");
        };
        assert_eq!(from, None);
        assert_eq!(to, None);
        assert!(agents.is_empty());
        assert_eq!(format, FormatArg::Json);
    }

    #[test]
    fn days_conflicts_with_from() {
        let err = parse(&["export", "--days", "7", "--from", "1"]).expect_err("must conflict");
        assert!(err.contains("--days"), "unexpected: {err}");
    }

    #[test]
    fn from_requires_to() {
        let err = parse(&["export", "--from", "1"]).expect_err("requires to");
        assert!(err.contains("--to"), "unexpected: {err}");
    }

    #[test]
    fn agent_enum_rejects_unknown_and_accepts_repeat() {
        let err = parse(&["export", "--days", "1", "--agent", "openai"]).expect_err("unknown agent");
        assert!(err.contains("invalid value"), "unexpected: {err}");

        let Some(CliCommand::Export { agents, .. }) =
            parse(&["export", "--days", "1", "--agent", "codex", "--agent", "gemini"])
                .expect("parse")
        else {
            panic!("expected export subcommand");
        };
        assert_eq!(agents, vec![AgentArg::Codex, AgentArg::Gemini]);
    }

    #[test]
    fn markdown_flag_and_defaults() {
        let Some(CliCommand::Export { format, no_metadata, out, .. }) =
            parse(&["export", "--days", "1", "--format", "markdown", "--no-metadata"])
                .expect("parse")
        else {
            panic!("expected export subcommand");
        };
        assert_eq!(format, FormatArg::Markdown);
        assert!(no_metadata);
        assert_eq!(out, None);
    }

    #[test]
    fn window_resolution_days_and_explicit() {
        let w = resolve_window(Some(7), None, None);
        assert!(w.to - w.from <= 7 * 86_400_000 + 1_000);
        let w = resolve_window(None, Some(100), Some(200));
        assert_eq!((w.from, w.to), (100, 200));
    }
}
