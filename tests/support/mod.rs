//! Shared helpers for the e2e tests and the benchmark: an isolated octos home,
//! `octos chat` runs, a UI Protocol stdio driver (`octos serve --stdio --solo`),
//! and a pty runner for the interactive cases.

#![allow(dead_code)]

use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The oh-my-octos binary under test: cargo sets `CARGO_BIN_EXE_*` for integration
/// tests; examples (the benchmark) locate it next to their own executable.
pub fn bin() -> PathBuf {
    if let Some(p) = option_env!("CARGO_BIN_EXE_oh-my-octos") {
        return PathBuf::from(p);
    }
    let exe = std::env::current_exe().expect("current_exe");
    // target/<profile>/examples/bench -> target/<profile>/oh-my-octos
    let profile_dir = exe
        .parent()
        .and_then(|d| {
            if d.file_name().map(|n| n == "examples").unwrap_or(false) {
                d.parent()
            } else {
                Some(d)
            }
        })
        .expect("profile dir");
    let candidate = profile_dir.join("oh-my-octos");
    assert!(
        candidate.is_file(),
        "build the binary first: cargo build (looked for {})",
        candidate.display()
    );
    candidate
}

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Environment the real-octos cases need; `None` = skip with a reason.
pub struct Env {
    pub octos: PathBuf,
    pub octoscode: Option<PathBuf>,
    pub provider: String,
    pub model: String,
    pub key_env: String,
    pub work: PathBuf,
    pub home: PathBuf,
    pub stage: PathBuf,
}

pub fn env() -> Result<Env, String> {
    let octos = std::env::var("OCTOS_BIN")
        .map(PathBuf::from)
        .or_else(|_| which("octos").ok_or(()))
        .map_err(|_| "OCTOS_BIN not set and octos not on PATH".to_string())?;
    if !octos.is_file() {
        return Err(format!("OCTOS_BIN {} is not a file", octos.display()));
    }
    let provider = std::env::var("OMO_E2E_PROVIDER").unwrap_or_else(|_| "deepseek".into());
    let model = std::env::var("OMO_E2E_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".into());
    let key_env = std::env::var("OMO_E2E_KEY_ENV").unwrap_or_else(|_| "DEEPSEEK_API_KEY".into());
    if std::env::var(&key_env)
        .map(|v| v.is_empty())
        .unwrap_or(true)
    {
        return Err(format!("{key_env} is not set"));
    }
    // Short path on purpose: octos serve creates a unix socket under the data dir.
    let root = std::env::var("OMO_E2E_WORK_ROOT").unwrap_or_else(|_| "/tmp".into());
    let work = PathBuf::from(root).join(format!("omo-e2e-{}", std::process::id()));
    let home = work.join("home");
    std::fs::create_dir_all(&home).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(work.join("tmp")).map_err(|e| e.to_string())?;
    std::fs::write(
        home.join("config.json"),
        json!({"provider":provider,"model":model,"api_key_env":key_env}).to_string(),
    )
    .unwrap();
    let stage = stage_skill(&work);
    let octoscode = std::env::var("OCTOSCODE_BIN")
        .map(PathBuf::from)
        .ok()
        .or_else(|| which("octoscode"))
        .filter(|p| p.is_file());
    Ok(Env {
        octos,
        octoscode,
        provider,
        model,
        key_env,
        work,
        home,
        stage,
    })
}

/// Copy of the skill (manifest, prompts, SKILL.md) plus the freshly built binary as
/// `main`, so `octos skills install <stage>` neither downloads nor runs cargo.
pub fn stage_skill(work: &Path) -> PathBuf {
    let stage = work.join("stage").join("oh-my-octos");
    std::fs::create_dir_all(stage.join("prompts")).unwrap();
    let root = repo_root();
    for f in ["manifest.json", "SKILL.md", "README.md", "LICENSE"] {
        let _ = std::fs::copy(root.join(f), stage.join(f));
    }
    for e in std::fs::read_dir(root.join("prompts")).unwrap().flatten() {
        let _ = std::fs::copy(e.path(), stage.join("prompts").join(e.file_name()));
    }
    std::fs::copy(bin(), stage.join("main")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(stage.join("main"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
    }
    stage
}

/// A random v4-style UUID (turn ids must parse as UUIDs). Reads /dev/urandom on unix.
pub fn uuid4() -> String {
    let mut b = [0u8; 16];
    let ok = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .is_ok();
    if !ok {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        b.copy_from_slice(&t.to_le_bytes());
    }
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: Vec<String> = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}{}{}{}-{}{}-{}{}-{}{}-{}{}{}{}{}{}",
        h[0],
        h[1],
        h[2],
        h[3],
        h[4],
        h[5],
        h[6],
        h[7],
        h[8],
        h[9],
        h[10],
        h[11],
        h[12],
        h[13],
        h[14],
        h[15]
    )
}

