#![windows_subsystem = "windows"]
mod apps;
mod checks;
mod controller;
mod diagnostics;
mod engine;
mod mitigation;
mod model;
mod native;
mod offline;
mod preferences;
mod recovery;
mod registry;
mod store;
mod tray;
mod tui;
mod workflow;

fn run() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--blocked") => Ok(()),
        Some("--tray") => tray::run(),
        Some("--snapshot") => diagnostics::snapshot(
            args.get(1)
                .ok_or_else(|| anyhow::anyhow!("Missing snapshot path"))?,
        ),
        Some(
            mode @ ("--apply" | "--restore" | "--apply-all" | "--apply-items" | "--restore-items"
            | "--continue"),
        ) => diagnostics::execute(mode, &args[1..]),
        Some("--check") => diagnostics::check(&args[1..]),
        _ => {
            if !native::admin()? {
                native::elevate(&args)?;
                return Ok(());
            }
            if !args.iter().any(|a| a == "--console") && native::modern_terminal(&args)? {
                return Ok(());
            }
            tui::run(
                args.first()
                    .filter(|a| a.as_str() != "--console")
                    .cloned()
                    .unwrap_or_default(),
            )
        }
    }
}
fn main() {
    if let Err(e) = run() {
        let error = format!("{e:#}");
        let _ = store::append("errors.log", &error);
        eprintln!("{error}");
        if !std::env::args().any(|a| {
            matches!(
                a.as_str(),
                "--check"
                    | "--snapshot"
                    | "--apply"
                    | "--restore"
                    | "--apply-all"
                    | "--apply-items"
                    | "--restore-items"
                    | "--continue"
                    | "--tray"
                    | "--blocked"
            )
        }) {
            let _ = native::open_console();
            eprintln!("{error}\n{}", store::root().join("errors.log").display());
            let _ = std::io::stdin().read_line(&mut String::new());
        }
        std::process::exit(1)
    }
}

#[cfg(test)]
mod tests;
