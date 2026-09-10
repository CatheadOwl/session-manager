//! The russh-backed remote session: connect/auth, the batch-metadata
//! exec channel, and SFTP full/incremental fetch into the transient
//! cache.
//!
//! ADR 0007 attribution of the operations here:
//! - connection management = infrastructure (no N×IO shape);
//! - `batch_metadata` = **batch** exit (one exec for N files);
//! - `fetch_to_local` = **cache** exit (full pull once, mtime+size gate);
//! - `fetch_index_incremental` = **batch** exit for append-only files
//!   (codex `session_index.jsonl`: offset read instead of re-transfer).
//!
//! All exec/SFTP calls in the product live in this file (the spike in
//! `examples/remote_spike.rs` is the validated call-shape reference;
//! russh 0.63 ring backend + russh-sftp 3.0).

use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use russh::keys::agent::client::AgentClient;
use russh::keys::agent::AgentIdentity;
use russh::keys::{PrivateKeyWithHashAlg, check_known_hosts, load_secret_key};
use russh::{ChannelMsg, client};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::Mutex;

use super::cache::{self, CacheMeta};
use super::error::RemoteError;
use super::frame::{self, FileMetadataBlob};
use super::RemotePath;
use crate::session_manager::settings::{SourceAuth, SshSource};

/// Named pipe of the Windows OpenSSH agent (the spike-verified path).
#[cfg(windows)]
const WINDOWS_AGENT_PIPE: &str = r"\\.\pipe\openssh-ssh-agent";

/// Outcome of the known_hosts check, recorded by the handler so
/// `connect` can turn russh's opaque `UnknownKey` failure into an
/// actionable variant (unknown host vs changed key).
#[derive(Debug, Clone, PartialEq, Eq)]
enum HostKeyOutcome {
    Accepted,
    Unknown,
    Changed(String),
}

struct RemoteHandler {
    host: String,
    port: u16,
    outcome: Arc<StdMutex<HostKeyOutcome>>,
}

impl client::Handler for RemoteHandler {
    type Error = russh::Error;

    /// Strict known_hosts gate: only a previously recorded matching key
    /// is accepted. Unknown hosts fail with a remedy message; changed
    /// keys fail loudly (possible MITM). Auto-learning (TOFU) is
    /// spike-only behavior and must not reach the product surface.
    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let russh::keys::PublicKeyOrCertificate::PublicKey { key, .. } = server_public_key else {
            // Host certificates are not part of the v1 trust model.
            *self.outcome.lock().expect("hostkey lock") = HostKeyOutcome::Unknown;
            return Ok(false);
        };
        let outcome = match check_known_hosts(&self.host, self.port, key) {
            Ok(true) => HostKeyOutcome::Accepted,
            Ok(false) => HostKeyOutcome::Unknown,
            Err(e) => HostKeyOutcome::Changed(e.to_string()),
        };
        log::debug!(
            "remote: known_hosts check host={} port={} outcome={:?}",
            self.host,
            self.port,
            outcome
        );
        let accepted = outcome == HostKeyOutcome::Accepted;
        *self.outcome.lock().expect("hostkey lock") = outcome;
        Ok(accepted)
    }
}

/// A long-lived, authenticated SSH session to one `SshSource`.
///
/// Every operation retries once through an internal reconnect when the
/// transport dropped mid-call (spec edge case "connection interrupted
/// mid-stream"): a transient network blip is invisible to callers, a
/// persistent one surfaces as `Disconnected` and the UI must offer
/// "reconnect", never "directory empty".
pub struct RemoteSession {
    source: Arc<SshSource>,
    cache_base: PathBuf,
    handle: Mutex<client::Handle<RemoteHandler>>,
}

impl RemoteSession {
    /// Connect + authenticate + hostkey gate. Auth order: agent
    /// identities first (Windows named pipe / Unix SSH_AUTH_SOCK),
    /// key file fallback per `SourceAuth::Key`.
    pub async fn connect(source: &SshSource) -> Result<Self, RemoteError> {
        Self::connect_with_cache(source, cache::default_cache_base()).await
    }