pub fn which(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

pub struct Run {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

pub fn run(argv: &[&str], cwd: &Path, extra_env: &[(&str, &str)], home: &Path) -> Run {
    let mut cmd = Command::new(argv[0]);
    cmd.args(&argv[1..])
        .current_dir(cwd)
        .env("OCTOS_HOME", home)
        .stdin(Stdio::null());
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn");
    Run {
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into(),
        stderr: String::from_utf8_lossy(&out.stderr).into(),
    }
}

impl Env {
    pub fn tmpdir(&self) -> String {
        self.work.join("tmp").to_string_lossy().into_owned()
    }

    /// Fresh project dir with the skill installed (from the staged copy).
    pub fn new_project(&self, name: &str) -> PathBuf {
        let p = self.work.join(name);
        std::fs::create_dir_all(&p).unwrap();
        let r = run(
            &[
                self.octos.to_str().unwrap(),
                "skills",
                "install",
                self.stage.to_str().unwrap(),
                "--force",
            ],
            &p,
            &[],
            &self.home,
        );
        assert_eq!(
            r.code,
            Some(0),
            "skill install failed in {}:\n{}{}",
            p.display(),
            r.stdout,
            r.stderr
        );
        p
    }

    /// One `octos chat -v --json` turn. Returns (answer text, stderr log).
    pub fn chat(
        &self,
        project: &Path,
        prompt: &str,
        flags: &[&str],
        extra_env: &[(&str, &str)],
    ) -> (String, String, Option<i32>) {
        let mut argv = vec![
            self.octos.to_str().unwrap(),
            "chat",
            "-v",
            "--no-session-persistence",
            "--json",
        ];
        argv.extend_from_slice(flags);
        argv.extend_from_slice(&["-m", prompt]);
        let tmp = self.tmpdir();
        let mut envs = vec![("TMPDIR", tmp.as_str())];
        envs.extend_from_slice(extra_env);
        let r = run(&argv, project, &envs, &self.home);
        std::fs::write(project.join("out.json"), &r.stdout).ok();
        std::fs::write(project.join("err.log"), &r.stderr).ok();
        let answer = serde_json::from_str::<Value>(&r.stdout)
            .ok()
            .and_then(|v| v.get("text").and_then(Value::as_str).map(String::from))
            .unwrap_or_default();
        (answer, r.stderr, r.code)
    }

    pub fn serve_log(&self, home: &Path) -> String {
        let mut s = String::new();
        if let Ok(rd) = std::fs::read_dir(home.join("logs")) {
            let mut files: Vec<_> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .map(|n| n.to_string_lossy().starts_with("serve."))
                        .unwrap_or(false)
                })
                .collect();
            files.sort();
            for f in files {
                s.push_str(&std::fs::read_to_string(f).unwrap_or_default());
            }
        }
        s
    }
}

/// The last `hook executed hook=[".../main", "hook", "<name>"] ...` log line.
pub fn hook_line(log: &str, name: &str) -> Option<String> {
    let needle = format!("\"{name}\"");
    log.lines()
        .rfind(|l| l.contains("hook executed") && l.contains(&needle))
        .map(String::from)
}

pub fn count_hook(log: &str, name: &str, needle: &str) -> usize {
    log.lines()
        .filter(|l| {
            l.contains("hook executed") && l.contains(&format!("\"{name}\"")) && l.contains(needle)
        })
        .count()
}

// ---------------------------------------------------------------- stdio driver

/// Minimal UI Protocol v1 driver over `octos serve --stdio --solo`, mirroring what
/// the arc-bench adapter and octoscode do: create a profile, open a session, run
/// turns, auto-approve.
pub struct Serve {
    child: Child,
    stdin: std::process::ChildStdin,
    responses: Arc<Mutex<HashMap<String, Sender<Value>>>>,
    notifications: Receiver<Value>,
    pub stderr_lines: Arc<Mutex<Vec<String>>>,
    counter: u64,
    pub session_id: String,
    pub profile_id: Option<String>,
    cwd: PathBuf,
}

