//! CLI adapter for the Q&A export capability (ADR 0005, workunit
//! 20260909-2109). Dual-mode binary: this module runs only when the first
//! CLI argument is a known subcommand — otherwise `run()` launches the GUI
//! unchanged. The CLI is a THIN adapter: it parses flags, resolves the time
//! window, and delegates all filtering/distilling/rendering to
//! `session_manager::export_*` — the same core the Tauri command uses.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use crate::session_manager;
use crate::session_manager::settings::SettingsManager;
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
    Jsonl,
    Markdown,
}

impl From<FormatArg> for session_manager::QaExportFormat {
    fn from(value: FormatArg) -> Self {
        match value {
            FormatArg::Json => Self::Json,
            FormatArg::Jsonl => Self::Jsonl,
            FormatArg::Markdown => Self::Markdown,
        }
    }
}

/// Parse a time bound: epoch milliseconds (integer, app-wide unit) or an
/// RFC3339 timestamp (converted to epoch ms, zero new dependencies — chrono
/// is already in the tree). RFC3339 removes the hand-computed-epoch friction
/// for calendar windows (evals/cli Case B).
pub fn parse_epoch_ms(value: &str) -> Result<i64, String> {
    if let Ok(ms) = value.parse::<i64>() {
        return Ok(ms);
    }
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.timestamp_millis())
        .map_err(|_| {
            format!(
                "invalid time bound '{value}': expected epoch milliseconds or RFC3339 (e.g. 2026-08-01T00:00:00Z)"
            )
        })
}

