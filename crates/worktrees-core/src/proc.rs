//! Subprocess helpers that need a HARD deadline.
//!
//! Moved here from the app (`lib.rs::run_deadline`) when a second headless
//! caller needed the same thing: a `claude -p` has no pane to Ctrl-C and no
//! one watching it, so "kill it at N seconds" is the only thing standing
//! between a wedged login profile and a process that runs until the machine is
//! rebooted. Its callers now are the app's headless calls, pi's model probes and
//! guidance's git calls — one answer, the same reason `status_of`'s parse and
//! the profile seam live in core rather than in whichever surface needed them
//! first.

use std::time::Duration;

/// Run `cmd` with piped output and a hard deadline; kill past it.
///
/// ⚠ It kills its DIRECT child and only that — one pid, no process group. A
/// caller that goes through a shell must therefore `exec` the real program
/// (`sh -c "exec claude …"`), or the direct child is `sh` and the timed-out
/// program survives it as an orphan: still burning turns, still holding any MCP
/// servers a profile started.
///
/// Reader threads drain both pipes. Without them a burst over the pipe buffer
/// (~64KB) blocks the child inside `write`, which never exits, which makes
/// `try_wait` return `None` forever — a deadline that fires on a process that
/// cannot die.
pub fn run_deadline(
    cmd: std::process::Command,
    secs: u64,
) -> std::io::Result<std::process::Output> {
    run_deadline_with(cmd, None, secs)
}

/// [`run_deadline`], with `input` fed to the child's stdin — the way to hand a
/// child a SECRET, because its argv is world-readable (`ps -Ao command`) for as
/// long as it runs. Same deadline, same kill, same drained pipes.
///
/// The write happens on its own thread and stdin is closed after it, so a child
/// that reads to EOF sees one, and a child that never reads cannot wedge us on a
/// full pipe: the writer is never joined, and dies with an EPIPE once the child
/// is gone (Rust ignores SIGPIPE).
pub fn run_deadline_stdin(
    cmd: std::process::Command,
    input: Vec<u8>,
    secs: u64,
) -> std::io::Result<std::process::Output> {
    run_deadline_with(cmd, Some(input), secs)
}

fn run_deadline_with(
    mut cmd: std::process::Command,
    input: Option<Vec<u8>>,
    secs: u64,
) -> std::io::Result<std::process::Output> {
    use std::io::{Read, Write};
    use std::process::Stdio;
    let stdin = if input.is_some() { Stdio::piped() } else { Stdio::null() };
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(stdin);
    let mut child = cmd.spawn()?;
    if let (Some(bytes), Some(mut si)) = (input, child.stdin.take()) {
        std::thread::spawn(move || {
            let _ = si.write_all(&bytes);
            // `si` drops here: EOF for the child.
        });
    }
    let so = child.stdout.take();
    let se = child.stderr.take();
    let ho = std::thread::spawn(move || {
        let mut b = Vec::new();
        if let Some(mut s) = so {
            let _ = s.read_to_end(&mut b);
        }
        b
    });
    let he = std::thread::spawn(move || {
        let mut b = Vec::new();
        if let Some(mut s) = se {
            let _ = s.read_to_end(&mut b);
        }
        b
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
    let status = loop {
        if let Some(st) = child.try_wait()? {
            break st;
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("timed out after {secs}s"),
            ));
        }
        std::thread::sleep(Duration::from_millis(120));
    };
    Ok(std::process::Output {
        status,
        stdout: ho.join().unwrap_or_default(),
        stderr: he.join().unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_process_past_the_deadline_is_killed_and_reported_as_timed_out() {
        let mut c = std::process::Command::new("/bin/sh");
        c.args(["-c", "sleep 30"]);
        let e = run_deadline(c, 1).expect_err("must time out");
        assert_eq!(e.kind(), std::io::ErrorKind::TimedOut);
    }

    /// The drain threads are the point: a child that writes more than a pipe
    /// buffer holds would wedge mid-write without them, and the deadline would
    /// fire on a process that cannot exit.
    #[test]
    fn a_burst_larger_than_the_pipe_buffer_is_drained_not_deadlocked() {
        let mut c = std::process::Command::new("/bin/sh");
        c.args(["-c", "i=0; while [ $i -lt 4000 ]; do echo 0123456789012345678901234567890123456789; i=$((i+1)); done"]);
        let out = run_deadline(c, 20).expect("must not deadlock");
        assert!(out.status.success());
        assert!(out.stdout.len() > 64 * 1024, "got {} bytes", out.stdout.len());
    }

    #[test]
    fn stdin_is_fed_and_then_closed() {
        let mut c = std::process::Command::new("/bin/sh");
        c.args(["-c", "cat"]);
        let out = run_deadline_stdin(c, b"secret\n".to_vec(), 10).expect("cat must see EOF and exit");
        assert_eq!(out.stdout, b"secret\n");
    }

    /// A child that never reads stdin, fed more than a pipe buffer holds: the
    /// writer blocks, and the deadline must still fire on the child.
    #[test]
    fn a_child_that_never_reads_stdin_still_dies_at_the_deadline() {
        let mut c = std::process::Command::new("/bin/sh");
        c.args(["-c", "sleep 30"]);
        let t = std::time::Instant::now();
        let e = run_deadline_stdin(c, vec![b'x'; 1 << 20], 1).expect_err("must time out");
        assert_eq!(e.kind(), std::io::ErrorKind::TimedOut);
        assert!(t.elapsed() < Duration::from_secs(10), "took {:?}", t.elapsed());
    }

    #[test]
    fn a_nonzero_exit_is_an_ok_output_not_an_error() {
        let mut c = std::process::Command::new("/bin/sh");
        c.args(["-c", "echo out; echo err >&2; exit 7"]);
        let out = run_deadline(c, 10).unwrap();
        assert_eq!(out.status.code(), Some(7));
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "out");
        assert_eq!(String::from_utf8_lossy(&out.stderr).trim(), "err");
    }
}
