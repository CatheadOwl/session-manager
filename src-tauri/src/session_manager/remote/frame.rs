//! Binary-safe framing for the batch-metadata exec channel (ADR 0007
//! "batch" exit: one exec round-trip carries stat + head + tail for N
//! files — the P1 real-alias benchmark showed 12-14 ms/file vs ~710 ms
//! for per-file reads at 355 ms RTT).
//!
//! ## Wire protocol (final shape)
//!
//! The remote script emits, per requested file, in scan order:
//!
//! ```text
//! META\t<path>\t<size>\t<mtime>\n   (header line, ASCII fields)
//! <exactly min(HEAD_MAX, size) bytes> (head of the file)
//! <exactly min(TAIL_MAX, size) bytes> (tail of the file)
//! ```
//!
//! or, when the file is missing (deleted between scan and fetch):
//!
//! ```text
//! MISS\t<path>\n
//! ```
//!
//! ## Why this is binary-safe without base64
//!
//! The header announces `size`; the parser therefore knows the exact
//! byte counts of both payload regions (`min(HEAD_MAX, size)` and
//! `min(TAIL_MAX, size)`) BEFORE reading them, and skips over those
//! regions positionally. Head/tail bytes may contain `\n`, `\t`, NUL,
//! quotes — anything: the parser never scans payload bytes for
//! delimiters. The only constraint is on the header line itself: the
//! remote path must be valid UTF-8 and must not contain `\n` or NUL
//! (both impossible-or-pathological for POSIX paths produced by our
//! remote scan; a path containing `\t` is fine because the header is
//! parsed right-to-left).
//!
//! Head/tail overlap when `size < HEAD_MAX + TAIL_MAX` (e.g. size 10000
//! yields head=8192 and tail=the whole 10000 bytes); that matches
//! `head -c` / `tail -c` semantics and costs nothing.

use std::fmt;

/// Maximum head bytes carried per file (matches the P1 benchmark shape).
pub const HEAD_MAX: usize = 8192;
/// Maximum tail bytes carried per file (matches the P1 benchmark shape).
pub const TAIL_MAX: usize = 16384;

/// Per-file metadata blob returned by `RemoteSession::batch_metadata`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMetadataBlob {
    /// Remote absolute path, as requested.
    pub path: String,
    /// Total size in bytes (GNU `stat -c %s` / BSD `stat -f %z` — the
    /// dialect probe lives in [`build_batch_script`]).
    pub size: u64,
    /// mtime in whole seconds since the Unix epoch (GNU `stat -c %Y` /
    /// BSD `stat -f %m`).
    pub mtime: i64,
    /// First `min(HEAD_MAX, size)` bytes.
    pub head: Vec<u8>,
    /// Last `min(TAIL_MAX, size)` bytes.
    pub tail: Vec<u8>,
}

/// Parse/validation failure of a batch stream. `at` is the byte offset
/// where parsing gave up, for diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    /// Stream ended inside a header line or a payload region.
    Truncated { at: usize },
    /// Header line does not have the `KIND\t...\t<num>\t<num>` shape.
    BadHeader { at: usize },
    /// size/mtime fields are not valid integers.
    BadNumber { at: usize },
    /// Header path field is not valid UTF-8.
    NotUtf8Path { at: usize },
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameError::Truncated { at } => {
                write!(f, "batch stream truncated at byte {at}")
            }
            FrameError::BadHeader { at } => {
                write!(f, "malformed batch header at byte {at}")
            }
            FrameError::BadNumber { at } => {
                write!(f, "non-numeric size/mtime in batch header at byte {at}")
            }
            FrameError::NotUtf8Path { at } => {
                write!(f, "non-UTF-8 path in batch header at byte {at}")
            }
        }
    }
}

impl std::error::Error for FrameError {}

/// Single-quote a path for safe embedding in a POSIX shell script.
///
/// Inside single quotes every byte is literal except the closing quote,
/// so `'` is rewritten as `'\''` (close, escaped quote, reopen). The
/// result is safe against spaces, tabs, newlines-as-shell-metachars,
/// `$`, globs, and quotes. NUL cannot occur in a path the remote fs
/// handed us (and would be unrepresentable in a shell word anyway).
pub fn shell_quote(path: &str) -> String {
    let mut quoted = String::with_capacity(path.len() + 2);
    quoted.push('\'');
    for ch in path.chars() {
        if ch == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(ch);
        }
    }
    quoted.push('\'');
    quoted
}