    /// Test seam: same as [`RemoteSession::connect`] with an explicit
    /// cache root (the default resolves `<OS cache dir>`).
    pub async fn connect_with_cache(
        source: &SshSource,
        cache_base: PathBuf,
    ) -> Result<Self, RemoteError> {
        let outcome = Arc::new(StdMutex::new(HostKeyOutcome::Unknown));
        let config = Arc::new(client::Config {
            inactivity_timeout: Some(Duration::from_secs(30)),
            ..Default::default()
        });
        let handler = RemoteHandler {
            host: source.host.clone(),
            port: source.port,
            outcome: outcome.clone(),
        };

        let mut handle = match client::connect(config, (source.host.as_str(), source.port), handler)
            .await
        {
            Ok(handle) => handle,
            Err(e) => {
                // Russh flattens a rejected host key into `UnknownKey`;
                // consult the recorded outcome for the precise variant.
                if matches!(e, russh::Error::UnknownKey) {
                    let variant = outcome.lock().expect("hostkey lock").clone();
                    return Err(match variant {
                        HostKeyOutcome::Changed(detail) => {
                            log::warn!(
                                "remote: host key changed for {}:{} ({detail})",
                                source.host,
                                source.port
                            );
                            RemoteError::HostKeyChanged {
                                host: source.host.clone(),
                                port: source.port,
                            }
                        }
                        _ => RemoteError::HostKeyUnknown {
                            host: source.host.clone(),
                            port: source.port,
                        },
                    });
                }
                return Err(RemoteError::Io(format!(
                    "connect {}:{} failed: {e}",
                    source.host, source.port
                )));
            }
        };

        Self::authenticate(&mut handle, &source.user, &source.auth).await?;

        log::debug!(
            "remote: connected source={} host={}:{} user={}",
            source.id,
            source.host,
            source.port,
            source.user
        );
        Ok(Self {
            source: Arc::new(source.clone()),
            cache_base,
            handle: Mutex::new(handle),
        })
    }

