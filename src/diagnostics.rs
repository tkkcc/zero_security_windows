use crate::{
    controller::{self, Command},
    engine::Engine,
    model::*,
    native, workflow,
};
use anyhow::{Result, ensure};
use serde_json::{Value, json};

fn operation_state(engine: &Engine, op: &Operation) -> Value {
    match engine.read(op) {
        Ok(value) => {
            json!({"operation": op, "value": value, "matched": crate::engine::satisfied(op, &value)})
        }
        Err(error) => json!({"operation": op, "error": format!("{error:#}")}),
    }
}
pub fn snapshot(path: &str) -> Result<()> {
    let engine = Engine::new()?;
    let items: Vec<_> = engine.catalog.iter().map(|feature| {
        let targets: Vec<_> = [feature.clone(), feature.restore_target()].iter().map(|target| {
            match engine.expand(target) {
                Ok(ops) => json!({"restoring": target.intent == "Restore", "operations": ops.iter().map(|op| operation_state(&engine, op)).collect::<Vec<_>>()}),
                Err(error) => json!({"restoring": target.intent == "Restore", "error": format!("{error:#}")}),
            }
        }).collect();
        json!({"id": feature.id, "check": engine.check(feature), "targets": targets})
    }).collect();
    std::fs::write(
        path,
        serde_json::to_vec_pretty(&json!({
            "version": env!("CARGO_PKG_VERSION"), "boot": engine.boot, "safe_mode": engine.safe, "items": items,
        }))?,
    )?;
    Ok(())
}
pub fn execute(mode: &str, args: &[String]) -> Result<()> {
    ensure!(native::admin()?, "Administrator rights are required");
    let engine = Engine::new()?;
    let (command, report) = if mode == "--apply-all" {
        (Command::All, args.first())
    } else {
        let id = args
            .first()
            .ok_or_else(|| anyhow::anyhow!("Missing item id"))?;
        let index = engine
            .catalog
            .iter()
            .position(|f| f.id == *id)
            .ok_or_else(|| anyhow::anyhow!("Unknown item: {id}"))?;
        (
            if mode == "--restore" {
                Command::Restore(index)
            } else {
                Command::One(index)
            },
            args.get(1),
        )
    };
    let (tx, rx) = std::sync::mpsc::channel();
    controller::batch(&engine, &tx, 0, &command, false)?;
    drop(tx);
    let processed = rx
        .into_iter()
        .find_map(|message| match message {
            controller::Message::Finished { processed, .. } => Some(processed),
            _ => None,
        })
        .unwrap_or(0);
    let indices: Vec<_> = match command {
        Command::One(i) | Command::Restore(i) => vec![i],
        _ => (0..engine.catalog.len()).collect(),
    };
    let items: Vec<_> = indices.iter().map(|i| {
        let f = &engine.catalog[*i];
        json!({"id": f.id, "check": engine.check(f), "result": engine.store.results.lock().unwrap().get(&f.id).cloned()})
    }).collect();
    let failed = items
        .iter()
        .any(|item| item["check"]["state"] == json!("Failed"));
    let data = json!({"version":env!("CARGO_PKG_VERSION"),"processed":processed,"restart":workflow::requires_restart(&engine),"boot":engine.boot,"items":items});
    if let Some(path) = report {
        std::fs::write(path, serde_json::to_vec_pretty(&data)?)?;
    }
    ensure!(
        !failed,
        "One or more items failed; inspect the operation log"
    );
    Ok(())
}
