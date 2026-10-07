use crate::{
    apps,
    controller::{Command, order},
    engine::Engine,
    model::*,
    native, preferences, registry,
    tui::App,
};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use rat_widget::table::selection::rowselection;
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;

#[test]
fn registry_notifications_rearm_without_polling_values() -> anyhow::Result<()> {
    use winreg::{RegKey, enums::HKEY_CURRENT_USER};
    let root = RegKey::predef(HKEY_CURRENT_USER);
    let name = format!(
        r"Software\ZeroSecurityWindowsThemeTest{}",
        std::process::id()
    );
    let (key, _) = root.create_subkey(&name)?;
    let result = (|| -> anyhow::Result<()> {
        let watch = registry::Watch::new(&format!(r"HKCU:\{name}"))?;
        for value in [0u32, 1, 0] {
            assert!(!watch.changed()?);
            key.set_value("AppsUseLightTheme", &value)?;
            assert!(watch.changed()?);
            assert!(!watch.changed()?);
        }
        Ok(())
    })();
    drop(key);
    root.delete_subkey_all(&name)?;
    result
}

#[test]
fn native_app_registration_and_current_state_resolve_an_old_install_error() -> anyhow::Result<()> {
    native::privilege("SeBackupPrivilege")?;
    native::privilege("SeRestorePrivilege")?;
    let key = format!("ZeroSecurityWindowsTestInstall{}", std::process::id());
    let path = format!(r"HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\{key}");
    let op = Operation {
        kind: "Winget".into(),
        name: "Test.Package".into(),
        uninstall_key: key,
        value: json!(true),
        ..Default::default()
    };
    let f = Feature {
        id: "test-install".into(),
        ops: vec![op.clone()],
        ..Default::default()
    };
    let engine = Engine::new()?;
    let result = (|| -> anyhow::Result<()> {
        assert_eq!(apps::registered_tool(&op)?, Some(false));
        engine.store.results.lock().unwrap().insert(
            f.id.clone(),
            ResultRecord {
                errors: vec!["previous installer failure".into()],
                ..Default::default()
            },
        );
        assert_eq!(engine.check(&f).state, Status::Failed);
        registry::set(&path, "DisplayName", "Test package", "String")?;
        assert_eq!(apps::registered_tool(&op)?, Some(true));
        assert_eq!(engine.check(&f).state, Status::Done);
        assert!(
            engine.store.results.lock().unwrap()[&f.id]
                .errors
                .is_empty()
        );
        Ok(())
    })();
    registry::key(&path, false)?;
    result
}

#[test]
fn scheduled_task_disabling_is_idempotent_without_running_its_action() -> anyhow::Result<()> {
    use windows::{
        Win32::System::{TaskScheduler::*, Variant::VARIANT},
        core::BSTR,
    };
    let name = format!(r"\ZeroSecurityWindowsTestTask{}", std::process::id());
    assert!(!native::task_enabled(&name)?);
    native::disable_task(&name)?;
    let document = r#"<Task version="1.4" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task"><Principals><Principal id="System"><UserId>S-1-5-18</UserId></Principal></Principals><Settings><AllowStartOnDemand>false</AllowStartOnDemand></Settings><Actions Context="System"><Exec><Command>C:\Windows\System32\cmd.exe</Command></Exec></Actions></Task>"#;
    let folder = native::task_root()?;
    let task = unsafe {
        folder.RegisterTask(
            &BSTR::from(&name),
            &BSTR::from(document),
            TASK_CREATE.0,
            &VARIANT::from("SYSTEM"),
            &VARIANT::default(),
            TASK_LOGON_SERVICE_ACCOUNT,
            &VARIANT::default(),
        )?
    };
    let last_run = unsafe { task.LastRunTime()? };
    let result = (|| -> anyhow::Result<()> {
        anyhow::ensure!(native::task_enabled(&name)?);
        native::disable_task(&name)?;
        anyhow::ensure!(!native::task_enabled(&name)?);
        native::disable_task(&name)?;
        anyhow::ensure!(unsafe { task.LastRunTime()? } == last_run);
        Ok(())
    })();
    unsafe {
        folder.DeleteTask(&BSTR::from(&name), 0)?;
    }
    result?;
    assert!(!native::task_enabled(&name)?);
    Ok(())
}

