use crate::{engine::Engine, model::*, store, workflow};
use anyhow::Result;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
    mpsc::{Receiver, Sender},
};

#[derive(Clone, Debug)]
pub enum Command {
    One(usize),
    Items(Vec<usize>),
    All,
    Restart,
    Scan(Vec<usize>),
}
pub enum Message {
    Ready(Arc<Engine>),
    State(usize, Check, usize),
    Queued(Vec<usize>),
    Finished {
        processed: usize,
        restart: bool,
        auto: bool,
        refresh: Vec<usize>,
    },
    Error(String),
}
pub fn order(catalog: &[Feature]) -> Vec<usize> {
    let mut indices: Vec<_> = (0..catalog.len()).collect();
    indices.sort_by_key(|i| {
        let f = &catalog[*i];
        if f.page == "Install" {
            2
        } else if f.security() || workflow::QUIET.contains(&f.id.as_str()) {
            0
        } else {
            1
        }
    });
    indices
}
pub fn launch(
    tx: Sender<Message>,
    commands: Receiver<Command>,
    epoch: Arc<AtomicUsize>,
    mode: String,
) {
    std::thread::spawn(move || {
        let result = (|| -> Result<()> {
            let engine = Arc::new(Engine::new()?);
            let _ = tx.send(Message::Ready(engine.clone()));
            let automatic = !mode.is_empty();
            if mode == "--safe-resume" {
                let version = epoch.fetch_add(1, Ordering::SeqCst) + 1;
                let all = workflow::safe_resume(&engine)?;
                let command = if all {
                    Command::All
                } else {
                    let pending = engine.store.pending.lock().unwrap();
                    Command::Items(
                        engine
                            .catalog
                            .iter()
                            .enumerate()
                            .filter(|(_, f)| pending.as_ref().unwrap().safe_ids.contains(&f.id))
                            .map(|(i, _)| i)
                            .collect(),
                    )
                };
                batch(&engine, &tx, version, &command, true)?;
            } else if mode == "--resume" {
                let (all, ids) = workflow::normal_resume(&engine)?;
                if all || !ids.is_empty() {
                    let version = epoch.fetch_add(1, Ordering::SeqCst) + 1;
                    let command = if all {
                        Command::All
                    } else {
                        Command::Items(
                            engine
                                .catalog
                                .iter()
                                .enumerate()
                                .filter(|(_, f)| ids.contains(&f.id))
                                .map(|(i, _)| i)
                                .collect(),
                        )
                    };
                    batch(&engine, &tx, version, &command, automatic)?;
                } else {
                    let _ = tx.send(Message::Finished {
                        processed: 0,
                        restart: workflow::requires_restart(&engine),
                        auto: true,
                        refresh: vec![],
                    });
                }
            }
            if !automatic {
                checks(
                    engine.clone(),
                    tx.clone(),
                    epoch.clone(),
                    (0..engine.catalog.len()).collect(),
                );
            }
            for command in commands {
                if let Command::Scan(indices) = command {
                    engine.begin_batch();
                    checks(engine.clone(), tx.clone(), epoch.clone(), indices);
                    continue;
                }
                if matches!(command, Command::Restart) {
                    if let Err(e) = workflow::restart() {
                        let _ = tx.send(Message::Error(format!("{e:#}")));
                    }
                    continue;
                }
                let version = epoch.fetch_add(1, Ordering::SeqCst) + 1;
                if let Err(e) = batch(&engine, &tx, version, &command, false) {
                    let _ = tx.send(Message::Error(format!("{e:#}")));
                }
            }
            epoch.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })();
        if let Err(e) = result {
            let _ = tx.send(Message::Error(format!("{e:#}")));
        }
    });
}
fn checks(engine: Arc<Engine>, tx: Sender<Message>, epoch: Arc<AtomicUsize>, indices: Vec<usize>) {
    if indices.is_empty() {
        return;
    }
    let indices = Arc::new(indices);
    let next = Arc::new(AtomicUsize::new(0));
    let version = epoch.load(Ordering::SeqCst);
    for _ in 0..4 {
        let (engine, tx, next, epoch, indices) = (
            engine.clone(),
            tx.clone(),
            next.clone(),
            epoch.clone(),
            indices.clone(),
        );
        std::thread::spawn(move || {
            loop {
                if epoch.load(Ordering::SeqCst) != version {
                    break;
                }
                let position = next.fetch_add(1, Ordering::Relaxed);
                let Some(&i) = indices.get(position) else {
                    break;
                };
                let f = &engine.catalog[i];
                let _ = tx.send(Message::State(i, Check::new(Status::Checking), version));
                let check = engine.check(f);
                if epoch.load(Ordering::SeqCst) == version
                    && tx.send(Message::State(i, check, version)).is_err()
                {
                    break;
                }
            }
        });
    }
}
fn affected(engine: &Engine, command: &Command) -> Vec<usize> {
    let touched: Vec<&Feature> = match command {
        Command::One(i) => vec![&engine.catalog[*i]],
        _ => return vec![],
    };
    engine
        .catalog
        .iter()
        .enumerate()
        .filter(|(_, f)| {
            touched.iter().any(|t| {
                t.group_zh == f.group_zh && matches!(t.group_zh.as_str(), "Defender" | "核心隔离")
                    || t.ops.iter().any(|a| {
                        f.ops.iter().any(|b| {
                            a.kind == b.kind && matches!(a.kind.as_str(), "Apps" | "TaskGroup")
                                || a.path == b.path && a.name == b.name && !a.path.is_empty()
                        })
                    })
            })
        })
        .map(|(i, _)| i)
        .collect()
}
fn publish(engine: &Engine, tx: &Sender<Message>, i: usize, version: usize) {
    let _ = tx.send(Message::State(i, engine.check(&engine.catalog[i]), version));
}
fn failure(
    engine: &Engine,
    tx: &Sender<Message>,
    i: usize,
    version: usize,
    error: anyhow::Error,
) -> Result<()> {
    let detail = format!("{error:#}");
    let f = &engine.catalog[i];
    engine.store.results.lock().unwrap().insert(
        f.id.clone(),
        ResultRecord {
            errors: vec![detail.clone()],
            boot: engine.boot,
            ..Default::default()
        },
    );
    engine.store.log(
        &f.id,
        &Operation {
            kind: "Execute".into(),
            ..Default::default()
        },
        &detail,
    )?;
    engine.store.save()?;
    let _ = tx.send(Message::State(
        i,
        Check {
            state: Status::Failed,
            detail,
        },
        version,
    ));
    Ok(())
}
fn perform(
    engine: &Engine,
    tx: &Sender<Message>,
    i: usize,
    version: usize,
    toggle: bool,
) -> Result<()> {
    let f = &engine.catalog[i];
    let _ = tx.send(Message::State(i, Check::new(Status::Running), version));
    let win11 = toggle && f.toggle() && crate::registry::exists(&f.ops[0].path)?;
    let settings_only = engine.safe;
    if let Err(error) = engine.execute(f, win11, false, settings_only) {
        return failure(engine, tx, i, version, error);
    }
    publish(engine, tx, i, version);
    Ok(())
}
const PARALLELISM: usize = 6;
fn parallel_jobs(indices: &[usize], run: impl Fn(usize) -> Result<bool> + Sync) -> Result<usize> {
    let next = AtomicUsize::new(0);
    let processed = AtomicUsize::new(0);
    let errors = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..indices.len().min(PARALLELISM) {
            scope.spawn(|| {
                loop {
                    let position = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&i) = indices.get(position) else {
                        break;
                    };
                    match run(i) {
                        Ok(true) => {
                            processed.fetch_add(1, Ordering::Relaxed);
                        }
                        Ok(false) => {}
                        Err(error) => errors.lock().unwrap().push(format!("{error:#}")),
                    }
                }
            });
        }
    });
    let errors = errors.into_inner().unwrap();
    anyhow::ensure!(errors.is_empty(), "{}", errors.join("\n"));
    Ok(processed.load(Ordering::Relaxed))
}
fn prerequisite(engine: &Engine, tx: &Sender<Message>, f: &Feature, version: usize) -> Result<()> {
    let id = match f.id.as_str() {
        "input-method" => "install-tencent.wetype",
        _ => return Ok(()),
    };
    if let Some(i) = engine.catalog.iter().position(|f| f.id == id)
        && engine.check(&engine.catalog[i]).actionable()
    {
        perform(engine, tx, i, version, false)?;
        anyhow::ensure!(
            engine
                .store
                .results
                .lock()
                .unwrap()
                .get(id)
                .is_none_or(|r| r.errors.is_empty()),
            "{}{}",
            choose(engine.zh, "需先安装：", "Install first: "),
            engine.catalog[i].name(engine.zh)
        );
    }
    Ok(())
}
fn batch(
    engine: &Engine,
    tx: &Sender<Message>,
    version: usize,
    command: &Command,
    auto: bool,
) -> Result<()> {
    engine.begin_batch();
    let all = matches!(command, Command::All);
    let safe_ids = engine
        .store
        .pending
        .lock()
        .unwrap()
        .as_ref()
        .map(|p| p.safe_ids.clone())
        .unwrap_or_default();
    let indices = match command {
        Command::One(i) => vec![*i],
        Command::Items(indices) => indices.clone(),
        _ => order(&engine.catalog),
    };
    let _ = tx.send(Message::Queued(indices.clone()));
    let safe = std::sync::Mutex::new(Vec::new());
    let final_item = |f: &Feature| matches!(f.id.as_str(), "input-method" | "desktop-icons");
    let mut local: Vec<_> = indices
        .iter()
        .copied()
        .filter(|i| {
            matches!(command, Command::One(_))
                || (engine.catalog[*i].page != "Install" && !final_item(&engine.catalog[*i]))
        })
        .collect();
    // Start fast settings before slow removals; jobs from different categories overlap.
    local.sort_by_key(|i| engine.catalog[*i].page == "Remove");
    let mut processed = parallel_jobs(&local, |i| {
        let f = &engine.catalog[i];
        if f.manual {
            publish(engine, tx, i, version);
            return Ok(false);
        }
        if engine.safe && !f.can_run_safe() && !safe_ids.contains(&f.id) {
            let _ = tx.send(Message::State(i, Check::new(Status::Deferred), version));
            return Ok(false);
        }
        let check = engine.check(f);
        if !check.actionable()
            && check.state != Status::SafeQueued
            && !(matches!(command, Command::One(_)) && f.toggle())
        {
            publish(engine, tx, i, version);
            return Ok(false);
        }
        let needs_safe = match engine.needs_safe(f) {
            Ok(needs_safe) => needs_safe,
            Err(error) => {
                failure(engine, tx, i, version, error)?;
                return Ok(false);
            }
        };
        if needs_safe {
            safe.lock().unwrap().push(f.id.clone());
            let _ = tx.send(Message::State(i, Check::new(Status::SafeQueued), version));
            return Ok(false);
        }
        if !engine.safe
            && let Err(error) = prerequisite(engine, tx, f, version)
        {
            failure(engine, tx, i, version, error)?;
            return Ok(false);
        }
        perform(engine, tx, i, version, matches!(command, Command::One(_)))?;
        Ok(true)
    })?;
    let safe = safe.into_inner().unwrap();
    if engine.safe {
        if engine
            .store
            .pending
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|p| p.phase == workflow::Phase::AwaitSafe)
        {
            workflow::safe_complete(engine)?;
        }
        let ids = if all {
            vec![]
        } else {
            indices
                .iter()
                .map(|i| engine.catalog[*i].id.clone())
                .collect()
        };
        workflow::prepare_normal(engine, all, &ids)?;
    } else if !safe.is_empty() {
        workflow::prepare_safe(engine, &safe, all)?;
    } else if workflow::requires_restart(engine) {
        workflow::prepare_normal(engine, all, &[])?;
    }
    // All local restart/sign-in settings have been applied before crossing the boot boundary.
    let boot_pending = engine.store.pending.lock().unwrap().is_some();
    if !boot_pending && !matches!(command, Command::One(_)) {
        let installers: Vec<_> = indices
            .iter()
            .copied()
            .filter(|i| engine.catalog[*i].page == "Install")
            .collect();
        processed += parallel_jobs(&installers, |i| {
            if !engine.check(&engine.catalog[i]).actionable() {
                publish(engine, tx, i, version);
                return Ok(false);
            }
            perform(engine, tx, i, version, false)?;
            Ok(true)
        })?;
        let finalizers: Vec<_> = indices
            .iter()
            .copied()
            .filter(|i| final_item(&engine.catalog[*i]) || engine.catalog[*i].id == "start-menu")
            .collect();
        processed += parallel_jobs(&finalizers, |i| {
            let f = &engine.catalog[i];
            if !engine.check(f).actionable() {
                publish(engine, tx, i, version);
                return Ok(false);
            }
            // Failure of an installation only blocks the setting that depends on it.
            if let Err(error) = prerequisite(engine, tx, f, version) {
                failure(engine, tx, i, version, error)?;
                return Ok(false);
            }
            perform(engine, tx, i, version, false)?;
            Ok(true)
        })?;
        if workflow::requires_restart(engine) {
            let ids = if all {
                vec![]
            } else {
                indices
                    .iter()
                    .map(|i| engine.catalog[*i].id.clone())
                    .collect()
            };
            workflow::prepare_normal(engine, all, &ids)?;
        }
    }
    if boot_pending && !matches!(command, Command::One(_)) {
        for &i in &indices {
            let f = &engine.catalog[i];
            if f.page == "Install" || final_item(f) {
                let _ = tx.send(Message::State(i, Check::new(Status::Deferred), version));
            }
        }
    }
    let _ = tx.send(Message::Finished {
        processed,
        restart: workflow::requires_restart(engine),
        auto,
        refresh: affected(engine, command),
    });
    Ok(())
}

pub fn failure_message(engine: &Engine, error: &str) -> String {
    let _ = store::append("errors.log", error);
    format!(
        "{} · {}",
        choose(engine.zh, "执行未完成", "Execution incomplete"),
        store::root().join("errors.log").display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installation_jobs_overlap_with_a_limit_and_keep_going_after_an_error() {
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let seen = AtomicUsize::new(0);
        let barrier = std::sync::Barrier::new(PARALLELISM);
        let result = parallel_jobs(&[0, 1, 2, 3, 4, 5], |i| {
            let now = active.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(now, Ordering::SeqCst);
            barrier.wait();
            seen.fetch_or(1 << i, Ordering::SeqCst);
            active.fetch_sub(1, Ordering::SeqCst);
            if i == 2 {
                anyhow::bail!("simulated installer failure");
            }
            Ok(true)
        });
        assert!(result.is_err());
        assert_eq!(peak.load(Ordering::SeqCst), PARALLELISM);
        assert_eq!(seen.load(Ordering::SeqCst), 63);
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }
}
