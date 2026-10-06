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
    Category(String),
    All,
    Restart,
    Scan(Vec<usize>),
}
pub enum Message {
    Ready(Arc<Engine>),
    State(usize, Check, usize),
    Focus(usize),
    Finished {
        processed: usize,
        restart: bool,
        auto: bool,
        refresh: Vec<usize>,
    },
    Error(String),
}
pub fn order(catalog: &[Feature]) -> Vec<usize> {
    catalog
        .iter()
        .enumerate()
        .filter(|(_, f)| f.security() || workflow::QUIET.contains(&f.id.as_str()))
        .chain(
            catalog
                .iter()
                .enumerate()
                .filter(|(_, f)| !f.security() && !workflow::QUIET.contains(&f.id.as_str())),
        )
        .map(|(i, _)| i)
        .collect()
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
                workflow::safe_resume(&engine, |i| {
                    let _ = tx.send(Message::Focus(i));
                    let _ = tx.send(Message::State(i, Check::new(Status::Running), version));
                })?;
                let _ = tx.send(Message::Finished {
                    processed: engine.store.results.lock().unwrap().len(),
                    restart: true,
                    auto: true,
                    refresh: vec![],
                });
            } else if mode == "--resume" {
                if workflow::normal_resume(&engine)? {
                    let version = epoch.fetch_add(1, Ordering::SeqCst) + 1;
                    batch(&engine, &tx, version, &Command::All, true, automatic)?;
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
                if let Err(e) = batch(&engine, &tx, version, &command, false, false) {
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
        Command::Category(c) => engine
            .catalog
            .iter()
            .filter(|f| f.category(engine.zh) == c)
            .collect(),
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
fn perform(
    engine: &Engine,
    tx: &Sender<Message>,
    i: usize,
    version: usize,
    toggle: bool,
) -> Result<()> {
    let f = &engine.catalog[i];
    let _ = tx.send(Message::Focus(i));
    let _ = tx.send(Message::State(i, Check::new(Status::Running), version));
    let win11 = toggle && f.toggle() && crate::registry::exists(&f.ops[0].path)?;
    engine.execute(f, win11, false, false)?;
    publish(engine, tx, i, version);
    Ok(())
}
fn parallel_jobs(indices: &[usize], run: impl Fn(usize) -> Result<bool> + Sync) -> Result<usize> {
    let next = AtomicUsize::new(0);
    let processed = AtomicUsize::new(0);
    let errors = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..indices.len().min(2) {
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
        "startup-uniget" => "install-xpfftq032ptphf",
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
    resumed: bool,
    auto: bool,
) -> Result<()> {
    let all = matches!(command, Command::All);
    let indices = match command {
        Command::One(i) => vec![*i],
        Command::Category(category) => order(&engine.catalog)
            .into_iter()
            .filter(|i| engine.catalog[*i].category(engine.zh) == category)
            .collect(),
        _ => order(&engine.catalog),
    };
    let mut safe = vec![];
    let mut processed = 0;
    // Stop the batch at a boot boundary before installers run under active protection.
    if all {
        for &i in indices.iter().filter(|i| {
            engine.catalog[**i].security()
                || workflow::QUIET.contains(&engine.catalog[**i].id.as_str())
        }) {
            let f = &engine.catalog[i];
            let state = engine.check(f);
            if !state.actionable() {
                publish(engine, tx, i, version);
                continue;
            }
            if engine.needs_safe(f)? {
                safe.push(f.id.clone());
                let _ = tx.send(Message::State(i, Check::new(Status::SafeQueued), version));
            } else {
                perform(engine, tx, i, version, false)?;
                processed += 1
            }
        }
        if !safe.is_empty() {
            workflow::prepare_safe(engine, &safe, true)?;
        } else if workflow::requires_restart(engine) || engine.safe {
            workflow::prepare_normal(engine, true)?;
        }
        if engine.store.pending.lock().unwrap().is_some() {
            let _ = tx.send(Message::Finished {
                processed,
                restart: true,
                auto,
                refresh: vec![],
            });
            return Ok(());
        }
        workflow::assert_quiet(engine)?;
    }
    let installers: Vec<usize> = indices
        .iter()
        .copied()
        .filter(|i| {
            !matches!(command, Command::One(_))
                && !engine.safe
                && engine.catalog[*i].page == "Install"
        })
        .collect();
    processed += parallel_jobs(&installers, |i| {
        if !engine.check(&engine.catalog[i]).actionable() {
            publish(engine, tx, i, version);
            return Ok(false);
        }
        if let Err(error) = perform(engine, tx, i, version, false) {
            let _ = tx.send(Message::State(
                i,
                Check {
                    state: Status::Failed,
                    detail: format!("{error:#}"),
                },
                version,
            ));
            return Err(error);
        }
        Ok(true)
    })?;
    for i in indices {
        let f = &engine.catalog[i];
        if installers.contains(&i)
            || (all && (f.security() || workflow::QUIET.contains(&f.id.as_str())))
        {
            continue;
        }
        let check = engine.check(f);
        if !check.actionable() && !(matches!(command, Command::One(_)) && f.toggle()) {
            publish(engine, tx, i, version);
            continue;
        }
        if engine.needs_safe(f)? {
            safe.push(f.id.clone());
            let _ = tx.send(Message::State(i, Check::new(Status::SafeQueued), version));
            continue;
        }
        prerequisite(engine, tx, f, version)?;
        perform(engine, tx, i, version, matches!(command, Command::One(_)))?;
        processed += 1;
    }
    if !safe.is_empty() {
        workflow::prepare_safe(engine, &safe, false)?;
    } else if workflow::requires_restart(engine) {
        workflow::prepare_normal(engine, false)?;
    }
    // A resumed batch is the only path that starts an automatic countdown.
    let _ = tx.send(Message::Finished {
        processed,
        restart: workflow::requires_restart(engine),
        auto: auto && resumed,
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
        let barrier = std::sync::Barrier::new(2);
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
        assert_eq!(peak.load(Ordering::SeqCst), 2);
        assert_eq!(seen.load(Ordering::SeqCst), 63);
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }
}
