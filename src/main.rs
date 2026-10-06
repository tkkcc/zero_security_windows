#![windows_subsystem = "windows"]
mod apps;
mod checks;
mod controller;
mod engine;
mod mitigation;
mod model;
mod native;
mod preferences;
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
        Some("--check") => {
            let engine = std::sync::Arc::new(engine::Engine::new()?);
            let results = std::sync::Mutex::new(vec![None; engine.catalog.len()]);
            let next = std::sync::atomic::AtomicUsize::new(0);
            let started = std::time::Instant::now();
            std::thread::scope(|scope| {
                for _ in 0..4 {
                    let (engine, results, next) = (&engine, &results, &next);
                    scope.spawn(move || loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if i >= engine.catalog.len() { break; }
                        let f = &engine.catalog[i];
                        let check = engine.check(f);
                        results.lock().unwrap()[i] = Some(serde_json::json!({
                            "id": f.id, "name": f.name(engine.zh), "state": check.state, "detail": check.detail
                        }));
                    });
                }
            });
            let report = serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"elapsed_ms":started.elapsed().as_millis(),"safe_mode":engine.safe,"boot":engine.boot,"items":*results.lock().unwrap()});
            let text = serde_json::to_string_pretty(&report)?;
            if let Some(path) = args.get(1) {
                std::fs::write(path, text)?;
            } else {
                println!("{text}")
            }
            Ok(())
        }
        _ => {
            if !native::admin()? {
                native::elevate(&args)?;
                return Ok(());
            }
            tui::run(args.first().cloned().unwrap_or_default())
        }
    }
}
fn main() {
    if let Err(e) = run() {
        let error = format!("{e:#}");
        let _ = store::append("errors.log", &error);
        eprintln!("{error}");
        if !std::env::args().any(|a| a == "--check" || a == "--tray" || a == "--blocked") {
            let _ = native::open_console();
            eprintln!("{error}\n{}", store::root().join("errors.log").display());
            let _ = std::io::stdin().read_line(&mut String::new());
        }
        std::process::exit(1)
    }
}

#[cfg(test)]
mod tests;