#[test]
fn independent_registry_types_and_deletion() -> anyhow::Result<()> {
    native::privilege("SeBackupPrivilege")?;
    native::privilege("SeRestorePrivilege")?;
    let path = format!(
        r"HKCU:\Software\ZeroSecurityWindowsTest{}",
        std::process::id()
    );
    let result = (|| {
        for (name, value, kind) in [
            ("Number", json!(42), "DWord"),
            ("Text", json!("中文 λ"), "String"),
            ("Binary", json!([0, 128, 255]), "Binary"),
            ("Languages", json!(["zh-CN", "en-US"]), "MultiString"),
        ] {
            registry::set(&path, name, value.clone(), kind)?;
            assert_eq!(registry::read(&path, name)?, value);
        }
        registry::write(&Operation {
            kind: "RegistryDelete".into(),
            path: path.clone(),
            name: "Number".into(),
            ..Default::default()
        })?;
        assert!(registry::read(&path, "Number")?.is_null());
        Ok(())
    })();
    registry::key(&path, false)?;
    result
}
#[test]
fn native_preferences_are_readable() -> anyhow::Result<()> {
    assert!(native::boot() > 0);
    assert_eq!(native::boot(), native::boot());
    assert!(preferences::mouse()?[3] >= 1);
    assert_eq!(preferences::keyboard()?.size, 24);
    assert!(!preferences::scheme()?.is_empty());
    preferences::language_bar(None)?;
    preferences::input_only()?;
    Ok(())
}
#[test]
fn file_execute_acl_targets_only_a_temporary_file() -> anyhow::Result<()> {
    let file = std::env::temp_dir().join(format!("zsw-acl-{}.tmp", std::process::id()));
    std::fs::write(&file, b"test")?;
    let result = (|| {
        assert!(!native::execute_blocked(&file)?);
        native::block_execute(&file)?;
        assert!(native::execute_blocked(&file)?);
        Ok(())
    })();
    std::fs::remove_file(file)?;
    result
}
#[test]
fn frame_and_navigation_work_while_every_item_is_checking() -> anyhow::Result<()> {
    for zh in [true, false] {
        for width in [55, 100, 140] {
            let mut app = App::new(catalog()?, zh)?;
            let mut terminal = Terminal::new(TestBackend::new(width, 30))?;
            terminal.draw(|f| app.draw(f))?;
            assert!(app.table.inner.height > 0);
            assert_eq!(app.table.vscroll.area.width, 1);
            let first = app.selected();
            rowselection::handle_events(
                &mut app.table,
                true,
                &Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
            );
            assert_ne!(app.selected(), first);
            let area = app.table.inner;
            rowselection::handle_events(
                &mut app.table,
                true,
                &Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Down(crossterm::event::MouseButton::Left),
                    column: area.x + 2,
                    row: area.y + 2,
                    modifiers: KeyModifiers::NONE,
                }),
            );
            assert!(!app.busy);
            let first = app.selected();
            rowselection::handle_events(
                &mut app.table,
                true,
                &Event::Mouse(MouseEvent {
                    kind: MouseEventKind::ScrollDown,
                    column: area.x + 2,
                    row: area.y + 2,
                    modifiers: KeyModifiers::NONE,
                }),
            );
            assert_ne!(app.selected(), first);
            let bar = app.table.vscroll.area;
            rowselection::handle_events(
                &mut app.table,
                true,
                &Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Down(crossterm::event::MouseButton::Left),
                    column: bar.x,
                    row: bar.y + bar.height - 2,
                    modifiers: KeyModifiers::NONE,
                }),
            );
            assert!(app.table.selected().unwrap() > 20);
            assert!(!app.busy);
            app.command(&Command::All);
            assert!(app.states.iter().all(|s| s.state == Status::Queued));
            terminal.draw(|f| app.draw(f))?;
        }
    }
    Ok(())
}
#[test]
fn failed_and_actionable_items_keep_their_positions_and_focus_survives_updates()
-> anyhow::Result<()> {
    let mut app = App::new(catalog()?, true)?;
    for state in &mut app.states {
        *state = Check::new(Status::Done)
    }
    let last = app.catalog.len() - 1;
    let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
    terminal.draw(|f| app.draw(f))?;
    app.focus(0);
    let order = app.visible.clone();
    app.states[0] = Check::new(Status::Ready);
    app.reorder();
    app.states[0] = Check::new(Status::Done);
    app.states[last] = Check::new(Status::Ready);
    app.reorder();
    assert_eq!(app.selected(), Some(0));
    assert!(app.table.selected().unwrap() >= app.table.vscroll.offset());
    assert!(
        app.table.selected().unwrap() < app.table.vscroll.offset() + app.table.vscroll.page_len()
    );
    app.states[last] = Check::new(Status::Failed);
    app.reorder();
    assert_eq!(app.visible, order);
    app.focus(last);
    app.states[0] = Check::new(Status::Ready);
    app.reorder();
    assert_eq!(app.selected(), Some(last));
    Ok(())
}
#[test]
fn delivery_optimization_precedes_installation() -> anyhow::Result<()> {
    let catalog = catalog()?;
    let order = order(&catalog);
    let position = |id: &str| order.iter().position(|i| catalog[*i].id == id).unwrap();
    assert!(position("delivery-optimization") < position("install-tencent.wetype"));
    Ok(())
}