#[derive(Debug, Subcommand)]
pub enum CliCommand {
    /// Export Q&A-distilled sessions (Active, non-archived only) to stdout or a file
    Export {
        /// Time window of N x 24h back from now (omit all window flags for all-time)
        #[arg(short, long, conflicts_with_all = ["from", "to"])]
        days: Option<u32>,
        /// Window start: epoch milliseconds or RFC3339 timestamp (inclusive)
        #[arg(long, conflicts_with = "days", requires = "to", value_parser = parse_epoch_ms)]
        from: Option<i64>,
        /// Window end: epoch milliseconds or RFC3339 timestamp (inclusive)
        #[arg(long, conflicts_with = "days", requires = "from", value_parser = parse_epoch_ms)]
        to: Option<i64>,
        /// Restrict to these agents (repeatable; default: all)
        #[arg(short = 'a', long = "agent", value_enum)]
        agents: Vec<AgentArg>,
        /// Output format (jsonl: one session per line, safe for `>>` appends)
        #[arg(short, long, value_enum, default_value_t = FormatArg::Json)]
        format: FormatArg,
        /// Omit provenance metadata (source tracing)
        #[arg(long, default_value_t = false)]
        no_metadata: bool,
        /// Write to this file instead of stdout (refuses to overwrite an existing file)
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// List available agent (provider) ids
    Agents,
}

/// True when the process should run in CLI mode: the first argument is a
/// known subcommand or a root help/version flag. Anything else (including
/// stray flags) falls through to the GUI so GUI launches never break on
/// unexpected args — but root `--help`/`--version` must answer on the
/// console, never open a window.
pub fn is_cli_invocation() -> bool {
    is_cli_arg(std::env::args().nth(1).as_deref())
}

fn is_cli_arg(arg: Option<&str>) -> bool {
    matches!(
        arg,
        Some("export") | Some("agents") | Some("--help") | Some("-h") | Some("--version") | Some("-V")
    )
}

/// Windows console attach (workunit 20260910-1408). Release builds link as a
/// GUI-subsystem exe (`#![cfg_attr(not(debug_assertions), windows_subsystem =
/// "windows")]` in main.rs), so Windows attaches no console and std handles
/// are NULL when launched interactively from cmd/pwsh: clap's `--help` output
/// went nowhere. Fix per Tauri maintainer guidance (tauri#8305 comment
/// 1826871949): attach to the parent console, then re-open only the INVALID
/// std handles to `CONOUT$` (AttachConsole alone does not rewire already
/// captured handles). Valid handles — pipe redirection, console-attached
/// debug builds — are left untouched so redirection semantics never change.
/// All failures (explorer launch has no parent console; headless hosts) are
/// silently ignored: the GUI path never prints anyway. Only the CLI branch
/// calls this, and it prints-then-exits, so no FreeConsole bookkeeping is
/// needed.
#[cfg(windows)]
pub(crate) fn attach_parent_console() {
    use std::os::windows::io::AsRawHandle;

    use windows::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
    use windows::Win32::System::Console::{
        AttachConsole, GetStdHandle, SetStdHandle, ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE,
        STD_OUTPUT_HANDLE,
    };

    unsafe {
        // From cmd/pwsh: attaches to the caller's console. From explorer:
        // fails, nothing to print to — return. (A process can also already
        // hold a console, e.g. test harnesses: attach then fails too.)
        if AttachConsole(ATTACH_PARENT_PROCESS).is_err() {
            return;
        }
        let invalid = |r: windows::core::Result<HANDLE>| match r {
            Ok(h) => h.0.is_null() || h == INVALID_HANDLE_VALUE,
            Err(_) => true,
        };
        let need_out = invalid(GetStdHandle(STD_OUTPUT_HANDLE));
        let need_err = invalid(GetStdHandle(STD_ERROR_HANDLE));
        if !need_out && !need_err {
            return;
        }
        // `CONOUT$` is a device name; std::fs open goes through CreateFileW
        // with no extra windows features needed.
        let Ok(conout) = std::fs::OpenOptions::new().write(true).open("CONOUT$") else {
            return;
        };
        let handle = HANDLE(conout.as_raw_handle());
        if need_out {
            let _ = SetStdHandle(STD_OUTPUT_HANDLE, handle);
        }
        if need_err {
            let _ = SetStdHandle(STD_ERROR_HANDLE, handle);
        }
        // Keep the OS handle alive for the rest of the (short-lived) process;
        // closing the File would invalidate the std handles we just set.
        std::mem::forget(conout);
    }
}

/// Parse and run the CLI path. Returns the process exit code.
pub fn run_cli() -> i32 {
    // GUI-subsystem release builds have no console until this call; must run
    // before clap's parse (clap prints help/version during parse and exits).
    #[cfg(windows)]
    attach_parent_console();
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
    // Settings sources overlay: the CLI has no managed Tauri state, so build
    // a SettingsManager from the same settings path the GUI uses. A broken
    // or missing settings file falls back to "no extra sources" (lenient
    // load); an unresolvable home merely warns.
    let extra_sources = match crate::config::get_app_settings_path() {
        Ok(path) => SettingsManager::new(path).enabled_sources(),
        Err(err) => {
            eprintln!("warning: cannot resolve settings path: {err}");
            Vec::new()
        }
    };
    let batch = session_manager::export_qa_sessions(
        &registry,
        &SessionScope::Active,
        window.from,
        window.to,
        providers.as_deref(),
        &extra_sources,
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
            // Exactly one trailing newline: renderers may or may not end
            // with one (jsonl does via writeln, pretty json does not) —
            // normalize so `>>` appends never produce blank separator lines.
            println!("{}", content.trim_end_matches('\n'));
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
    version,
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

    #[test]
    fn cli_dispatch_whitelist() {
        for arg in ["export", "agents", "--help", "-h", "--version", "-V"] {
            assert!(is_cli_arg(Some(arg)), "{arg} should be CLI");
        }
        // Stray flags and file paths must fall through to the GUI.
        for arg in ["--foo", "some-path.json", ""] {
            assert!(!is_cli_arg(Some(arg)), "{arg} must stay GUI");
        }
        assert!(!is_cli_arg(None));
    }

    #[test]
    fn root_help_and_version_parse_and_exit_in_clap() {
        // clap handles --help/--version by exiting during parse; from the
        // parser's perspective they are errors of kind DisplayHelp/Version.
        let err = Cli::try_parse_from(["session-manager", "--version"]).expect_err("exits");
        assert!(err.to_string().contains("0.2"), "unexpected: {err}");
    }

    #[test]
    fn bare_export_means_all_time() {
        // The documented default: no window flags = full range.
        let w = resolve_window(None, None, None);
        assert_eq!((w.from, w.to), (i64::MIN, i64::MAX));
    }

    #[test]
    fn console_attach_never_panics() {
        // Workunit 20260910-1408: attach must be safe in any host state —
        // test harness (already holds a console → attach fails → early
        // return), headless CI (no parent console → early return). It must
        // never panic or disturb already-valid std handles.
        #[cfg(windows)]
        attach_parent_console();
    }

    #[test]
    fn help_text_documents_load_bearing_semantics() {
        // Guards the eval-driven wording (clig.dev review + evals/cli
        // baseline): scope, units, bounds, and default window must stay
        // discoverable from --help alone.
        use clap::CommandFactory;
        let help = Cli::command().render_help().to_string();
        assert!(help.contains("Active, non-archived"), "scope must be documented: {help}");
        let export_help = Cli::command()
            .find_subcommand("export")
            .expect("export subcommand")
            .clone()
            .render_help()
            .to_string();
        for expected in [
            "epoch milliseconds",
            "RFC3339",
            "inclusive",
            "all-time",
            "refuses to overwrite",
            "jsonl",
        ] {
            assert!(export_help.contains(expected), "missing '{expected}' in export help");
        }
    }

    #[test]
    fn help_subcommand_is_available() {
        // clap's built-in `help` subcommand (clig.dev: git-style help access).
        let err = Cli::try_parse_from(["session-manager", "help", "export"]).expect_err("displays help");
        assert!(err.to_string().contains("Usage"), "unexpected: {err}");
    }

    #[test]
    fn epoch_bounds_accept_ms_and_rfc3339() {
        assert_eq!(parse_epoch_ms("1785542400000").expect("ms"), 1_785_542_400_000);
        assert_eq!(
            parse_epoch_ms("2026-08-01T00:00:00Z").expect("rfc3339"),
            1_785_542_400_000
        );
        // Offset-aware RFC3339 converts to absolute epoch ms.
        assert_eq!(
            parse_epoch_ms("2026-08-01T02:00:00+02:00").expect("offset"),
            1_785_542_400_000
        );
        assert!(parse_epoch_ms("august").is_err());
        assert!(parse_epoch_ms("").is_err());
    }

    #[test]
    fn rfc3339_window_parses_end_to_end() {
        let Some(CliCommand::Export { from: Some(from), to: Some(to), .. }) = parse(&[
            "export",
            "--from",
            "2026-08-01T00:00:00Z",
            "--to",
            "2026-08-31T23:59:59Z",
        ])
        .expect("parse")
        else {
            panic!("expected export subcommand");
        };
        assert_eq!(from, 1_785_542_400_000);
        assert_eq!(to, 1_788_220_799_000);
    }

    #[test]
    fn jsonl_format_is_selectable() {
        let Some(CliCommand::Export { format, .. }) =
            parse(&["export", "--days", "1", "--format", "jsonl"]).expect("parse")
        else {
            panic!("expected export subcommand");
        };
        assert_eq!(format, FormatArg::Jsonl);
    }

    #[test]
    fn short_flags_parse_equivalently() {
        // -d/-a/-f/-o mirror their long forms; --no-metadata stays long-only
        // by decision (negative-semantics short flags are cryptic).
        let Some(CliCommand::Export { days, agents, format, no_metadata, out, .. }) =
            parse(&["export", "-d", "7", "-a", "codex", "-f", "jsonl", "-o", "w.json"])
                .expect("parse")
        else {
            panic!("expected export subcommand");
        };
        assert_eq!(days, Some(7));
        assert_eq!(agents, vec![AgentArg::Codex]);
        assert_eq!(format, FormatArg::Jsonl);
        assert!(!no_metadata);
        assert_eq!(out, Some(PathBuf::from("w.json")));

        // Short flags participate in the same validation as long ones.
        let err = parse(&["export", "-d", "7", "--from", "1"]).expect_err("must conflict");
        assert!(err.contains("--days"), "unexpected: {err}");
    }
}