impl Serve {
    pub fn spawn(octos: &Path, cwd: &Path, home: &Path) -> Serve {
        let mut child = Command::new(octos)
            .args([
                "serve",
                "--stdio",
                "--solo",
                "--data-dir",
                home.to_str().unwrap(),
            ])
            .current_dir(cwd)
            .env("OCTOS_HOME", home)
            .env("RUST_LOG", "info")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn octos serve --stdio");
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let responses: Arc<Mutex<HashMap<String, Sender<Value>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (ntx, nrx) = channel::<Value>();
        let r2 = responses.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let Ok(frame) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if frame.get("method").is_some() {
                    let _ = ntx.send(frame);
                } else if let Some(id) = frame.get("id").map(|v| {
                    v.as_str()
                        .map(String::from)
                        .unwrap_or_else(|| v.to_string())
                }) {
                    if let Some(tx) = r2.lock().unwrap().remove(&id) {
                        let _ = tx.send(frame);
                    }
                }
            }
        });
        let stderr_lines = Arc::new(Mutex::new(Vec::new()));
        let s2 = stderr_lines.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                s2.lock().unwrap().push(line);
            }
        });
        let session_id = format!("omo-e2e:{}", std::process::id());
        Serve {
            child,
            stdin,
            responses,
            notifications: nrx,
            stderr_lines,
            counter: 0,
            session_id,
            profile_id: None,
            cwd: cwd.to_path_buf(),
        }
    }

    fn send(
        &mut self,
        method: &str,
        params: Value,
        want: bool,
        timeout: Duration,
    ) -> Result<Value, String> {
        self.counter += 1;
        let id = format!("rs-{}", self.counter);
        let mut msg = json!({"jsonrpc":"2.0","method":method,"params":params});
        let rx = if want {
            msg["id"] = json!(id);
            let (tx, rx) = channel();
            self.responses.lock().unwrap().insert(id.clone(), tx);
            Some(rx)
        } else {
            None
        };
        writeln!(self.stdin, "{}", msg)
            .map_err(|e| format!("write failed: {e}; stderr tail: {}", self.stderr_tail()))?;
        self.stdin.flush().ok();
        let Some(rx) = rx else { return Ok(Value::Null) };
        let frame = rx.recv_timeout(timeout).map_err(|_| {
            format!(
                "timeout waiting for {method}; stderr tail: {}",
                self.stderr_tail()
            )
        })?;
        if let Some(err) = frame.get("error") {
            return Err(format!("{method} failed: {err}"));
        }
        Ok(frame.get("result").cloned().unwrap_or(Value::Null))
    }

    pub fn stderr_tail(&self) -> String {
        let l = self.stderr_lines.lock().unwrap();
        l.iter()
            .rev()
            .take(8)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Create a solo profile with the given provider/model (serve has no config default).
    pub fn bootstrap_profile(
        &mut self,
        provider: &str,
        model: &str,
        key_env: &str,
    ) -> Result<String, String> {
        let unique = format!("omo-{}-{}", std::process::id(), self.counter);
        let res = self.send("profile/local/create", json!({"requested_id":unique,"name":"oh-my-octos e2e","username":unique,"email":format!("{unique}@solo.local")}), true, Duration::from_secs(60))?;
        let pid = res
            .get("profile_id")
            .and_then(Value::as_str)
            .ok_or("profile/local/create gave no profile_id")?
            .to_string();
        let api_type = if provider == "anthropic" {
            "anthropic"
        } else {
            "openai"
        };
        self.send("profile/llm/upsert", json!({"profile_id":pid,"set_primary":true,"selection":{"family_id":provider,"model_id":model,"route":{"api_type":api_type,"api_key_env":key_env}}}), true, Duration::from_secs(60))?;
        self.profile_id = Some(pid.clone());
        Ok(pid)
    }

    pub fn open(&mut self) -> Result<(), String> {
        let mut params = json!({"session_id":self.session_id,"cwd":self.cwd.to_string_lossy()});
        if let Some(p) = &self.profile_id {
            params["profile_id"] = json!(p);
        }
        self.send("session/open", params, true, Duration::from_secs(120))
            .map(|_| ())
    }

    /// Run one turn; returns (ok, text, methods seen).
    pub fn turn(&mut self, text: &str, timeout: Duration) -> (bool, String, Vec<String>) {
        let turn_id = uuid4();
        if let Err(e) = self.send("turn/start", json!({"session_id":self.session_id,"turn_id":turn_id,"input":[{"kind":"text","text":text}]}), true, Duration::from_secs(60)) {
            return (false, e, vec![]);
        }
        let deadline = Instant::now() + timeout;
        let mut chunks = String::new();
        let mut methods = vec![];
        loop {
            if Instant::now() > deadline {
                return (false, "turn timed out".into(), methods);
            }
            if let Ok(Some(st)) = self.child.try_wait() {
                return (
                    false,
                    format!("octos exited {st}; stderr tail: {}", self.stderr_tail()),
                    methods,
                );
            }
            let Ok(frame) = self.notifications.recv_timeout(Duration::from_secs(5)) else {
                continue;
            };
            let method = frame
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let params = frame.get("params").cloned().unwrap_or(json!({}));
            if method != "server/heartbeat" {
                methods.push(method.clone());
            }
            let same_turn = params.get("turn_id").and_then(Value::as_str) == Some(turn_id.as_str());
            match method.as_str() {
                "message/delta" if same_turn => {
                    chunks.push_str(params.get("text").and_then(Value::as_str).unwrap_or(""))
                }
                "approval/requested" => {
                    let _ = self.send("approval/respond", json!({"approval_id":params.get("approval_id"),"decision":"approve","approval_scope":"request"}), true, Duration::from_secs(30));
                }
                "turn/completed" if same_turn => return (true, chunks, methods),
                "turn/error" if same_turn => {
                    return (
                        false,
                        format!(
                            "{}: {}",
                            params
                                .get("code")
                                .and_then(Value::as_str)
                                .unwrap_or("error"),
                            params.get("message").and_then(Value::as_str).unwrap_or("")
                        ),
                        methods,
                    );
                }
                _ => {}
            }
        }
    }

    pub fn close(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---------------------------------------------------------------- pty runner

/// Run a command in a pseudo-terminal, feeding keystrokes when the screen matches.
pub struct Pty {
    pub master: Box<dyn portable_pty::MasterPty + Send>,
    pub child: Box<dyn portable_pty::Child + Send + Sync>,
    rx: Receiver<Vec<u8>>,
    writer: Box<dyn Write + Send>,
    pub buf: Vec<u8>,
}

pub fn strip_ansi(b: &[u8]) -> String {
    // Drops CSI (ESC [ ... final), OSC (ESC ] ... BEL | ESC \), DCS/APC/PM/SOS
    // (ESC P/_/^/X ... ESC \), two-byte ESC sequences, and carriage returns.
    let s = String::from_utf8_lossy(b);
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c != '\x1b' {
            if c != '\r' {
                out.push(c);
            }
            i += 1;
            continue;
        }
        let next = chars.get(i + 1).copied();
        match next {
            Some('[') => {
                i += 2;
                while i < chars.len() && !('@'..='~').contains(&chars[i]) {
                    i += 1;
                }
                i += 1;
            }
            Some(']') | Some('P') | Some('_') | Some('^') | Some('X') => {
                i += 2;
                while i < chars.len() {
                    if chars[i] == '\x07' {
                        i += 1;
                        break;
                    }
                    if chars[i] == '\x1b' && chars.get(i + 1) == Some(&'\\') {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            }
            Some('(') | Some(')') | Some('*') | Some('+') | Some('#') => i += 3,
            Some(_) => i += 2,
            None => i += 1,
        }
    }
    out
}

impl Pty {
    pub fn spawn(argv: &[&str], cwd: &Path, envs: &[(&str, &str)], rows: u16, cols: u16) -> Pty {
        use portable_pty::{CommandBuilder, PtySize, native_pty_system};
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut cmd = CommandBuilder::new(argv[0]);
        cmd.args(&argv[1..]);
        cmd.cwd(cwd);
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let writer = pair.master.take_writer().unwrap();
        // One reader thread for the life of the pty: a blocking read never wedges a deadline.
        let (tx, rx) = channel::<Vec<u8>>();
        std::thread::spawn(move || {
            let mut chunk = [0u8; 8192];
            loop {
                match reader.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if tx.send(chunk[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        Pty {
            master: pair.master,
            child,
            rx,
            writer,
            buf: Vec::new(),
        }
    }

    /// Pump output until `react(screen, writer)` returns true or the timeout passes.
    /// `react` runs on every update and may type (e.g. answer a menu).
    pub fn pump(
        &mut self,
        timeout: Duration,
        mut react: impl FnMut(&str, &mut dyn Write) -> bool,
    ) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            match self.rx.recv_timeout(Duration::from_millis(300)) {
                Ok(bytes) => self.buf.extend_from_slice(&bytes),
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                Err(_) => {}
            }
            let screen = strip_ansi(&self.buf);
            if react(&screen, &mut self.writer) {
                return true;
            }
            if Instant::now() > deadline || self.child.try_wait().ok().flatten().is_some() {
                break;
            }
        }
        react(&strip_ansi(&self.buf), &mut self.writer)
    }

    pub fn write(&mut self, data: &[u8]) {
        let _ = self.writer.write_all(data);
        let _ = self.writer.flush();
    }

    pub fn screen(&self) -> String {
        strip_ansi(&self.buf)
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
    }
}
