//! The Playground desktop binary: window loop, egui screens, wiring. Blueprint §1, §14.
//!
//! ```text
//! pg-app                      open the game
//! pg-app --smoke              run a scripted session without a window (CI)
//! options: --data-dir <dir>   where saves and settings live (default: the per-user data folder)
//!          --pack <dir>       also load a content pack (developer use, repeatable)
//!          --no-vsync         do not wait for the display
//!          --shadow <n>       re-run every keyframe span on n threads and compare (developer)
//!          --demo <screen>    open straight on a screen (new, options, ai, saved, game, inspector, pause, overlay)
//! ```

mod app;
mod game_view;
mod os;
mod shell;
mod smoke;
mod ui;

use std::path::PathBuf;
use std::process::ExitCode;

struct Options {
    smoke: bool,
    data_dir: Option<PathBuf>,
    packs: Vec<PathBuf>,
    vsync: bool,
    demo: Option<String>,
    shadow: Option<usize>,
}

fn parse_args() -> Result<Options, String> {
    let mut o = Options {
        smoke: false,
        data_dir: None,
        packs: Vec::new(),
        vsync: true,
        demo: None,
        shadow: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--smoke" => o.smoke = true,
            "--no-vsync" => o.vsync = false,
            "--shadow" => {
                let n = args.next().ok_or("--shadow needs a thread count")?;
                o.shadow = Some(n.parse().map_err(|_| "--shadow needs a number")?);
            }
            "--demo" => o.demo = Some(args.next().ok_or("--demo needs a screen name")?),
            "--data-dir" => {
                o.data_dir = Some(args.next().ok_or("--data-dir needs a folder")?.into())
            }
            "--pack" => o
                .packs
                .push(args.next().ok_or("--pack needs a folder")?.into()),
            "--version" | "-V" => {
                println!("pg-app {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--help" | "-h" => {
                println!(
                    "{}",
                    include_str!("main.rs")
                        .lines()
                        .take(10)
                        .map(|l| l.trim_start_matches("//! ").trim_start_matches("//!"))
                        .collect::<Vec<_>>()
                        .join("\n")
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown option '{other}' (try --help)")),
        }
    }
    Ok(o)
}

fn run() -> Result<(), String> {
    let o = parse_args()?;
    let base = os::find_base_pack()
        .ok_or("the base content (data/base) was not found; run the game from its folder")?;
    let content = app::load_content(&base, &o.packs)?;
    if o.smoke {
        return smoke::run(content);
    }
    let data_dir = o.data_dir.unwrap_or_else(os::default_data_dir);
    let services = app::real_services(&data_dir)?;
    let mut controller = pg_runtime::app::AppController::new(services, Some(content));
    controller.set_shadow_threads(o.shadow);
    let mut app = app::App::new(controller);
    if let Some(d) = &o.demo {
        app.apply_demo(d);
    }
    shell::run(app, o.vsync)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