    /// Agent-first authentication with key-file fallback. Every agent
    /// identity is tried (public keys only; certificates skipped, as in
    /// the spike), then the configured key file.
    async fn authenticate(
        handle: &mut client::Handle<RemoteHandler>,
        user: &str,
        auth: &SourceAuth,
    ) -> Result<(), RemoteError> {
        // --- agent pass (platform transport, spike-verified shape) ---
        #[cfg(windows)]
        let agent_ok = Self::try_agent_auth_windows(handle, user).await;
        #[cfg(not(windows))]
        let agent_ok = Self::try_agent_auth_unix(handle, user).await;
        if agent_ok {
            return Ok(());
        }

        // --- key-file fallback ---
        if let SourceAuth::Key { key_path } = auth {
            let key = load_secret_key(key_path, None)
                .map_err(|e| RemoteError::AuthFailed(format!("load key {key_path}: {e}")))?;
            let res = handle
                .authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), None))
                .await
                .map_err(|e| RemoteError::Io(format!("key auth transport error: {e}")))?;
            if res.success() {
                return Ok(());
            }
        }

        Err(RemoteError::AuthFailed(format!(
            "no accepted credential for user `{user}` (agent identities + configured key)"
        )))
    }

    /// Transport liveness probe (used by `RemoteSessionPool` to decide
    /// whether to reuse or rebuild this session). A busy session (lock
    /// held by an in-flight operation) counts as alive.
    pub fn is_closed(&self) -> bool {
        self.handle
            .try_lock()
            .map(|guard| guard.is_closed())
            .unwrap_or(false)
    }

    /// Batch metadata for many files over ONE exec round-trip (ADR 0007
    /// batch exit; wire format in `frame.rs`). Missing remote files are
    /// skipped (`MISS` frames) — the caller's list converges on the
    /// next scan.
    pub async fn batch_metadata(
        &self,
        files: &[RemotePath],
    ) -> Result<Vec<FileMetadataBlob>, RemoteError> {
        if files.is_empty() {
            return Ok(Vec::new());
        }
        let script = frame::build_batch_script(files);
        let stdout = match self.exec_collect(&script).await {
            Ok(out) => out,
            Err(RemoteError::Disconnected) => {
                log::debug!("remote: batch_metadata hit disconnect, retrying once");
                self.reconnect().await?;
                self.exec_collect(&script).await?
            }
            Err(e) => return Err(e),
        };
        frame::parse_batch_stream(&stdout).map_err(|e| {
            log::warn!("remote: batch stream parse failed source={}: {e}", self.source.id);
            RemoteError::Io(format!("batch protocol error: {e}"))
        })
    }

    /// Full-fetch a single remote file into the transient cache (ADR
    /// 0007 cache exit) and return the local path.
    ///
    /// `known_attrs` — the (size, mtime) the caller already holds from
    /// a prior scan/batch. When present, the freshness check skips the
    /// SFTP stat round-trip entirely; only a cache miss pays it.
    pub async fn fetch_to_local(
        &self,
        path: &RemotePath,
        known_attrs: Option<(u64, i64)>,
    ) -> Result<PathBuf, RemoteError> {
        let fetch = |attrs: Option<(u64, i64)>| async move { self.fetch_once(path, attrs).await };
        match fetch(known_attrs).await {
            Err(RemoteError::Disconnected) => {
                log::debug!("remote: fetch_to_local hit disconnect, retrying once path={path}");
                self.reconnect().await?;
                fetch(known_attrs).await
            }
            other => other,
        }
    }

    async fn fetch_once(
        &self,
        path: &RemotePath,
        known_attrs: Option<(u64, i64)>,
    ) -> Result<PathBuf, RemoteError> {
        let (size, mtime) = match known_attrs {
            Some(attrs) => attrs,
            None => {
                let sftp = self.open_sftp().await?;
                let meta = sftp.metadata(path).await.map_err(|e| sftp_err(path, e))?;
                let size = meta.size.ok_or_else(|| {
                    RemoteError::Io(format!("remote stat omitted size for {path}"))
                })?;
                let mtime = systemtime_to_epoch(&meta)
                    .ok_or_else(|| RemoteError::Io(format!("remote stat omitted mtime for {path}")))?;
                (size, mtime)
            }
        };

        if cache::is_fresh(&self.cache_base, &self.source.id, path, size, mtime) {
            log::debug!("remote-cache: fresh hit source={} path={path}", self.source.id);
            return Ok(cache::entry_path(&self.cache_base, &self.source.id, path));
        }

        let sftp = self.open_sftp().await?;
        let mut file = sftp.open(path).await.map_err(|e| sftp_err(path, e))?;
        let mut bytes = Vec::with_capacity(size.min(8 * 1024 * 1024) as usize);
        file.read_to_end(&mut bytes)
            .await
            .map_err(|e| RemoteError::Io(format!("read {path}: {e}")))?;
        if bytes.len() as u64 != size {
            return Err(RemoteError::Io(format!(
                "short read for {path}: got {} of {size} bytes",
                bytes.len()
            )));
        }
        cache::store(
            &self.cache_base,
            &self.source.id,
            path,
            &bytes,
            CacheMeta { size, mtime },
        )
        .map_err(|e| RemoteError::Io(format!("cache store for {path}: {e}")))
    }

    /// Drop the cache entry for a remote path (remote file deleted).
    pub fn invalidate(&self, path: &RemotePath) {
        cache::invalidate(&self.cache_base, &self.source.id, path);
    }

    /// Append-tail incremental read (codex `session_index.jsonl`): read
    /// from `cached_offset` to the current end. Pure read — the caller
    /// owns the offset bookkeeping (cache/merge happens in the phase 3
    /// consumer).
    pub async fn fetch_index_incremental(
        &self,
        path: &RemotePath,
        cached_offset: u64,
    ) -> Result<Vec<u8>, RemoteError> {
        let read = || async { self.read_from_offset(path, cached_offset).await };
        match read().await {
            Err(RemoteError::Disconnected) => {
                log::debug!(
                    "remote: fetch_index_incremental hit disconnect, retrying once path={path}"
                );
                self.reconnect().await?;
                read().await
            }
            other => other,
        }
    }

    async fn read_from_offset(
        &self,
        path: &RemotePath,
        offset: u64,
    ) -> Result<Vec<u8>, RemoteError> {
        let sftp = self.open_sftp().await?;
        let mut file = sftp.open(path).await.map_err(|e| sftp_err(path, e))?;
        file.seek(std::io::SeekFrom::Start(offset))
            .await
            .map_err(|e| RemoteError::Io(format!("seek {path} to {offset}: {e}")))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .await
            .map_err(|e| RemoteError::Io(format!("read {path} from {offset}: {e}")))?;
        Ok(bytes)
    }

    /// Re-establish the transport, discarding the dead handle. The old
    /// handle is replaced only on success so a failed reconnect leaves
    /// the session (harmlessly) holding the dead one.
    async fn reconnect(&self) -> Result<(), RemoteError> {
        let fresh = Self::connect_with_cache(&self.source, self.cache_base.clone()).await?;
        let mut guard = self.handle.lock().await;
        if !guard.is_closed() {
            let _ = guard
                .disconnect(russh::Disconnect::ByApplication, "replaced by reconnect", "")
                .await;
        }
        *guard = fresh_handle_of(fresh);
        Ok(())
    }

    /// Open one SFTP subsystem channel (per operation — cheap relative
    /// to the transfers it serves, and avoids sharing one multiplexed
    /// session across concurrent futures).
    async fn open_sftp(
        &self,
    ) -> Result<russh_sftp::client::SftpSession, RemoteError> {
        let guard = self.handle.lock().await;
        if guard.is_closed() {
            return Err(RemoteError::Disconnected);
        }
        let channel = guard.channel_open_session().await.map_err(|e| {
            if guard.is_closed() {
                RemoteError::Disconnected
            } else {
                log::warn!("remote: SFTP channel refused source={}: {e}", self.source.id);
                RemoteError::ExecUnavailable(format!("sftp channel open: {e}"))
            }
        })?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|_e| RemoteError::Disconnected)?;
        russh_sftp::client::SftpSession::new(channel.into_stream())
            .await
            .map_err(|e| RemoteError::Io(format!("sftp init: {e}")))
    }

    /// Run one exec command and collect stdout (the spike's
    /// `exec_collect` shape). Non-zero exit is an `Io` error carrying
    /// stderr context.
    async fn exec_collect(&self, command: &str) -> Result<Vec<u8>, RemoteError> {
        let guard = self.handle.lock().await;
        if guard.is_closed() {
            return Err(RemoteError::Disconnected);
        }
        let mut channel = guard.channel_open_session().await.map_err(|e| {
            if guard.is_closed() {
                RemoteError::Disconnected
            } else {
                // Connection alive but the exec channel was refused —
                // the restricted-shell edge case (spec edge table).
                log::warn!(
                    "remote: exec channel refused source={}: {e}",
                    self.source.id
                );
                RemoteError::ExecUnavailable(format!("exec channel open: {e}"))
            }
        })?;
        channel
            .exec(true, command)
            .await
            .map_err(|_e| RemoteError::Disconnected)?;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut exit_status: Option<u32> = None;
        while let Some(msg) = channel.wait().await {
            match msg {
                ChannelMsg::Data { ref data } => stdout.extend_from_slice(data),
                ChannelMsg::ExtendedData { ref data, .. } => stderr.extend_from_slice(data),
                ChannelMsg::ExitStatus { exit_status: code } => exit_status = Some(code),
                _ => {}
            }
        }
        match exit_status {
            Some(0) => Ok(stdout),
            Some(code) => Err(RemoteError::Io(format!(
                "remote exec exited {code}: {}",
                String::from_utf8_lossy(&stderr)
            ))),
            // Channel closed without an exit status: the classic
            // signature of the transport dying under the channel.
            None => {
                log::warn!(
                    "remote: exec ended without exit status source={} — treating as disconnect",
                    self.source.id
                );
                Err(RemoteError::Disconnected)
            }
        }
    }
}