/// Build the remote batch script for the given files. POSIX sh +
/// `stat`/`head`/`tail`/`printf` on the remote side, where `stat` is
/// probed BY BEHAVIOR, never by platform: GNU `-c '%s %Y'` first, BSD
/// `-f '%z %m'` fallback (ADR 0013). Both dialects print
/// `size mtime-seconds` in that order, so the parsing side is
/// dialect-blind. Order matters: `uname`-style branching breaks when a
/// GNU coreutils `stat` shadows the BSD one on macOS (or vice versa),
/// while try-GNU-fail-fallback self-heals in both directions. On
/// either dialect a file that slips past both probes (vanished mid-
/// batch, or an unknown userland) lands in the `|| true` → empty
/// `set --` → MISS guard below — a per-file skip, never wrong data.
/// (`head -c`/`tail -c` with `--` are valid on both GNU and BSD.)
///
/// Per file (see the module docs for the wire format): stat first, then
/// the META header, then exact-count head/tail bytes. A missing file
/// (deleted after scan) emits `MISS\t<path>` so the stream stays
/// parseable and the caller can converge on the next scan.
pub fn build_batch_script(files: &[String]) -> String {
    let mut script = String::new();
    for path in files {
        let quoted = shell_quote(path);
        // `set -- $(stat ...)` splits size/mtime into $1/$2; the guard
        // chain keeps one vanished file from killing the whole batch.
        script.push_str(&format!(
            "f={quoted}; if [ -f \"$f\" ]; then \
             set -- $(stat -c '%s %Y' -- \"$f\" 2>/dev/null || stat -f '%z %m' -- \"$f\" 2>/dev/null || true); \
             if [ $# -eq 2 ]; then \
             printf 'META\\t%s\\t%s\\t%s\\n' \"$f\" \"$1\" \"$2\"; \
             head -c {HEAD_MAX} -- \"$f\"; \
             tail -c {TAIL_MAX} -- \"$f\"; \
             else printf 'MISS\\t%s\\n' \"$f\"; fi; \
             else printf 'MISS\\t%s\\n' \"$f\"; fi; "
        ));
    }
    script
}

/// Parse a complete batch stream (the collected stdout of the batch
/// script). Payload bytes are copied verbatim; no line-splitting ever
/// touches payload regions (see "Why this is binary-safe" above).
pub fn parse_batch_stream(bytes: &[u8]) -> Result<Vec<FileMetadataBlob>, FrameError> {
    let mut blobs = Vec::new();
    let mut pos = 0usize;
    while pos < bytes.len() {
        let nl = find_newline(bytes, pos).ok_or(FrameError::Truncated { at: pos })?;
        let line = std::str::from_utf8(&bytes[pos..nl])
            .map_err(|_| FrameError::NotUtf8Path { at: pos })?;
        let at = pos;
        pos = nl + 1;

        let (kind, _rest) = line.split_once('\t').ok_or(FrameError::BadHeader { at })?;
        if kind == "MISS" {
            // `MISS\t<path>` — path is everything after the first tab
            // (may itself contain tabs).
            continue;
        }
        if kind != "META" {
            return Err(FrameError::BadHeader { at });
        }
        // Right-to-left split: mtime, then size, then path — a path
        // containing tabs cannot confuse the numeric fields. The path
        // still carries the leading `META\t` kind tag; strip it (the
        // kind was already validated above).
        let (head_part, mtime_str) =
            line.rsplit_once('\t').ok_or(FrameError::BadHeader { at })?;
        let (tagged_path, size_str) =
            head_part.rsplit_once('\t').ok_or(FrameError::BadHeader { at })?;
        let path = tagged_path
            .strip_prefix("META\t")
            .ok_or(FrameError::BadHeader { at })?;
        let size: u64 = size_str
            .parse()
            .map_err(|_| FrameError::BadNumber { at })?;
        let mtime: i64 = mtime_str
            .parse()
            .map_err(|_| FrameError::BadNumber { at })?;

        let head_len = (HEAD_MAX as u64).min(size) as usize;
        let tail_len = (TAIL_MAX as u64).min(size) as usize;
        let end = pos
            .checked_add(head_len)
            .and_then(|p| p.checked_add(tail_len))
            .ok_or(FrameError::Truncated { at })?;
        if end > bytes.len() {
            return Err(FrameError::Truncated { at: pos });
        }
        blobs.push(FileMetadataBlob {
            path: path.to_string(),
            size,
            mtime,
            head: bytes[pos..pos + head_len].to_vec(),
            tail: bytes[pos + head_len..end].to_vec(),
        });
        pos = end;
    }
    Ok(blobs)
}

