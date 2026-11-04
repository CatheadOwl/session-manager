//! Resume launch: execute a rendered launch-template command line.
//!
//! The template is FREE-FORM and user-authored (settings
//! `sessions.resumeLaunchTemplate`) — the app has NO built-in terminal
//! knowledge. This command only runs the rendered string through the
//! platform shell, detached, with the session's project directory set as
//! the process working directory (`current_dir`) — a plain
//! `powershell -NoExit -Command …` line starts in the project directory
//! WITHOUT any cd; a `wt -d …` / `gnome-terminal …` line may override the
//! directory itself. The GUI process never waits on the spawned line (the
//! terminal outlives this app call).

use std::process::{Child, Command};

/// Spawn `command` through the platform shell. The line reaches the shell
/// byte-for-byte — quoting is the template author's concern, their recipe
/// IS a shell command line by design. `work_dir` becomes the process's
/// working directory (inherited by everything the line spawns).
#[cfg(target_os = "windows")]
fn spawn_shell(command: &str, work_dir: Option<&str>) -> std::io::Result<Child> {
    let mut cmd = Command::new("cmd");
    use std::os::windows::process::CommandExt as _;
    // `cmd /S /D /C "<line>"` is the canonical way to hand cmd an
    // arbitrary pre-built command line: /S strips exactly the outer quote
    // pair we add (inner quotes survive intact), /D skips AutoRun scripts.
    cmd.raw_arg(format!("/S /D /C \"{}\"", command));
    if let Some(dir) = work_dir {
        cmd.current_dir(dir);
    }
    cmd.spawn()
}

#[cfg(not(target_os = "windows"))]
fn spawn_shell(command: &str, work_dir: Option<&str>) -> std::io::Result<Child> {
    let mut cmd = Command::new("sh").arg("-c").arg(command);
    if let Some(dir) = work_dir {
        cmd.current_dir(dir);
    }
    cmd.spawn()
}

/// Launch templates are SINGLE command lines. A hand-edited settings file
/// can still carry newlines (a trailing blank line, a pasted second line)
/// — through `sh -c` a second line would EXECUTE, which is a hazard, not
/// a feature — so this collapses every newline to a space before spawn.
fn collapse_newlines(command: &str) -> String {
    // CRLF first (one separator, not two spaces), then stray CR/LF.
    command.replace("\r\n", " ").replace(['\r', '\n'], " ")
}