/// Extract the live handle from a freshly built session (used by the
/// reconnect path, which swaps it into the shared slot).
fn fresh_handle_of(session: RemoteSession) -> client::Handle<RemoteHandler> {
    // RemoteSession is intentionally move-only; this consumes the
    // short-lived reconnect instance.
    let RemoteSession { handle, .. } = session;
    handle.into_inner()
}

/// Agent identity pass shared by both platform transports: try every
/// plain public key (certificates skipped, spike-validated).
impl RemoteSession {
    async fn try_agent_identities<S>(
        handle: &mut client::Handle<RemoteHandler>,
        user: &str,
        agent: &mut AgentClient<S>,
        ids: Vec<AgentIdentity>,
    ) -> bool
    where
        S: russh::keys::agent::client::AgentStream + Unpin + Send,
    {
        for id in &ids {
            let AgentIdentity::PublicKey { key, .. } = id else {
                continue; // certificates: spike-validated skip
            };
            match handle
                .authenticate_publickey_with(user, key.clone(), None, agent)
                .await
            {
                Ok(res) if res.success() => return true,
                Ok(_) => continue,
                Err(e) => {
                    log::debug!("remote: agent auth attempt error: {e}");
                    break;
                }
            }
        }
        false
    }
}

/// Windows agent pass: OpenSSH agent on its named pipe.
#[cfg(windows)]
impl RemoteSession {
    async fn try_agent_auth_windows(
        handle: &mut client::Handle<RemoteHandler>,
        user: &str,
    ) -> bool {
        let mut agent = match AgentClient::connect_named_pipe(WINDOWS_AGENT_PIPE).await {
            Ok(agent) => agent,
            Err(e) => {
                log::debug!("remote: agent unavailable ({WINDOWS_AGENT_PIPE}): {e}");
                return false;
            }
        };
        let ids = match agent.request_identities().await {
            Ok(ids) => ids,
            Err(e) => {
                log::debug!("remote: agent request_identities failed: {e}");
                return false;
            }
        };
        Self::try_agent_identities(handle, user, &mut agent, ids).await
    }
}

