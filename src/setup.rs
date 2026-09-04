//! `oh-my-octos setup`: the guided install path. Runs the existing octos
//! commands in order and checks each step. Stores no keys, writes no config on
//! its own, does not replace `octos init`. Re-running is safe.

use crate::util::{run_with_timeout, which};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

#[derive(Debug, Default, Clone)]
pub struct Options {
    pub profile: Option<String>,
    pub project: Option<PathBuf>,
    pub source: String,
    pub with: Vec<String>,
    pub with_given: bool,
    pub install_octoscode: bool,
    pub show_next: bool,
}

impl Options {
    pub fn parse(args: &[String]) -> Result<Options, String> {
        let mut o = Options {
            source: "octos-org/oh-my-octos".into(),
            install_octoscode: true,
            show_next: true,
            ..Default::default()
        };
        let mut i = 0;
        while i < args.len() {
            let a = args[i].as_str();
            let mut val = || -> Result<String, String> {
                i += 1;
                args.get(i)
                    .cloned()
                    .ok_or_else(|| format!("{a} needs a value"))
            };
            match a {
                "--profile" => o.profile = Some(val()?),
                "--project" => o.project = Some(PathBuf::from(val()?)),
                "--source" => o.source = val()?,
                "--with" => {
                    o.with_given = true;
                    o.with.extend(
                        val()?
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty()),
                    );
                }
                "--no-octoscode" => o.install_octoscode = false,
                "--no-serve" => o.show_next = false,
                "-h" | "--help" => return Err(USAGE.to_string()),
                other => return Err(format!("unknown argument: {other}\n{USAGE}")),
            }
            i += 1;
        }
        Ok(o)
    }
}

pub const USAGE: &str = "usage: oh-my-octos setup [--profile <id>] [--project <dir>] [--source <skill source>]
                        [--with slides,mofa,phonefarm] [--no-octoscode] [--no-serve]

Optional packs (each is one `octos skills install` command; nothing is bundled):
  slides     PPT decks via mofa-slides            (~11 MB; needs GEMINI_API_KEY and the `mofa` CLI)
  mofa       the whole mofa suite, 20 skills      (~540 MB, a few minutes)
  phonefarm  Android / OpenHarmony device automation
In a terminal the command asks which packs you want; with --with, or without a terminal, it does not ask.

Environment:
  OMO_SKIP_BINARY_INSTALL=1   do not try to install octos or octoscode when they are missing";

pub struct Pack {
    pub name: &'static str,
    pub source: &'static str,
    pub blurb: &'static str,
}

pub const PACKS: &[Pack] = &[
    Pack {
        name: "slides",
        source: "mofa-org/mofa-skills/mofa-slides",
        blurb: "PPT decks (mofa-slides, ~11 MB; needs GEMINI_API_KEY and the mofa CLI)",
    },
    Pack {
        name: "mofa",
        source: "mofa-org/mofa-skills",
        blurb: "the whole mofa suite: slides, cards, comics, podcast, pdf, xlsx... (20 skills, ~540 MB)",
    },
    Pack {
        name: "phonefarm",
        source: "BH3GEI/phonefarm/skills/phonefarm",
        blurb: "Android / OpenHarmony device automation and testing",
    },
];

fn step(s: &str) {
    println!("\n==> {s}");
}
fn ok(s: &str) {
    println!("    ok: {s}");
}
fn note(s: &str) {
    println!("    note: {s}");
}
fn fail(s: &str) -> i32 {
    eprintln!("    FAILED: {s}");
    1
}

fn skip_binary_install() -> bool {
    std::env::var("OMO_SKIP_BINARY_INSTALL")
        .map(|v| v == "1")
        .unwrap_or(false)
}

