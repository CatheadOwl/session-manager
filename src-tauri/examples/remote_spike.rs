//! Remote-source spike (workunit 20260910-1145 phase 0).
//!
//! Validates the russh auth matrix and channel shapes against a real host:
//!   1. ssh-agent on the Windows named pipe (with key-file fallback)
//!   2. exec channel: stat + head (streaming stdout)
//!   3. SFTP subsystem: metadata (size) + read
//!
//! Usage:
//!   cargo run --example remote_spike -- <host> <port> <user> <key-path> <remote-test-file>
//!
//! NOTE: check_server_key accepts any host key (spike-only); the real
//! connection layer must honor known_hosts (see phase 2 spec).
//!
//! Windows-only: the agent probe targets the Windows OpenSSH named pipe
//! (`connect_named_pipe` is cfg(windows) in russh), so this example does not
//! compile on other platforms and is excluded there.

#![cfg(windows)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use russh::keys::agent::client::AgentClient;
use russh::keys::agent::AgentIdentity;
use russh::keys::{PrivateKeyWithHashAlg, load_secret_key};
use russh::{ChannelMsg, client};

struct SpikeHandler;

impl client::Handler for SpikeHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        // spike-only: accept. Real layer: known_hosts check.
        Ok(true)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let host = args.next().expect("host");
    let port: u16 = args.next().expect("port").parse()?;
    let user = args.next().expect("user");
    let key_path = args.next().expect("key path");
    let test_file = args.next().expect("remote test file (absolute path)");

    let config = Arc::new(client::Config {
        inactivity_timeout: Some(Duration::from_secs(30)),
        ..Default::default()
    });

    // --- 1. agent probe (Windows named pipe) ---
    let agent_pipe = r"\\.\pipe\openssh-ssh-agent";
    let mut agent_identities: Vec<AgentIdentity> = Vec::new();
    match AgentClient::connect_named_pipe(agent_pipe).await {
        Ok(mut agent) => match agent.request_identities().await {
            Ok(ids) => {
                println!("[agent] OK: {} identities on {agent_pipe}", ids.len());
                agent_identities = ids;
            }
            Err(e) => println!("[agent] connected but request_identities failed: {e}"),
        },
        Err(e) => println!("[agent] UNAVAILABLE ({agent_pipe}): {e}"),
    }

    // --- 2. connect + auth (agent first, key file fallback) ---
    let t = Instant::now();
    let mut handle = client::connect(config, (host.as_str(), port), SpikeHandler).await?;
    println!("[connect] {} ms (incl. handshake + hostkey)", t.elapsed().as_millis());

    let t = Instant::now();
    let mut authed = false;
    if !agent_identities.is_empty() {
        if let Ok(mut agent) = AgentClient::connect_named_pipe(agent_pipe).await {
            for id in &agent_identities {
                let key = match id {
                    AgentIdentity::PublicKey { key, .. } => key.clone(),
                    AgentIdentity::Certificate { .. } => continue,
                };
                match handle
                    .authenticate_publickey_with(&user, key, None, &mut agent)
                    .await
                {
                    Ok(res) if res.success() => {
                        println!("[auth] agent publickey OK ({} ms)", t.elapsed().as_millis());
                        authed = true;
                        break;
                    }
                    Ok(_) => continue,
                    Err(e) => {
                        println!("[auth] agent attempt error: {e}");
                        break;
                    }
                }
            }
        }
    }
    if !authed {
        let key = load_secret_key(&key_path, None)?;
        let res = handle
            .authenticate_publickey(&user, PrivateKeyWithHashAlg::new(Arc::new(key), None))
            .await?;
        if !res.success() {
            return Err("authentication failed (agent + key file)".into());
        }
        println!("[auth] key-file publickey OK ({} ms)", t.elapsed().as_millis());
    }

    // --- 3. exec channel: stat + head ---
    let t = Instant::now();
    let stat_out = exec_collect(&handle, &format!("stat -c '%s %Y' '{test_file}'")).await?;
    println!(
        "[exec] stat: {} ({} ms)",
        String::from_utf8_lossy(&stat_out).trim(),
        t.elapsed().as_millis()
    );

    let t = Instant::now();
    let head = exec_collect(&handle, &format!("head -c 64 '{test_file}'")).await?;
    println!(
        "[exec] head 64B: {} bytes, first line: {:?} ({} ms)",
        head.len(),
        String::from_utf8_lossy(&head).lines().next().unwrap_or(""),
        t.elapsed().as_millis()
    );

    // --- 4. SFTP: metadata + read ---
    let t = Instant::now();
    let channel = handle.channel_open_session().await?;
    channel.request_subsystem(true, "sftp").await?;
    let sftp = russh_sftp::client::SftpSession::new(channel.into_stream()).await?;
    println!("[sftp] session up ({} ms)", t.elapsed().as_millis());

    let t = Instant::now();
    let meta = sftp.metadata(&test_file).await?;
    println!(
        "[sftp] metadata: size={:?} ({} ms)",
        meta.size,
        t.elapsed().as_millis()
    );

    use tokio::io::AsyncReadExt;
    let mut file = sftp.open(&test_file).await?;
    let mut buf = vec![0u8; 64];
    let t = Instant::now();
    file.read_exact(&mut buf).await?;
    println!(
        "[sftp] read 64B: {:?}... ({} ms)",
        String::from_utf8_lossy(&buf).get(..40).unwrap_or(""),
        t.elapsed().as_millis()
    );

    handle
        .disconnect(russh::Disconnect::ByApplication, "spike done", "")
        .await?;
    println!("[done] all matrix items passed");
    Ok(())
}

async fn exec_collect(
    handle: &client::Handle<SpikeHandler>,
    cmd: &str,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut channel = handle.channel_open_session().await?;
    channel.exec(true, cmd).await?;
    let mut buf = Vec::new();
    while let Some(msg) = channel.wait().await {
        match msg {
            ChannelMsg::Data { ref data } => buf.extend_from_slice(data),
            ChannelMsg::ExitStatus { exit_status } => {
                if exit_status != 0 {
                    return Err(format!("remote exit {exit_status}: {}", String::from_utf8_lossy(&buf)).into());
                }
            }
            _ => {}
        }
    }
    Ok(buf)
}