/// Unix agent pass: SSH_AUTH_SOCK (openssh-agent default).
#[cfg(not(windows))]
impl RemoteSession {
    async fn try_agent_auth_unix(
        handle: &mut client::Handle<RemoteHandler>,
        user: &str,
    ) -> bool {
        let mut agent = match AgentClient::connect_env().await {
            Ok(agent) => agent,
            Err(e) => {
                log::debug!("remote: agent unavailable (SSH_AUTH_SOCK): {e}");
                return false;
            }
        };
        let ids = match agent.request_identities().await {
            Ok(ids) => ids,
            Err(e) => {
                log::debug!("remote: agent request_identities failed: {e}");
                return false;
            }
        };
        Self::try_agent_identities(handle, user, &mut agent, ids).await
    }
}

/// Map an SFTP failure to the reaction-shaped `RemoteError`.
fn sftp_err(path: &str, e: russh_sftp::client::error::Error) -> RemoteError {
    use russh_sftp::client::error::Error as SftpError;
    use russh_sftp::protocol::StatusCode;
    if let SftpError::Status(status) = &e {
        if status.status_code == StatusCode::NoSuchFile {
            return RemoteError::NotFound(path.to_string());
        }
    }
    RemoteError::Io(format!("sftp {path}: {e}"))
}

/// SFTP attrs mtime (seconds since epoch) as i64.
fn systemtime_to_epoch(meta: &russh_sftp::protocol::FileAttributes) -> Option<i64> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let modified: SystemTime = meta.modified().ok()?;
    match modified.duration_since(UNIX_EPOCH) {
        Ok(d) => Some(d.as_secs() as i64),
        Err(e) => Some(-(e.duration().as_secs() as i64)),
    }
}

