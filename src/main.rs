//! oh-my-octos binary. Installed by `octos skills install octos-org/oh-my-octos`
//! as the skill's `main`; also usable on its own for `setup`.
//!
//!   oh-my-octos hook <cost-guard|edit-check|project-context>   (called by Octos; payload on stdin)
//!   oh-my-octos setup [...]                                     (guided install path)
//!   oh-my-octos --version

use oh_my_octos::{hooks, setup};

const HELP: &str = "oh-my-octos — curated defaults for Octos as a coding agent

commands:
  hook <cost-guard|edit-check|project-context>   lifecycle hook (Octos runs these; JSON payload on stdin)
  setup [options]                                 guided install path (run `setup --help`)
  --version";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("hook") => match args.get(1) {
            Some(name) => hooks::run(name),
            None => {
                eprintln!("usage: oh-my-octos hook <cost-guard|edit-check|project-context>");
                2
            }
        },
        Some("setup") => match setup::Options::parse(&args[1..]) {
            Ok(o) => setup::run(&o),
            Err(msg) => {
                println!("{msg}");
                if msg == setup::USAGE { 0 } else { 2 }
            }
        },
        Some("--version") | Some("-V") => {
            println!("oh-my-octos {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Some("-h") | Some("--help") | None => {
            println!("{HELP}");
            0
        }
        Some(other) => {
            eprintln!("unknown command '{other}'\n{HELP}");
            2
        }
    };
    std::process::exit(code);
}
