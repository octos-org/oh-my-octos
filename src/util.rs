//! Small shared helpers: subprocess with deadline, terminal detection, paths.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Output of a child process that was given a deadline.
pub struct Run {
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

/// Run `argv` in `cwd` with `stdin_data` on stdin, killing it at `timeout`.
pub fn run_with_timeout(
    argv: &[&str],
    cwd: Option<&Path>,
    stdin_data: Option<&[u8]>,
    timeout: Duration,
) -> std::io::Result<Run> {
    let mut cmd = Command::new(argv[0]);
    cmd.args(&argv[1..])
        .stdin(if stdin_data.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    let mut child = cmd.spawn()?;
    if let Some(data) = stdin_data {
        use std::io::Write;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(data);
        }
    }
    // Drain pipes on threads so a chatty child cannot block on a full pipe.
    let mut out = child.stdout.take().unwrap();
    let mut err = child.stderr.take().unwrap();
    let out_t = std::thread::spawn(move || {
        let mut s = Vec::new();
        let _ = out.read_to_end(&mut s);
        s
    });
    let err_t = std::thread::spawn(move || {
        let mut s = Vec::new();
        let _ = err.read_to_end(&mut s);
        s
    });
    let start = Instant::now();
    let mut timed_out = false;
    let status = loop {
        if let Some(st) = child.try_wait()? {
            break st.code();
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            timed_out = true;
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    if timed_out {
        // A killed child may leave grandchildren holding the pipes (e.g. `sh -c "sleep 20"`):
        // do not wait for EOF, the reader threads finish on their own when the pipes close.
        return Ok(Run {
            status,
            stdout: String::new(),
            stderr: String::new(),
            timed_out,
        });
    }
    let stdout = String::from_utf8_lossy(&out_t.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&err_t.join().unwrap_or_default()).into_owned();
    Ok(Run {
        status,
        stdout,
        stderr,
        timed_out,
    })
}

/// Is `name` an executable on PATH?
pub fn which(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(m) = candidate.metadata() {
                    if m.permissions().mode() & 0o111 != 0 {
                        return Some(candidate);
                    }
                }
                continue;
            }
            #[cfg(not(unix))]
            return Some(candidate);
        }
    }
    None
}

/// Parent process id, used as the budget bucket when Octos sends no session id.
pub fn parent_pid() -> u32 {
    #[cfg(unix)]
    {
        std::os::unix::process::parent_id()
    }
    #[cfg(not(unix))]
    {
        0
    }
}