// ---------------------------------------------------------------------------
// Integration smoke test (real host). Ignored by default; run with:
//   cargo test --offline --lib -- --ignored remote_spike_smoke
// Env: REMOTE_SPIKE_HOST, REMOTE_SPIKE_PORT (default 22),
//      REMOTE_SPIKE_USER, REMOTE_SPIKE_KEY (key file path),
//      REMOTE_SPIKE_FILE (absolute remote test file).
// Mirrors examples/remote_spike.rs parameters (phase 0 validation).
// ---------------------------------------------------------------------------
#[cfg(test)]
mod integration {
    use super::*;
    use crate::session_manager::settings::SshSource;

    fn spike_source() -> Option<SshSource> {
        let host = std::env::var("REMOTE_SPIKE_HOST").ok()?;
        let user = std::env::var("REMOTE_SPIKE_USER").ok()?;
        let key = std::env::var("REMOTE_SPIKE_KEY").ok()?;
        let file = std::env::var("REMOTE_SPIKE_FILE").ok()?;
        let mut extra = std::collections::BTreeMap::new();
        extra.insert(
            "testFile".to_string(),
            serde_json::Value::String(file),
        );
        Some(SshSource {
            id: "spike".to_string(),
            label: None,
            host,
            port: std::env::var("REMOTE_SPIKE_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(22),
            user,
            root: "~".to_string(),
            auth: SourceAuth::Key { key_path: key },
            provider_hint: None,
            enabled: true,
            extra,
        })
    }

    fn spike_file(source: &SshSource) -> String {
        source
            .extra
            .get("testFile")
            .and_then(|v| v.as_str())
            .expect("testFile")
            .to_string()
    }

    #[tokio::test]
    #[ignore = "needs a real SSH host (REMOTE_SPIKE_* env vars)"]
    async fn remote_spike_smoke() {
        let Some(source) = spike_source() else {
            panic!("REMOTE_SPIKE_HOST/USER/KEY/FILE not set");
        };
        let file = spike_file(&source);
        let cache_base =
            std::env::temp_dir().join("session-manager-remote-smoke").join(format!(
                "{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis()
            ));

        let session = RemoteSession::connect_with_cache(&source, cache_base.clone())
            .await
            .expect("connect+auth (host must already be in known_hosts)");

        // batch_metadata over one exec round-trip.
        let blobs = session
            .batch_metadata(&[file.clone()])
            .await
            .expect("batch_metadata");
        assert_eq!(blobs.len(), 1, "exactly the one requested file");
        let blob = &blobs[0];
        assert!(blob.size > 0, "test file is non-empty");
        assert_eq!(blob.head.len(), (frame::HEAD_MAX as u64).min(blob.size) as usize);
        assert_eq!(blob.tail.len(), (frame::TAIL_MAX as u64).min(blob.size) as usize);

        // fetch_to_local: fresh miss → transfer; second call → cache hit.
        let local = session
            .fetch_to_local(&file, Some((blob.size, blob.mtime)))
            .await
            .expect("fetch_to_local");
        assert_eq!(
            std::fs::metadata(&local).expect("local meta").len(),
            blob.size,
            "cached file matches remote size"
        );
        let again = session
            .fetch_to_local(&file, Some((blob.size, blob.mtime)))
            .await
            .expect("second fetch");
        assert_eq!(local, again, "cache hit returns the same entry path");

        // Incremental read from the middle returns exactly the suffix.
        if blob.size > 10 {
            let tail = session
                .fetch_index_incremental(&file, blob.size - 10)
                .await
                .expect("incremental");
            assert_eq!(tail.len() as u64, 10);
        }

        // invalidate removes the entry.
        session.invalidate(&file);
        assert!(!local.exists(), "invalidate removes the cache entry");
    }
}