/// Run `command` through the platform shell, detached, rooted at
/// `work_dir` (the session's project directory). Errors on the local
/// spawn only (empty line, work dir missing, shell not found); the
/// command's own outcome is the user's terminal's business, not ours.
#[tauri::command]
pub fn run_shell_command(command: String, work_dir: Option<String>) -> Result<(), String> {
    let trimmed = collapse_newlines(&command).trim().to_string();
    if trimmed.is_empty() {
        return Err("Empty launch command".to_string());
    }
    let work_dir = work_dir.filter(|dir| !dir.trim().is_empty());
    if let Some(dir) = &work_dir {
        if !std::path::Path::new(dir).is_dir() {
            return Err(format!("Project directory no longer exists: {dir}"));
        }
    }
    match spawn_shell(&trimmed, work_dir.as_deref()) {
        Ok(mut child) => {
            // Detached: reap the direct child from a throwaway thread so
            // it never lingers as a zombie (terminal windows themselves
            // outlive the direct child on most recipes).
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            Ok(())
        }
        Err(e) => Err(format!("Failed to launch: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "windows")]
    use std::os::windows::process::CommandExt as _;

    #[test]
    fn empty_command_rejected() {
        assert_eq!(
            run_shell_command("   ".to_string(), None),
            Err("Empty launch command".to_string())
        );
        // A newline-only "command" collapses to empty and is rejected too.
        assert_eq!(
            run_shell_command("\r\n  \n".to_string(), None),
            Err("Empty launch command".to_string())
        );
    }

    /// Single-line contract: a hand-edited multi-line template collapses
    /// to one line — the second line must NOT execute (through `sh -c` an
    /// uncollapsed newline would run it for real).
    #[test]
    fn newlines_collapse_to_spaces_no_second_line_executes() {
        assert_eq!(collapse_newlines("a\r\nb\nc"), "a b c");
        assert_eq!(collapse_newlines("plain"), "plain");

        let dir = std::env::temp_dir().join("sm-launch-single-line-test");
        std::fs::create_dir_all(&dir).expect("mkdir");
        let second = dir.join("second.txt");
        let _ = std::fs::remove_file(&second);

        #[cfg(target_os = "windows")]
        let payload = format!(
            "echo FIRST > \"{}\"\r\necho SECOND > \"{}\"",
            dir.join("first.txt").display(),
            second.display()
        );
        #[cfg(not(target_os = "windows"))]
        let payload = format!(
            "echo FIRST > \"{}\"\necho SECOND > \"{}\"",
            dir.join("first.txt").display(),
            second.display()
        );
        assert_eq!(run_shell_command(payload, None), Ok(()));
        assert!(
            !second.exists(),
            "the post-newline line must never execute (collapsed to one line)"
        );
    }

    #[test]
    fn missing_work_dir_rejected() {
        assert!(run_shell_command("echo ok".to_string(), Some("Q:/no/such/dir".to_string()))
            .unwrap_err()
            .starts_with("Project directory no longer exists"));
    }

    #[test]
    fn spawns_a_real_command_and_reaps_it() {
        // echo exits immediately; a successful detached spawn + reap must
        // not block or error.
        assert_eq!(run_shell_command("echo ok".to_string(), None), Ok(()));
    }

    /// The work_dir contract: the spawned shell's process working
    /// directory IS the given directory — no cd in the payload needed.
    /// Asserted by printing the shell's cwd to a file and comparing.
    #[test]
    fn work_dir_becomes_the_shell_cwd() {
        use std::path::PathBuf;

        let work_dir: PathBuf = std::env::temp_dir().join("sm-launch-cwd-test");
        std::fs::create_dir_all(&work_dir).expect("mkdir");
        let out = work_dir.join("cwd.txt");
        let _ = std::fs::remove_file(&out);

        #[cfg(target_os = "windows")]
        let probe = format!("cd > \"{}\"", out.display());
        #[cfg(not(target_os = "windows"))]
        let probe = format!("pwd > \"{}\"", out.display());

        assert_eq!(
            run_shell_command(probe, Some(work_dir.to_string_lossy().to_string())),
            Ok(())
        );
        // Detached spawn: poll for the probe output (the child reaps
        // itself; we have no handle to wait on).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut recorded = String::new();
        while std::time::Instant::now() < deadline {
            if let Ok(text) = std::fs::read_to_string(&out) {
                recorded = text;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(!recorded.is_empty(), "probe must have run");
        // Case-insensitive prefix compare: Windows reports the drive with
        // its own casing and may shorten the path form.
        let recorded = recorded.trim().to_lowercase();
        let expected = work_dir.to_string_lossy().to_lowercase();
        assert!(
            recorded.replace('/', "\\").contains(&expected.replace('/', "\\"))
                || expected.contains(&recorded),
            "shell cwd {recorded:?} should be the work dir {expected:?}"
        );
    }

    /// Empirical discriminator — QUOTED vs UNQUOTED -Command payload through
    /// the exact production spawn path (cmd /S /D /C → pwsh). The payload
    /// deliberately contains a cmd-special character (`|`): code execution
    /// writes pwsh's version to a file, so `Some(version)` means the
    /// payload ran as code; `None` (deadline, no file) means the outer
    /// shell consumed it. Pins WHY the shipped default keeps the payload
    /// quoted (about_PowerShell_exe's standard form).
    #[cfg(target_os = "windows")]
    fn pwsh_command_form_probe(payload_quoted: bool) -> Option<String> {
        use std::path::PathBuf;
        use std::time::{Duration, Instant};

        let dir: PathBuf = std::env::temp_dir().join("sm-pwsh-quote-probe");
        std::fs::create_dir_all(&dir).expect("mkdir");
        let out = dir.join(if payload_quoted { "quoted.txt" } else { "unquoted.txt" });
        let _ = std::fs::remove_file(&out);
        let body = "$PSVersionTable.PSVersion.ToString() | Set-Content -Path '{}'".replace(
            "{}",
            &out.to_string_lossy().replace('\\', "\\\\"),
        );
        let rendered = if payload_quoted {
            format!("pwsh -NoProfile -Command \"{body}\"")
        } else {
            format!("pwsh -NoProfile -Command {body}")
        };
        assert_eq!(run_shell_command(rendered, None), Ok(()));
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if let Ok(text) = std::fs::read_to_string(&out) {
                let text = text.trim().to_string();
                if !text.is_empty() {
                    return Some(text);
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        None
    }

    /// The shipped default's quoting: a QUOTED payload survives the outer
    /// shell and executes as code.
    #[cfg(target_os = "windows")]
    #[test]
    fn pwsh_quoted_command_executes_as_code() {
        let version = pwsh_command_form_probe(true).expect("quoted payload must execute");
        assert!(version.starts_with("7."), "got: {version}");
    }

    /// The counter-case that justifies the quotes: the same payload
    /// UNQUOTED is parsed by the outer cmd wrapper — the `|` splits it at
    /// the outer level and pwsh never runs it. (A plain-token payload like
    /// `codex resume <id>` happens to survive unquoted, which is why both
    /// forms "work" for simple commands; quoting is the robust form.)
    #[cfg(target_os = "windows")]
    #[test]
    fn pwsh_unquoted_command_dies_on_cmd_special_chars() {
        assert!(pwsh_command_form_probe(false).is_none());
    }

    /// THE regression pin: the 0.4.x Windows default once dropped the
    /// outer quotes around the /k payload, which bound `&&` to the OUTER
    /// shell — in a real interactive console the cd ran forever-waiting
    /// and the resume command never did. Discrimination needs an
    /// INTERACTIVE-LIKE stdin: under the test harness a /k shell hits EOF
    /// and exits immediately (letting the outer shell run the tail even
    /// for the broken shape), so this test holds a stdin pipe OPEN. The
    /// quoted shape runs its &&-payload BEFORE reading stdin (file appears
    /// fast); the broken shape parks in an interactive child that waits on
    /// the silent pipe forever (file never appears) — then we kill the
    /// tree and fail.
    #[cfg(target_os = "windows")]
    #[test]
    fn cmd_k_alternative_runs_past_the_ampersand() {
        use std::path::PathBuf;
        use std::process::Stdio;
        use std::time::{Duration, Instant};

        const CREATE_NO_WINDOW: u32 = 0x0800_0000;

        // Pinned to the cmd /k MANUAL alternative explicitly: the
        // &&-binding subtlety lives in that shape, and the shipped default
        // is PowerShell via current_dir (&& never appears).
        let template = crate::session_manager::settings::WINDOWS_CMD_K_ALTERNATIVE;
        let work_dir: PathBuf = std::env::temp_dir().join("sm-launch-template-ampersand-test");
        std::fs::create_dir_all(&work_dir).expect("mkdir");
        let out = work_dir.join("out.txt");
        let _ = std::fs::remove_file(&out);
        let rendered = template
            .replace("{WorkDir}", &work_dir.to_string_lossy().replace('/', "\\"))
            .replace(
                "{ResumeCommand}",
                &format!("echo RESUME-RAN-OK > \"{}\"", out.display()),
            );

        // Same shell wrapping as production, but stdin is a pipe WE hold
        // open — emulating an interactive console that never types.
        let mut cmd = Command::new("cmd");
        cmd.raw_arg(format!("/S /D /C \"{}\"", rendered))
            .stdin(Stdio::piped());
        let mut child = cmd.spawn().expect("spawn");

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut ran = false;
        while Instant::now() < deadline {
            if let Ok(text) = std::fs::read_to_string(&out) {
                if text.trim() == "RESUME-RAN-OK" {
                    ran = true;
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        // The keep-open shell never exits by design — kill the tree.
        let _ = Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .status();
        let _ = child.wait();
        assert!(
            ran,
            "the command after && never ran; rendered template: {rendered}"
        );
    }
}