#[test]
fn retry_reads_fresh_inventory_after_a_temporary_detection_failure() -> anyhow::Result<()> {
    let engine = Engine::new()?;
    assert!(
        engine
            .fact("tools", || anyhow::bail!("network not ready"))
            .is_err()
    );
    engine.begin_batch();
    assert_eq!(
        engine.fact("tools", || Ok(json!(["Tencent.WeType"])))?,
        json!(["Tencent.WeType"])
    );
    Ok(())
}

#[test]
fn safe_mode_expands_local_settings_without_opening_the_task_service() -> anyhow::Result<()> {
    let mut engine = Engine::new()?;
    engine.safe = true;
    let f = Feature {
        ops: vec![
            Operation::reg(r"HKCU:\Software\ZeroSecurityWindowsTest", "Enabled", 0),
            Operation {
                kind: "Task".into(),
                path: "task-service-unavailable".into(),
                ..Default::default()
            },
            Operation {
                kind: "TaskGroup".into(),
                pattern: "*".into(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    assert!(f.can_run_safe());
    let ops = engine.expand(&f)?;
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].kind, "Registry");
    for kind in ["Winget", "Apps", "OptionalFeature", "Firewall"] {
        let f = Feature {
            ops: vec![Operation {
                kind: kind.into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(!f.can_run_safe());
    }
    Ok(())
}

#[test]
fn location_off_state_does_not_require_a_policy_lock() -> anyhow::Result<()> {
    let engine = Engine::new()?;
    engine.store.initialize()?;
    let root = format!(
        r"HKCU:\Software\ZeroSecurityWindowsLocationTest{}",
        std::process::id()
    );
    let mut f = engine.feature("location")?.clone();
    f.id = "test-location-preferences".into();
    f.ops
        .retain(|op| matches!(op.kind.as_str(), "Registry" | "RegistryDelete"));
    for (i, op) in f.ops.iter_mut().enumerate() {
        op.path = format!(r"{root}\{i}");
    }
    let result = (|| -> anyhow::Result<()> {
        for op in &f.ops {
            registry::write(op)?;
        }
        assert_eq!(engine.check(&f).state, Status::Done);
        let consent = f.ops.iter().find(|op| op.name == "Value").unwrap();
        registry::set(&consent.path, &consent.name, "Allow", "String")?;
        assert_eq!(engine.check(&f).state, Status::Ready);
        registry::write(consent)?;
        assert_eq!(engine.check(&f).state, Status::Done);
        let policy = f
            .ops
            .iter()
            .find(|op| op.name == "DisableLocation")
            .unwrap();
        registry::set(&policy.path, &policy.name, 1, "DWord")?;
        assert_eq!(engine.check(&f).state, Status::Ready);
        registry::write(policy)?;
        assert_eq!(engine.check(&f).state, Status::Done);
        Ok(())
    })();
    registry::key(&root, false)?;
    result
}

#[test]
#[ignore = "修复本机位置开关；仅在用户要求时执行"]
fn repair_location_controls_on_this_machine() -> anyhow::Result<()> {
    let engine = Engine::new()?;
    let f = engine.feature("location")?;
    let result = engine.execute(f, false, false, false)?;
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert!(!result.restart);
    assert_eq!(native::service_start("lfsvc", None)?, 3);
    assert!(
        registry::read(
            r"HKLM:\SOFTWARE\Policies\Microsoft\Windows\LocationAndSensors",
            "DisableLocation",
        )?
        .is_null()
    );
    assert!(preferences::settings_page("privacy-location", false)?);
    assert_eq!(engine.check(f).state, Status::Done);
    let repeated = engine.execute(f, false, false, false)?;
    assert!(repeated.errors.is_empty());
    assert!(!repeated.changed);
    Ok(())
}
