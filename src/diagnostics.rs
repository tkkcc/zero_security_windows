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
    let (command, report) = if mode == "--continue" {
        (
            controller::resume_command(&engine)?.unwrap_or(Command::Items(vec![])),
            args.first(),
        )
    } else if matches!(mode, "--apply-items" | "--restore-items") {
        let ids: Vec<String> = serde_json::from_slice(&std::fs::read(
            args.first()
                .ok_or_else(|| anyhow::anyhow!("Missing selection file"))?,
        )?)?;
        let indices = select(&engine, &ids)?;
        (
            if mode == "--restore-items" {
                Command::RestoreItems(indices)
            } else {
                Command::Items(indices)
            },
            args.get(1),
        )
    } else if mode == "--apply-all" {
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
    controller::batch(&engine, &tx, 0, &command, mode == "--continue")?;
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
        Command::Items(indices) | Command::RestoreItems(indices) => indices,
        _ => (0..engine.catalog.len()).collect(),
    };
    let items = states(&engine, &indices);
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
fn states(engine: &Engine, indices: &[usize]) -> Vec<Value> {
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    let results = Mutex::new(vec![Value::Null; indices.len()]);
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..4.min(indices.len()) {
            let (results, next) = (&results, &next);
            scope.spawn(move || {
                loop {
                    let position = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&i) = indices.get(position) else {
                        break;
                    };
                    let f = &engine.catalog[i];
                    let defaults = match engine.default_check(f) {
                        Ok(check) => serde_json::to_value(check).unwrap(),
                        Err(error) => json!({"state":"Unknown","detail":format!("{error:#}")}),
                    };
                    let check = engine.check(f);
                    let result = engine.store.results.lock().unwrap().get(&f.id).cloned();
                    results.lock().unwrap()[position] =
                        json!({"id":f.id,"check":check,"defaults":defaults,"result":result});
                }
            });
        }
    });
    results.into_inner().unwrap()
}
pub(crate) fn select(engine: &Engine, ids: &[String]) -> Result<Vec<usize>> {
    ids.iter()
        .map(|id| {
            engine
                .catalog
                .iter()
                .position(|f| f.id == *id)
                .ok_or_else(|| anyhow::anyhow!("Unknown item: {id}"))
        })
        .collect()
}
pub fn check(args: &[String]) -> Result<()> {
    let engine = Engine::new()?;
    let indices = if let Some(file) = args.get(1) {
        let ids: Vec<String> = serde_json::from_slice(&std::fs::read(file)?)?;
        select(&engine, &ids)?
    } else {
        (0..engine.catalog.len()).collect()
    };
    let items = states(&engine, &indices);
    let report = json!({"version":env!("CARGO_PKG_VERSION"),"boot":engine.boot,"safe_mode":engine.safe,"items":items});
    std::fs::write(
        args.first()
            .ok_or_else(|| anyhow::anyhow!("Missing report file"))?,
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}
