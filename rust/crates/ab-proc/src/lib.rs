//! Argv-only process runner: timeouts, stdout/stderr caps, **no shell**.
//!
//! Never pass a command line string to `cmd` / `sh -c` in production call sites.
//! Callers supply program path + argv only. Used by Codex CLI RPC and Claude CLI probes.

use std::io::{self, Read};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// Default wall-clock timeout for a child process.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// Default per-stream capture cap (2 MiB).
pub const DEFAULT_OUTPUT_CAP: usize = 2 * 1024 * 1024;

/// Outcome of a finished (or timed-out / capped) process run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessOutput {
    pub status_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub timed_out: bool,
    pub output_truncated: bool,
}

impl ProcessOutput {
    pub fn success(&self) -> bool {
        !self.timed_out && self.status_code == Some(0)
    }

    pub fn stdout_lossy(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    pub fn stderr_lossy(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}

/// Run `program` with `args` (argv only — never a shell).
pub fn run(
    program: impl AsRef<std::ffi::OsStr>,
    args: &[impl AsRef<std::ffi::OsStr>],
    timeout: Duration,
    output_cap: usize,
) -> io::Result<ProcessOutput> {
    let env: &[(&str, &str)] = &[];
    run_with_env(program, args, timeout, output_cap, env)
}

/// Like [`run`] with extra env pairs `(key, value)`.
pub fn run_with_env(
    program: impl AsRef<std::ffi::OsStr>,
    args: &[impl AsRef<std::ffi::OsStr>],
    timeout: Duration,
    output_cap: usize,
    env: &[(impl AsRef<std::ffi::OsStr>, impl AsRef<std::ffi::OsStr>)],
) -> io::Result<ProcessOutput> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }

    let mut child = cmd.spawn()?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let (tx_out, rx_out) = mpsc::channel::<(Vec<u8>, bool)>();
    let (tx_err, rx_err) = mpsc::channel::<(Vec<u8>, bool)>();

    if let Some(pipe) = stdout {
        let cap = output_cap;
        thread::spawn(move || {
            let _ = tx_out.send(read_capped(pipe, cap));
        });
    } else {
        let _ = tx_out.send((Vec::new(), false));
    }
    if let Some(pipe) = stderr {
        let cap = output_cap;
        thread::spawn(move || {
            let _ = tx_err.send(read_capped(pipe, cap));
        });
    } else {
        let _ = tx_err.send((Vec::new(), false));
    }

    // Wait with timeout via try_wait poll (portable; no wait_timeout crate).
    let deadline = std::time::Instant::now() + timeout;
    let mut timed_out = false;
    let status_code = loop {
        match child.try_wait()? {
            Some(st) => break st.code(),
            None => {
                if std::time::Instant::now() >= deadline {
                    timed_out = true;
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                thread::sleep(Duration::from_millis(15));
            }
        }
    };

    // Readers exit when pipes close (child dead). Bound join wait.
    let join_deadline = Duration::from_secs(2);
    let (stdout, out_trunc) = rx_out
        .recv_timeout(join_deadline)
        .unwrap_or((Vec::new(), false));
    let (stderr, err_trunc) = rx_err
        .recv_timeout(join_deadline)
        .unwrap_or((Vec::new(), false));

    Ok(ProcessOutput {
        status_code,
        stdout,
        stderr,
        timed_out,
        output_truncated: out_trunc || err_trunc,
    })
}

fn read_capped(mut pipe: impl Read, cap: usize) -> (Vec<u8>, bool) {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    let mut truncated = false;
    loop {
        match pipe.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => {
                let room = cap.saturating_sub(buf.len());
                if room == 0 {
                    truncated = true;
                    continue; // discard remainder until EOF
                }
                let take = n.min(room);
                buf.extend_from_slice(&tmp[..take]);
                if take < n {
                    truncated = true;
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    (buf, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    fn true_cmd() -> (&'static str, Vec<&'static str>) {
        ("cmd", vec!["/C", "exit", "0"])
    }

    #[cfg(not(windows))]
    fn true_cmd() -> (&'static str, Vec<&'static str>) {
        ("true", vec![])
    }

    #[cfg(windows)]
    fn echo_args(msg: &str) -> (&'static str, Vec<String>) {
        ("cmd".into(), vec!["/C".into(), format!("echo {msg}")])
    }

    #[cfg(not(windows))]
    fn echo_args(msg: &str) -> (&'static str, Vec<String>) {
        ("printf".into(), vec!["%s".into(), msg.into()])
    }

    #[cfg(windows)]
    fn sleep_args(secs: u32) -> (&'static str, Vec<String>) {
        // ping -n N+1 waits ~N seconds
        (
            "ping",
            vec!["-n".into(), format!("{}", secs + 1), "127.0.0.1".into()],
        )
    }

    #[cfg(not(windows))]
    fn sleep_args(secs: u32) -> (&'static str, Vec<String>) {
        ("sleep", vec![secs.to_string()])
    }

    #[test]
    fn runs_success_exit_zero() {
        let (prog, args) = true_cmd();
        let out = run(prog, &args, Duration::from_secs(5), DEFAULT_OUTPUT_CAP).unwrap();
        assert!(
            out.success(),
            "timed_out={} code={:?}",
            out.timed_out,
            out.status_code
        );
    }

    #[test]
    fn captures_stdout() {
        let (prog, args) = echo_args("agentbar-hello");
        let out = run(prog, &args, Duration::from_secs(5), DEFAULT_OUTPUT_CAP).unwrap();
        assert!(out.success(), "code={:?}", out.status_code);
        let s = out.stdout_lossy();
        assert!(s.contains("agentbar-hello"), "stdout was: {s:?}");
    }

    #[test]
    fn times_out_long_process() {
        let (prog, args) = sleep_args(30);
        let out = run(prog, &args, Duration::from_millis(300), DEFAULT_OUTPUT_CAP).unwrap();
        assert!(out.timed_out, "expected timeout");
        assert!(!out.success());
    }

    #[test]
    fn respects_output_cap() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("ab_proc_cap_{}.txt", std::process::id()));
        let payload = "x".repeat(10_000);
        std::fs::write(&path, &payload).unwrap();

        #[cfg(windows)]
        let result = run(
            "cmd",
            &["/C", "type", path.to_str().unwrap()],
            Duration::from_secs(5),
            100,
        );
        #[cfg(not(windows))]
        let result = run("cat", &[path.as_os_str()], Duration::from_secs(5), 100);

        let _ = std::fs::remove_file(&path);
        let out = result.unwrap();
        assert!(out.stdout.len() <= 100, "len={}", out.stdout.len());
        assert!(out.output_truncated);
    }
}