fn find_newline(bytes: &[u8], from: usize) -> Option<usize> {
    bytes[from..]
        .iter()
        .position(|&b| b == b'\n')
        .map(|i| from + i)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a wire-format record exactly like the remote script would.
    fn record(path: &str, content: &[u8], mtime: i64) -> Vec<u8> {
        let size = content.len() as u64;
        let head_len = (HEAD_MAX as u64).min(size) as usize;
        let tail_len = (TAIL_MAX as u64).min(size) as usize;
        let mut out = format!("META\t{path}\t{size}\t{mtime}\n").into_bytes();
        out.extend_from_slice(&content[..head_len]);
        let size_idx = content.len();
        let tail_start = size_idx - tail_len;
        out.extend_from_slice(&content[tail_start..]);
        out
    }

    #[test]
    fn shell_quote_plain_path() {
        assert_eq!(shell_quote("/home/u/a b.jsonl"), "'/home/u/a b.jsonl'");
    }

    #[test]
    fn shell_quote_single_quote_in_path() {
        // close-quote, escaped quote, reopen — the canonical POSIX idiom.
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote("a'b'c"), "'a'\\''b'\\''c'");
    }

    #[test]
    fn shell_quote_metacharacters_stay_literal() {
        let q = shell_quote("$HOME`; rm -rf / # \t*");
        assert!(q.starts_with('\'') && q.ends_with('\''));
        // No unescaped metachar survives: every ' in the middle must be
        // the three-char escape terminator.
        assert!(!q[1..q.len() - 1].contains('\''));
    }

    #[test]
    fn parse_single_small_file_head_equals_tail_equals_content() {
        let content = b"hello world";
        let stream = record("/tmp/s.jsonl", content, 1_700_000_000);
        let blobs = parse_batch_stream(&stream).expect("parse");
        assert_eq!(blobs.len(), 1);
        let b = &blobs[0];
        assert_eq!(b.path, "/tmp/s.jsonl");
        assert_eq!(b.size, 11);
        assert_eq!(b.mtime, 1_700_000_000);
        // size < both limits → head == tail == whole content.
        assert_eq!(b.head, content);
        assert_eq!(b.tail, content);
    }

    #[test]
    fn parse_exact_counts_for_mid_size_file() {
        // size between HEAD_MAX and HEAD_MAX+TAIL_MAX: head is capped,
        // tail spans back over the head region.
        let size = HEAD_MAX + 100;
        let content: Vec<u8> = (0..=255u8).cycle().take(size).collect();
        let stream = record("/tmp/mid.bin", &content, 42);
        let blobs = parse_batch_stream(&stream).expect("parse");
        assert_eq!(blobs[0].head.len(), HEAD_MAX);
        assert_eq!(blobs[0].tail.len(), size); // min(TAIL_MAX, size) == size
        assert_eq!(&blobs[0].head[..], &content[..HEAD_MAX]);
        assert_eq!(blobs[0].tail, content); // whole file
    }

    #[test]
    fn parse_large_file_caps_both_regions() {
        let size = HEAD_MAX + TAIL_MAX + 5000;
        let content: Vec<u8> = (0..=255u8).cycle().take(size).collect();
        let stream = record("/tmp/big.bin", &content, 7);
        let blobs = parse_batch_stream(&stream).expect("parse");
        assert_eq!(blobs[0].head.len(), HEAD_MAX);
        assert_eq!(blobs[0].tail.len(), TAIL_MAX);
        assert_eq!(&blobs[0].head[..], &content[..HEAD_MAX]);
        assert_eq!(blobs[0].tail[..], content[size - TAIL_MAX..]);
    }

    #[test]
    fn parse_binary_safe_payload_with_delimiter_bytes() {
        // head/tail containing \n, \t, NUL, quotes and a fake META line.
        let mut content = Vec::new();
        content.extend_from_slice(b"META\t/x/fake\t1\t2\n");
        content.extend_from_slice(b"line1\nline2\twith\ttabs\x00\x00\xff\xfe");
        content.extend_from_slice(b"quote'\"\\\nMISS\t/not/a/real/miss\n");
        let stream = record("/tmp/'quo ted'\tname.bin", &content, 1);
        let blobs = parse_batch_stream(&stream).expect("parse");
        assert_eq!(blobs.len(), 1);
        // Path with quotes AND a tab round-trips (right-to-left split).
        assert_eq!(blobs[0].path, "/tmp/'quo ted'\tname.bin");
        assert_eq!(blobs[0].head, content);
        assert_eq!(blobs[0].tail, content);
    }

    #[test]
    fn parse_multiple_records_in_order_with_miss() {
        let mut stream = Vec::new();
        stream.extend_from_slice(&record("/a.jsonl", b"aaa", 1));
        stream.extend_from_slice(b"MISS\t/gone.jsonl\n");
        stream.extend_from_slice(&record("/b.jsonl", b"bbb", 2));
        let blobs = parse_batch_stream(&stream).expect("parse");
        let paths: Vec<&str> = blobs.iter().map(|b| b.path.as_str()).collect();
        assert_eq!(paths, vec!["/a.jsonl", "/b.jsonl"]);
        assert_eq!(blobs[1].mtime, 2);
    }

    #[test]
    fn parse_empty_file_yields_empty_head_and_tail() {
        let stream = record("/empty.jsonl", b"", 3);
        let blobs = parse_batch_stream(&stream).expect("parse");
        assert_eq!(blobs[0].size, 0);
        assert!(blobs[0].head.is_empty());
        assert!(blobs[0].tail.is_empty());
    }

    #[test]
    fn parse_empty_stream_yields_no_blobs() {
        assert_eq!(parse_batch_stream(b"").expect("parse"), Vec::new());
    }

    #[test]
    fn parse_truncated_payload_is_an_error() {
        let content = vec![b'x'; 100];
        let mut stream = record("/t.jsonl", &content, 1);
        let full_len = stream.len();
        stream.truncate(full_len - 1);
        // Header consumed; payload region reports short.
        assert!(matches!(
            parse_batch_stream(&stream),
            Err(FrameError::Truncated { .. })
        ));
    }

    #[test]
    fn parse_truncated_header_is_an_error() {
        let mut stream = b"META\t/x\t12".to_vec();
        stream.extend_from_slice(&[b'x'; 12]);
        assert!(matches!(
            parse_batch_stream(&stream),
            Err(FrameError::Truncated { .. })
        ));
    }

    #[test]
    fn parse_bad_number_is_an_error() {
        let stream = b"META\t/x.jsonl\tnotanumber\t123\nabc";
        assert_eq!(
            parse_batch_stream(stream),
            Err(FrameError::BadNumber { at: 0 })
        );
    }

    #[test]
    fn parse_unknown_kind_is_an_error() {
        let stream = b"WAT\t/x\t1\t2\nab";
        assert_eq!(
            parse_batch_stream(stream),
            Err(FrameError::BadHeader { at: 0 })
        );
    }

    #[test]
    fn parse_non_utf8_path_is_an_error() {
        let stream = b"META\t/x\xff\xfe\t1\t2\na";
        assert!(matches!(
            parse_batch_stream(stream),
            Err(FrameError::NotUtf8Path { .. })
        ));
    }

    #[test]
    fn script_quotes_paths_and_uses_benchmarked_window_sizes() {
        let script = build_batch_script(&[
            "/home/u/plain.jsonl".to_string(),
            "/home/u/it's quoted.jsonl".to_string(),
        ]);
        assert!(script.contains("f='/home/u/plain.jsonl';"));
        assert!(script.contains("f='/home/u/it'\\''s quoted.jsonl';"));
        assert!(script.contains("head -c 8192"));
        assert!(script.contains("tail -c 16384"));
        assert!(script.contains("printf 'META\\t%s\\t%s\\t%s\\n'"));
        assert!(script.contains("printf 'MISS\\t%s\\n'"));
    }

    /// ADR 0013: the stat call must be a BEHAVIOR probe — GNU syntax
    /// first, BSD fallback, `|| true` into the MISS guard — so GNU,
    /// BusyBox, and macOS/*BSD hosts all work and a shadowed `stat`
    /// (GNU coreutils installed over the BSD one, or vice versa) still
    /// hits its own dialect branch. No `uname`, no per-platform fork.
    #[test]
    fn script_probes_gnu_stat_first_with_bsd_fallback() {
        let script = build_batch_script(&["/r/a.jsonl".to_string()]);
        assert!(
            script.contains(
                "stat -c '%s %Y' -- \"$f\" 2>/dev/null \
                 || stat -f '%z %m' -- \"$f\" 2>/dev/null || true"
            ),
            "stat must probe GNU then BSD by behavior, then fall to the MISS guard"
        );
    }
}
