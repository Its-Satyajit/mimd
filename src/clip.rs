//! System clipboard access.
//!
//! Tries the platform tool that exists (`wl-copy`/`wl-paste` on Wayland,
//! `xclip` on X11) and falls back to mimd's own buffer when neither is
//! installed, so copy/paste always does *something*.

use std::io::Write;
use std::process::{Command, Stdio};

fn tool_exists(bin: &str) -> bool {
    std::path::Path::new(bin).exists() || which(bin)
}

fn which(bin: &str) -> bool {
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            if std::path::Path::new(dir).join(bin).is_file() {
                return true;
            }
        }
    }
    false
}

fn wayland() -> bool {
    std::env::var("WAYLAND_DISPLAY").is_ok()
}

/// Put `text` on the system clipboard. Returns false if unavailable.
pub fn copy(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    let (bin, args): (&str, Vec<&str>) = if wayland() && tool_exists("wl-copy") {
        ("wl-copy", vec!["--type", "text/plain;charset=utf-8", "--"])
    } else if tool_exists("xclip") {
        ("xclip", vec!["-selection", "clipboard", "-in"])
    } else {
        return false;
    };
    let mut child = match Command::new(bin).args(&args).stdin(Stdio::piped()).spawn() {
        Ok(c) => c,
        Err(_) => return false,
    };
    if let Some(stdin) = child.stdin.as_mut() {
        if stdin.write_all(text.as_bytes()).is_err() {
            return false;
        }
    }
    // wl-copy/xclip daemonise to hold the selection; do not wait on them.
    drop(child.stdin.take());
    let _ = child.wait();
    true
}

/// Read the system clipboard. Returns None if unavailable.
pub fn paste() -> Option<String> {
    let out = if wayland() && tool_exists("wl-paste") {
        Command::new("wl-paste")
            .args(["--no-newline"])
            .output()
            .ok()?
    } else if tool_exists("xclip") {
        Command::new("xclip")
            .args(["-selection", "clipboard", "-out"])
            .output()
            .ok()?
    } else {
        return None;
    };
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}