/// Run a command inheriting the terminal (interactive steps like `octos init`).
fn interactive(argv: &[&str]) -> bool {
    Command::new(argv[0])
        .args(&argv[1..])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn capture(argv: &[&str], cwd: Option<&Path>, timeout_s: u64) -> (bool, String) {
    match run_with_timeout(argv, cwd, None, Duration::from_secs(timeout_s)) {
        Ok(r) => (r.status == Some(0), format!("{}{}", r.stdout, r.stderr)),
        Err(e) => (false, e.to_string()),
    }
}

fn home_dir() -> PathBuf {
    if let Some(h) = std::env::var_os("OCTOS_HOME") {
        return PathBuf::from(h);
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".octos")
}

fn config_provider(home: &Path) -> Option<String> {
    for p in [
        home.join("config.json"),
        home.join(".octos").join("config.json"),
    ] {
        if let Ok(text) = std::fs::read_to_string(&p) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                return Some(
                    v.get("provider")
                        .and_then(|p| p.as_str())
                        .unwrap_or("")
                        .to_string(),
                );
            }
        }
    }
    None
}

fn octoscode_target() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-gnu"),
        _ => None,
    }
}

/// Download the octoscode release tarball with curl and unpack the binary into ~/.local/bin.
fn install_octoscode_from_release() -> Result<PathBuf, String> {
    let target = octoscode_target().ok_or("no octoscode release for this platform")?;
    let url = format!(
        "https://github.com/octos-org/octoscode/releases/latest/download/octoscode-{target}.tar.xz"
    );
    let tmp = std::env::temp_dir().join(format!("oh-my-octos-octoscode-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let archive = tmp.join("octoscode.tar.xz");
    let (okd, out) = capture(
        &["curl", "-fsSL", &url, "-o", archive.to_str().unwrap()],
        None,
        300,
    );
    if !okd {
        return Err(format!("download failed: {}", out.trim()));
    }
    let (okx, out) = capture(
        &[
            "tar",
            "-xJf",
            archive.to_str().unwrap(),
            "-C",
            tmp.to_str().unwrap(),
        ],
        None,
        120,
    );
    if !okx {
        return Err(format!("unpack failed: {}", out.trim()));
    }
    let mut found = None;
    fn walk(d: &Path, found: &mut Option<PathBuf>) {
        if let Ok(rd) = std::fs::read_dir(d) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, found);
                } else if p.file_name().and_then(|n| n.to_str()) == Some("octoscode") {
                    *found = Some(p);
                }
            }
        }
    }
    walk(&tmp, &mut found);
    let src = found.ok_or("archive did not contain an `octoscode` binary")?;
    let bin_dir = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(".local")
        .join("bin");
    std::fs::create_dir_all(&bin_dir).map_err(|e| e.to_string())?;
    let dest = bin_dir.join("octoscode");
    std::fs::copy(&src, &dest).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755));
    }
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(dest)
}

