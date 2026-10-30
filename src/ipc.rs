//! Tiny request/reply messaging between TimeSlay processes.
//!
//! The side panel, the dock and the shortcut daemon are all this one binary
//! started with different flags, so they talk over Unix sockets in the runtime
//! directory. A request is one word; the reply is one word.

use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

/// The side panel's socket: `show`, `hide`, `status` and JSON inbox requests.
pub const SIDEBAR: &str = "timeslay-sidebar.sock";
/// The dock's socket: `hide`, `status` and JSON inbox requests.
pub const DOCK: &str = "timeslay-dock.sock";

/// What `status` answers while the window or strip is on screen.
pub const VISIBLE: &str = "visible";
/// What `status` answers while it is not.
pub const HIDDEN: &str = "hidden";
/// What every other command answers.
pub const OK: &str = "ok";

/// How long a peer gets to answer before it is treated as gone.
const TIMEOUT: Duration = Duration::from_secs(2);

/// The largest request a peer may send. Inbox requests carry a page title and,
/// for Slack, the text of a message; a megabyte is far more than any of them.
const MAX_REQUEST: u64 = 1024 * 1024;

/// The path of a socket, preferring the XDG runtime directory.
pub fn path(name: &str) -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(name)
}

/// Sends one command and returns the reply. Fails when nothing is listening.
pub fn request(name: &str, command: &str) -> Result<String> {
    request_with_timeout(name, command, TIMEOUT)
}

/// Like [`request`], for work that may take longer than a visibility check:
/// adding a task waits on Google's API, which can take a few seconds.
pub fn request_with_timeout(name: &str, command: &str, timeout: Duration) -> Result<String> {
    let path = path(name);
    let mut stream = UnixStream::connect(&path)
        .with_context(|| format!("nothing is listening on {}", path.display()))?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    stream.write_all(command.as_bytes())?;
    stream.shutdown(Shutdown::Write)?;

    let mut reply = String::new();
    stream.read_to_string(&mut reply)?;
    Ok(reply.trim().to_owned())
}

/// Whether the peer behind `name` is running and on screen.
pub fn is_visible(name: &str) -> bool {
    request(name, "status").is_ok_and(|reply| reply == VISIBLE)
}

/// Polls until `ready` holds or `timeout` passes.
pub fn wait_until(timeout: Duration, mut ready: impl FnMut() -> bool) -> bool {
    let started = Instant::now();
    loop {
        if ready() {
            return true;
        }
        if started.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// Binds the socket, replacing a stale one left by a process that crashed.
pub fn bind(name: &str) -> Result<UnixListener> {
    let path = path(name);
    let _ = std::fs::remove_file(&path);
    UnixListener::bind(&path).with_context(|| format!("failed to bind {}", path.display()))
}

/// Removes the socket file, for a process that is about to exit.
pub fn unbind(name: &str) {
    let _ = std::fs::remove_file(path(name));
}

/// Answers requests on `listener` until it fails, one at a time.
pub fn serve(listener: UnixListener, mut handle: impl FnMut(&str) -> String) -> Result<()> {
    for stream in listener.incoming() {
        let mut stream = stream.context("failed to accept a connection")?;
        let _ = stream.set_read_timeout(Some(TIMEOUT));
        let _ = stream.set_write_timeout(Some(TIMEOUT));

        let mut command = String::new();
        if let Err(error) = (&stream).take(MAX_REQUEST).read_to_string(&mut command) {
            log::warn!("failed to read a command: {error}");
            continue;
        }
        let command = command.trim();
        if command.is_empty() {
            continue;
        }

        let reply = handle(command);
        let _ = stream.write_all(reply.as_bytes());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_gets_the_reply_of_the_server() {
        let name = format!("timeslay-test-{}.sock", std::process::id());
        let listener = bind(&name).unwrap();
        std::thread::spawn(move || {
            let _ = serve(listener, |command| match command {
                "status" => VISIBLE.to_owned(),
                _ => OK.to_owned(),
            });
        });

        assert!(is_visible(&name));
        assert_eq!(request(&name, "hide").unwrap(), OK);
        unbind(&name);
        assert!(request(&name, "status").is_err());
    }
}
