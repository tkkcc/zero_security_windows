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
    Restore(usize),
    RestoreItems(Vec<usize>),
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
        refresh: Vec<usize>,
    },
    Error(String),
}
fn final_item(f: &Feature) -> bool {
    matches!(
        f.id.as_str(),
        "input-method" | "desktop-icons" | "start-menu" | "taskbar-pins"
    )
}
pub fn order(catalog: &[Feature]) -> Vec<usize> {
    let mut indices: Vec<_> = (0..catalog.len()).collect();
    indices.sort_by_key(|i| {
        let f = &catalog[*i];
        if final_item(f) {
            3
        } else if f.page == "Install" {
            2
        } else if f.security() || workflow::QUIET.contains(&f.id.as_str()) {
            0
        } else {
            1
        }
    });
    indices
}
fn job_groups(catalog: &[Feature], indices: &[usize], one: bool) -> [Vec<usize>; 3] {
    let mut groups = [vec![], vec![], vec![]];
    for &i in indices {
        let f = &catalog[i];
        let group = if one {
            0
        } else if final_item(f) {
            2
        } else if f.page == "Install" {
            1
        } else {
            0
        };
        groups[group].push(i);
    }
    groups[0].sort_by_key(|i| catalog[*i].page == "Remove");
    groups
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
                    let indices = engine
                        .catalog
                        .iter()
                        .enumerate()
                        .filter(|(_, f)| pending.as_ref().unwrap().safe_ids.contains(&f.id))
                        .map(|(i, _)| i)
                        .collect();
                    if pending.as_ref().unwrap().restoring {
                        Command::RestoreItems(indices)
                    } else {
                        Command::Items(indices)
                    }
                };
                batch(&engine, &tx, version, &command, true)?;
            } else if mode == "--resume" {
                if let Some(command) = resume_command(&engine)? {
                    let version = epoch.fetch_add(1, Ordering::SeqCst) + 1;
                    batch(&engine, &tx, version, &command, automatic)?;
                } else {
                    let _ = tx.send(Message::Finished {
                        processed: 0,
                        restart: workflow::requires_restart(&engine),
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
pub(crate) fn resume_command(engine: &Engine) -> Result<Option<Command>> {
    let (all, ids, restoring) = workflow::normal_resume(engine)?;
    if all {
        return Ok(Some(Command::All));
    }
    let indices: Vec<_> = engine
        .catalog
        .iter()
        .enumerate()
        .filter(|(_, feature)| ids.contains(&feature.id))
        .map(|(i, _)| i)
        .collect();
    Ok((!indices.is_empty()).then_some(if restoring {
        Command::RestoreItems(indices)
    } else {
        Command::Items(indices)
    }))
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
        Command::One(i) | Command::Restore(i) => vec![&engine.catalog[*i]],
        Command::Items(indices) | Command::RestoreItems(indices) => {
            indices.iter().map(|i| &engine.catalog[*i]).collect()
        }
        _ => return vec![],
    };
    engine
        .catalog
        .iter()
        .enumerate()
        .filter(|(_, f)| {
            touched.iter().any(|t| {
                t.id == "phishing-services" && f.id == "phishing-protection"
                    || t.group_zh == f.group_zh
                        && matches!(t.group_zh.as_str(), "Defender" | "核心隔离")
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
fn needs_safe(engine: &Engine, f: &Feature, auto: bool) -> Result<bool> {
    let needed = engine.needs_safe(f)?;
    if needed && auto && engine.attempted_safe.lock().unwrap().contains(&f.id) {
        let errors = engine
            .store
            .results
            .lock()
            .unwrap()
            .get(&f.id)
            .map(|record| record.errors.join("; "))
            .unwrap_or_default();
        anyhow::bail!(
            "Safe Mode did not complete {}; automatic continuation stopped. {}",
            f.id,
            errors
        );
    }
    Ok(needed)
}
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
pub(crate) fn batch(
    engine: &Engine,
    tx: &Sender<Message>,
    version: usize,
    command: &Command,
    auto: bool,
) -> Result<()> {
    engine.begin_batch();
    if matches!(command, Command::Restore(_) | Command::RestoreItems(_)) {
        return restore_batch(engine, tx, version, command, auto);
    }
    let all = matches!(command, Command::All);
    let safe_ids = engine
        .store
        .pending
        .lock()
        .unwrap()
        .as_ref()
        .map(|p| p.safe_ids.clone())
        .unwrap_or_default();
    let mut indices = match command {
        Command::One(i) => vec![*i],
        Command::Items(indices) => indices.clone(),
        _ => order(&engine.catalog),
    };
    if matches!(command, Command::All) {
        indices.retain(|i| !engine.catalog[*i].default_off);
    }
    let resume_ids = if all {
        vec![]
    } else {
        indices
            .iter()
            .map(|i| engine.catalog[*i].id.clone())
            .collect()
    };
    let _ = tx.send(Message::Queued(indices.clone()));
    let safe = std::sync::Mutex::new(Vec::new());
    let [local, installers, finalizers] = job_groups(
        &engine.catalog,
        &indices,
        matches!(command, Command::One(_)),
    );
    // MMAgent updates share one configuration and overwrite concurrent changes.
    let (memory, local): (Vec<_>, Vec<_>) = local.into_iter().partition(|i| {
        engine.catalog[*i].ops.iter().any(|op| op.kind == "MemoryAgent")
    });
    // Start fast settings before slow removals; jobs from different categories overlap.
    let run_local = |i: usize| {
        let f = &engine.catalog[i];
        if f.manual || f.default_off && matches!(command, Command::All) {
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
        let needs_safe = match needs_safe(engine, f, auto) {
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
    };
    let mut processed = parallel_jobs(&local, run_local)?;
    for i in memory {
        processed += usize::from(run_local(i)?);
    }
    let safe = safe.into_inner().unwrap();
    if !engine.safe && !matches!(command, Command::One(_)) {
        processed += parallel_jobs(&installers, |i| {
            if !engine.check(&engine.catalog[i]).actionable() {
                publish(engine, tx, i, version);
                return Ok(false);
            }
            perform(engine, tx, i, version, false)?;
            Ok(true)
        })?;
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
    }
    // Finish normal-mode installs and their dependent settings before scheduling any restart.
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
        workflow::prepare_normal(engine, all, &resume_ids)?;
    } else if !safe.is_empty() {
        workflow::prepare_safe(engine, &safe, all, false)?;
        workflow::prepare_normal(engine, all, &resume_ids)?;
    } else if workflow::requires_restart(engine) {
        workflow::prepare_normal(engine, all, &resume_ids)?;
    }
    if engine.safe && !matches!(command, Command::One(_)) {
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
        refresh: affected(engine, command),
    });
    Ok(())
}
fn restore_batch(
    engine: &Engine,
    tx: &Sender<Message>,
    version: usize,
    command: &Command,
    auto: bool,
) -> Result<()> {
    let indices = match command {
        Command::Restore(i) => vec![*i],
        Command::RestoreItems(ids) => ids.clone(),
        _ => unreachable!(),
    };
    if !auto && engine.store.pending.lock().unwrap().is_some() {
        workflow::cleanup(engine)?;
    }
    let _ = tx.send(Message::Queued(indices.clone()));
    let mut safe_ids = vec![];
    let mut normal_ids = vec![];
    let mut processed = 0;
    for &i in &indices {
        let f = &engine.catalog[i];
        let target = f.restore_target();
        if auto
            && matches!(
                engine.default_check(f)?.state,
                Status::Done | Status::Absent
            )
        {
            publish(engine, tx, i, version);
            continue;
        }
        if f.id == "smart-app-control" && !engine.safe {
            if auto && engine.attempted_safe.lock().unwrap().contains(&f.id) {
                failure(
                    engine,
                    tx,
                    i,
                    version,
                    anyhow::anyhow!(
                        "Offline Smart App Control restoration did not complete; automatic continuation stopped"
                    ),
                )?;
                engine
                    .store
                    .results
                    .lock()
                    .unwrap()
                    .get_mut(&f.id)
                    .unwrap()
                    .restoring = true;
            } else {
                if let Err(error) = workflow::prepare_sac_recovery(engine) {
                    failure(engine, tx, i, version, error)?;
                    engine
                        .store
                        .results
                        .lock()
                        .unwrap()
                        .get_mut(&f.id)
                        .unwrap()
                        .restoring = true;
                }
                publish(engine, tx, i, version);
            }
            continue;
        }
        if engine.safe && !target.can_run_safe() {
            normal_ids.push(f.id.clone());
            let _ = tx.send(Message::State(i, Check::new(Status::Deferred), version));
            continue;
        }
        let needs_safe = match needs_safe(engine, &target, auto) {
            Ok(needed) => needed,
            Err(error) => {
                failure(engine, tx, i, version, error)?;
                engine
                    .store
                    .results
                    .lock()
                    .unwrap()
                    .get_mut(&f.id)
                    .unwrap()
                    .restoring = true;
                continue;
            }
        };
        if needs_safe {
            safe_ids.push(f.id.clone());
            let _ = tx.send(Message::State(i, Check::new(Status::SafeQueued), version));
            continue;
        }
        let _ = tx.send(Message::State(i, Check::new(Status::Running), version));
        if let Err(error) = engine.restore(f, false, engine.safe) {
            failure(engine, tx, i, version, error)?;
            engine
                .store
                .results
                .lock()
                .unwrap()
                .get_mut(&f.id)
                .unwrap()
                .restoring = true;
        }
        publish(engine, tx, i, version);
        processed += 1;
    }
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
        workflow::prepare_normal(engine, false, &normal_ids)?;
    } else if !safe_ids.is_empty() {
        workflow::prepare_safe(engine, &safe_ids, false, true)?;
        workflow::prepare_normal(engine, false, &safe_ids)?;
    } else if workflow::requires_restart(engine) {
        workflow::prepare_normal(engine, false, &[])?;
    }
    if let Some(p) = engine.store.pending.lock().unwrap().as_mut() {
        p.restoring = true;
    }
    engine.store.save()?;
    let _ = tx.send(Message::Finished {
        processed,
        restart: workflow::requires_restart(engine),
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
    fn failed_safe_restore_stops_automatic_boots_but_allows_an_explicit_retry() -> Result<()> {
        let mut engine = Engine::new()?;
        engine.safe = false;
        let mut target = engine.feature("process-mitigations")?.restore_target();
        target.safe = true;
        assert!(needs_safe(&engine, &target, true)?);
        engine
            .attempted_safe
            .lock()
            .unwrap()
            .push(target.id.clone());
        assert!(needs_safe(&engine, &target, true).is_err());
        assert!(needs_safe(&engine, &target, false)?);
        engine.safe = true;
        assert!(!needs_safe(&engine, &target, true)?);
        Ok(())
    }
    #[test]
    fn batch_groups_apply_local_settings_then_install_and_finalize_once() -> Result<()> {
        let catalog = catalog()?;
        let ordered = order(&catalog);
        let groups = job_groups(&catalog, &ordered, false);
        let mut visits = vec![0; catalog.len()];
        for (stage, group) in groups.iter().enumerate() {
            for &i in group {
                visits[i] += 1;
                if catalog[i].page == "Install" {
                    assert_eq!(stage, 1);
                }
                if final_item(&catalog[i]) {
                    assert_eq!(stage, 2);
                }
                if catalog[i].restart {
                    assert_eq!(stage, 0);
                }
            }
        }
        assert!(visits.iter().all(|count| *count == 1));
        let settings_end = groups[0]
            .iter()
            .position(|i| catalog[*i].page == "Remove")
            .unwrap();
        assert!(
            groups[0][..settings_end]
                .iter()
                .all(|i| catalog[*i].page != "Remove")
        );
        assert!(
            groups[0][settings_end..]
                .iter()
                .all(|i| catalog[*i].page == "Remove")
        );
        for i in ordered {
            let [local, installers, finalizers] = job_groups(&catalog, &[i], true);
            assert_eq!(local, [i]);
            assert!(installers.is_empty() && finalizers.is_empty());
        }
        Ok(())
    }
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