pub fn run(o: &Options) -> i32 {
    // 1. octos binary --------------------------------------------------------
    step("octos binary");
    if which("octos").is_none() {
        if skip_binary_install() {
            return fail("octos is not on PATH and OMO_SKIP_BINARY_INSTALL=1");
        }
        match std::env::consts::OS {
            "macos" => {
                if which("brew").is_none() {
                    return fail("Homebrew is required on macOS: https://brew.sh");
                }
                interactive(&[
                    "brew",
                    "tap",
                    "octos-org/octos",
                    "https://github.com/octos-org/octos",
                ]);
                interactive(&["brew", "install", "octos-org/octos/octos"]);
            }
            "linux" => {
                // octos ships its installer as a script; running it is the supported path on Linux.
                let (okd, out) = capture(
                    &[
                        "curl",
                        "-fsSL",
                        "https://github.com/octos-org/octos/releases/latest/download/install.sh",
                        "-o",
                        "/tmp/octos-install.sh",
                    ],
                    None,
                    300,
                );
                if !okd {
                    return fail(&format!(
                        "could not download the octos installer: {}",
                        out.trim()
                    ));
                }
                interactive(&["bash", "/tmp/octos-install.sh"]);
            }
            other => {
                return fail(&format!(
                    "unsupported OS {other}; install octos manually: https://github.com/octos-org/octos#start-here"
                ));
            }
        }
        if which("octos").is_none() {
            return fail("octos still not on PATH after install; open a new shell and re-run");
        }
    }
    let (_, ver) = capture(&["octos", "--version"], None, 30);
    ok(ver.trim());

    // 2. config --------------------------------------------------------------
    step("config");
    let home = home_dir();
    if home.join("config.json").is_file() || home.join(".octos").join("config.json").is_file() {
        ok(&format!("config exists under {}", home.display()));
    } else if std::io::stdin().is_terminal() {
        // `octos init --cwd X` writes X/.octos/config.json and octos reads <OCTOS_HOME>/config.json,
        // so init must run one level above the home dir; that only lines up when it is named `.octos`.
        if home.file_name().and_then(|n| n.to_str()) == Some(".octos") {
            println!(
                "    no config yet; running 'octos init' (interactive: pick a provider and a real model name)"
            );
            let parent = home
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("."));
            interactive(&["octos", "init", "--cwd", parent.to_str().unwrap_or(".")]);
        } else {
            return fail(&format!(
                "no config at {}/config.json; run 'octos init' in the directory that contains it, then re-run",
                home.display()
            ));
        }
    } else {
        return fail("no config and no terminal to run 'octos init'; run it yourself, then re-run");
    }

    // 3. provider credential -------------------------------------------------
    step("provider credential");
    match config_provider(&home) {
        Some(p) if !p.is_empty() => {
            let (_, doctor) = capture(&["octos", "doctor"], None, 120);
            if doctor.contains("API key — resolved") {
                ok(&format!("provider {p}, credential resolves"));
            } else if std::io::stdin().is_terminal() {
                println!("    signing in to {p}");
                if !interactive(&["octos", "auth", "login", "--provider", &p]) {
                    return fail("octos auth login failed");
                }
            } else {
                println!("    provider {p}: sign in later with: octos auth login --provider {p}");
            }
        }
        _ => println!("    no provider in config; 'octos init' sets one"),
    }

    // 4. the skill -----------------------------------------------------------
    step("oh-my-octos skill");
    let mut installed = false;
    if let Some(pid) = &o.profile {
        installed |= interactive(&[
            "octos",
            "skills",
            "--profile",
            pid,
            "install",
            &o.source,
            "--force",
        ]);
    }
    if let Some(dir) = &o.project {
        installed |= Command::new("octos")
            .args(["skills", "install", &o.source, "--force"])
            .current_dir(dir)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
    }
    if o.profile.is_none() && o.project.is_none() {
        println!(
            "    no --profile or --project given; installing into the current directory for 'octos chat'"
        );
        installed |= interactive(&["octos", "skills", "install", &o.source, "--force"]);
    }
    if !installed {
        return fail("skill install did not succeed");
    }
    ok(&format!("installed from {}", o.source));

    // 4b. optional packs -----------------------------------------------------
    step("optional packs");
    let mut with: Vec<String> = o.with.clone();
    if !o.with_given && std::io::stdin().is_terminal() {
        println!("    Also install? Enter numbers separated by spaces, or press Enter to skip.");
        for (i, p) in PACKS.iter().enumerate() {
            println!("      {}) {:<10} {}", i + 1, p.name, p.blurb);
        }
        print!("    > ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        for tok in line.split_whitespace() {
            match tok.parse::<usize>() {
                Ok(n) if n >= 1 && n <= PACKS.len() => with.push(PACKS[n - 1].name.to_string()),
                _ if PACKS.iter().any(|p| p.name == tok) => with.push(tok.to_string()),
                _ => println!("    ignoring '{tok}'"),
            }
        }
    }
    with.sort();
    with.dedup();
    if with.is_empty() {
        ok("none (add later with: octos skills install <source>, sources listed in README.md)");
    }
    for name in &with {
        let Some(pack) = PACKS.iter().find(|p| p.name == name) else {
            println!("    unknown pack '{name}' (known: slides mofa phonefarm)");
            continue;
        };
        println!("    {}: octos skills install {}", pack.name, pack.source);
        let mut okp = false;
        if let Some(pid) = &o.profile {
            okp |= interactive(&[
                "octos",
                "skills",
                "--profile",
                pid,
                "install",
                pack.source,
                "--force",
            ]);
        }
        if let Some(dir) = &o.project {
            okp |= Command::new("octos")
                .args(["skills", "install", pack.source, "--force"])
                .current_dir(dir)
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
        }
        if o.profile.is_none() && o.project.is_none() {
            okp |= interactive(&["octos", "skills", "install", pack.source, "--force"]);
        }
        if !okp {
            println!("    {}: install failed (see above)", pack.name);
        }
        match pack.name {
            "slides" | "mofa" => {
                if std::env::var("GEMINI_API_KEY")
                    .map(|v| v.is_empty())
                    .unwrap_or(true)
                {
                    note(
                        "GEMINI_API_KEY is not set; mofa skills need it (export it, or add it to the profile env)",
                    );
                }
                if which("mofa").is_none() {
                    // The skill's downloaded `main` is the mofa CLI itself; SKILL.md declares `requires_bins: mofa`.
                    let candidates = [o.project.clone(), std::env::current_dir().ok()];
                    let hint = candidates
                        .iter()
                        .flatten()
                        .map(|d| d.join(".octos/skills/mofa-slides/main"))
                        .find(|p| p.is_file());
                    match hint {
                        Some(p) => note(&format!(
                            "put the mofa CLI on PATH, e.g.: mkdir -p ~/.local/bin && ln -sf \"{}\" ~/.local/bin/mofa",
                            p.display()
                        )),
                        None => note(
                            "mofa CLI not on PATH (the mofa-slides skill ships it as <skills dir>/mofa-slides/main; symlink it as 'mofa')",
                        ),
                    }
                }
            }
            "phonefarm" if which("adb").is_none() && which("hdc").is_none() => {
                note(
                    "neither adb nor hdc is on PATH; phonefarm needs one of them to talk to a device",
                );
            }
            _ => {}
        }
    }

    // 4c. octoscode ----------------------------------------------------------
    step("octoscode (terminal client)");
    if let Some(p) = which("octoscode") {
        let (_, v) = capture(&[p.to_str().unwrap_or("octoscode"), "--version"], None, 30);
        ok(v.trim());
    } else if !o.install_octoscode || skip_binary_install() {
        println!(
            "    skipped (install later: brew install octos-org/octoscode/octoscode, or npm install -g @octos-org/octoscode)"
        );
    } else {
        let mut done = false;
        if std::env::consts::OS == "macos" && which("brew").is_some() {
            interactive(&[
                "brew",
                "tap",
                "octos-org/octoscode",
                "https://github.com/octos-org/octoscode",
            ]);
            done = interactive(&["brew", "install", "octos-org/octoscode/octoscode"]);
        }
        if !done {
            match install_octoscode_from_release() {
                Ok(dest) => {
                    ok(&format!("octoscode installed at {}", dest.display()));
                    if which("octoscode").is_none() {
                        note("~/.local/bin is not on PATH; add it to your shell profile");
                    }
                }
                Err(e) => println!(
                    "    octoscode: {e}; install later with: npm install -g @octos-org/octoscode"
                ),
            }
        } else if let Some(p) = which("octoscode") {
            let (_, v) = capture(&[p.to_str().unwrap_or("octoscode"), "--version"], None, 30);
            ok(v.trim());
        }
    }

    // 5. doctor ----------------------------------------------------------------
    step("octos doctor");
    let mut child = Command::new("octos")
        .arg("doctor")
        .stdin(Stdio::null())
        .spawn();
    if let Ok(c) = child.as_mut() {
        let _ = c.wait();
    }

    // 6. next ----------------------------------------------------------------
    step("next");
    if o.show_next {
        println!(
            "    octoscode                     # terminal client (spawns its own octos server)"
        );
        println!("    octos serve --solo            # then open http://localhost:50080");
        println!(
            "    octos chat                    # headless / one-shot, in a project with the skill installed"
        );
    }
    0
}
