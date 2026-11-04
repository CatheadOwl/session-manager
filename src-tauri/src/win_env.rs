//! Registry-backed PATH rebuild for spawned shells (`win_env` — Windows only).
//!
//! A child process inherits THIS app's environment block as it was at
//! launch time. Two failure modes hide in that:
//! - Degraded launcher: the app itself may be started by an entry point
//!   that only propagates the machine PATH (remote-desktop helpers,
//!   elevation, some launchers) — user-PATH tools (e.g. npm-installed
//!   CLIs) vanish for everything we spawn.
//! - Stale snapshot: tools installed while the app is running never
//!   appear in its frozen env, while a freshly opened terminal (built
//!   from the registry via the shell's parent) sees them.
//!
//! Resume launch's user contract is "like typing it in a terminal I just
//! opened", so spawn composes the child's PATH from the source of truth a
//! new terminal gets it from: the machine and user registry `Path` values
//! (HKLM Session Manager\Environment first, HKCU\Environment appended —
//! the same order `CreateEnvironmentBlock` uses, no dedup). Other
//! variables are NOT rebuilt: PATH is where this observably breaks, and
//! the command line itself stays byte-for-byte the template author's.
//!
//! Expansion caveat: `RegGetValueW` expands REG_EXPAND_SZ entries against
//! the CALLER's environment. Degraded launchers keep the user's scalar
//! variables (%USERPROFILE% et al) even when they drop the user PATH, so
//! expansion stays correct in the observed failure mode.
//!
//! Failure policy: a failed registry read skips that side; if BOTH fail,
//! `native_path()` returns None and callers fall back to plain
//! inheritance — the repair must never be the reason a launch fails.

use std::ffi::c_void;

use windows::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    RegGetValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ,
};
use windows::core::{PCWSTR, w};

/// Machine `Path`: HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment.
const MACHINE_SUBKEY: PCWSTR = w!("SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment");
/// User `Path`: HKCU\Environment.
const USER_SUBKEY: PCWSTR = w!("Environment");
const PATH_VALUE: PCWSTR = w!("Path");

/// Start big enough for typical PATHs; `ERROR_MORE_DATA` grows the buffer.
const INITIAL_BUFFER_BYTES: u32 = 8192;

/// The PATH a freshly opened terminal would see: machine `Path`, then the
/// user's, joined with `;`. `None` when neither registry value could be
/// read — callers should keep plain env inheritance in that case.
pub(crate) fn native_path() -> Option<String> {
    let machine = read_path(HKEY_LOCAL_MACHINE, MACHINE_SUBKEY);
    let user = read_path(HKEY_CURRENT_USER, USER_SUBKEY);
    match (machine, user) {
        (Some(m), Some(u)) => Some(format!("{m};{u}")),
        (one, other) => one.or(other),
    }
}

/// Read and expand one registry `Path` value. Empty counts as absent.
fn read_path(root: HKEY, subkey: PCWSTR) -> Option<String> {
    let mut capacity = INITIAL_BUFFER_BYTES;
    loop {
        let mut buffer = vec![0u8; capacity as usize];
        let mut written = capacity;
        let err = unsafe {
            RegGetValueW(
                root,
                subkey,
                PATH_VALUE,
                RRF_RT_REG_SZ,
                None,
                Some(buffer.as_mut_ptr() as *mut c_void),
                Some(&mut written),
            )
        };
        match err {
            // RRF_RT_REG_SZ (no RRF_NOEXPAND) already expanded REG_EXPAND_SZ.
            ERROR_SUCCESS => {
                let pairs = &buffer[..written as usize];
                let units: Vec<u16> = pairs
                    .chunks_exact(2)
                    .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                    .collect();
                let value = String::from_utf16_lossy(&units);
                let value = value.trim_end_matches('\0');
                return (!value.is_empty()).then(|| value.to_string());
            }
            ERROR_MORE_DATA => capacity = written,
            _ => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The composition contract, verified against the same registry the
    /// production reads use: machine first, user appended, `;`-joined.
    /// (The reader itself is exercised through `native_path`; a direct
    /// machine+user compare would just re-run its own code.)
    #[test]
    fn native_path_has_both_sides_when_available() {
        let native = native_path().expect("machine and user Path both exist on real Windows");
        let machine = read_path(HKEY_LOCAL_MACHINE, MACHINE_SUBKEY);
        let user = read_path(HKEY_CURRENT_USER, USER_SUBKEY);
        if let Some(machine) = machine {
            assert!(native.starts_with(&machine), "machine Path must come first");
        }
        if let Some(user) = user {
            let suffix = format!(";{user}");
            assert!(native.ends_with(&suffix), "user Path must be appended last");
        }
    }
}
