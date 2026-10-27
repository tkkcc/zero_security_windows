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
fn restoration_history_does_not_replace_the_current_configuration_state() -> anyhow::Result<()> {
    use winreg::{RegKey, enums::HKEY_CURRENT_USER};
    let root = RegKey::predef(HKEY_CURRENT_USER);
    let name = format!(
        r"Software\ZeroSecurityWindowsCurrentStateTest{}",
        std::process::id()
    );
    let (key, _) = root.create_subkey(&name)?;
    let path = format!(r"HKCU:\{name}");
    let feature = Feature {
        id: "test-current-state".into(),
        ops: vec![Operation::reg(&path, "Enabled", 0)],
        restore_ops: vec![Operation::reg(&path, "Enabled", 1)],
        ..Default::default()
    };
    let engine = Engine::new()?;
    engine.store.results.lock().unwrap().insert(
        feature.id.clone(),
        ResultRecord {
            restoring: true,
            ..Default::default()
        },
    );
    let result = (|| -> anyhow::Result<()> {
        key.set_value("Enabled", &1u32)?;
        assert_eq!(engine.default_check(&feature)?.state, Status::Done);
        assert_eq!(engine.check(&feature).state, Status::Ready);
        key.set_value("Enabled", &0u32)?;
        assert_eq!(engine.default_check(&feature)?.state, Status::Ready);
        assert_eq!(engine.check(&feature).state, Status::Done);
        engine
            .store
            .results
            .lock()
            .unwrap()
            .get_mut(&feature.id)
            .unwrap()
            .errors
            .push("restore failed".into());
        assert_eq!(engine.check(&feature).state, Status::Failed);
        key.set_value("Enabled", &1u32)?;
        assert_eq!(engine.check(&feature).state, Status::Ready);
        Ok(())
    })();
    drop(key);
    root.delete_subkey_all(name)?;
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
        assert!(!apps::registered_tool(&op)?);
        engine.store.results.lock().unwrap().insert(
            f.id.clone(),
            ResultRecord {
                errors: vec!["previous installer failure".into()],
                ..Default::default()
            },
        );
        assert_eq!(engine.check(&f).state, Status::Failed);
        registry::set(&path, "DisplayName", "Test package", "String")?;
        assert!(apps::registered_tool(&op)?);
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
fn scheduled_task_switches_are_idempotent_without_running_the_action() -> anyhow::Result<()> {
    use windows::{
        Win32::System::{TaskScheduler::*, Variant::VARIANT},
        core::BSTR,
    };
    let name = format!(r"\ZeroSecurityWindowsTestTask{}", std::process::id());
    assert!(!native::task_enabled(&name)?);
    native::set_task_enabled(&name, false)?;
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
        native::set_task_enabled(&name, false)?;
        anyhow::ensure!(!native::task_enabled(&name)?);
        native::set_task_enabled(&name, false)?;
        native::set_task_enabled(&name, true)?;
        anyhow::ensure!(native::task_enabled(&name)?);
        native::set_task_enabled(&name, true)?;
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
    preferences::default_input_active()?;
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
            app.focus(app.catalog.iter().position(|f| f.id == "mouse").unwrap());
            let mut terminal = Terminal::new(TestBackend::new(width, 24))?;
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
            let before_page = app.table.selected().unwrap();
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
            assert!(app.table.selected().unwrap() > before_page);
            assert!(!app.busy);
            assert!(!app.command(&Command::All));
            assert!(!app.busy);
            assert!(
                app.states
                    .iter()
                    .all(|state| state.state == Status::PendingCheck)
            );
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
fn location_off_state_requires_the_disable_policy() -> anyhow::Result<()> {
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
        registry::set(&policy.path, &policy.name, 0, "DWord")?;
        assert_eq!(engine.check(&f).state, Status::Ready);
        registry::write(policy)?;
        assert_eq!(engine.check(&f).state, Status::Done);
        Ok(())
    })();
    registry::key(&root, false)?;
    result
}

#[test]
fn privacy_detection_does_not_reapply_disabled_settings() -> anyhow::Result<()> {
    let engine = Engine::new()?;
    engine.store.initialize()?;
    let path = format!(
        r"HKCU:\Software\ZeroSecurityWindowsGuiTest{}",
        std::process::id()
    );
    let mut feature = engine.feature("advertising")?.clone();
    feature.id = "test-gui-preferences".into();
    for op in &mut feature.ops {
        op.path = format!(r"{}\{}", path, op.path.split_once('\\').unwrap().1);
    }
    let result = (|| -> anyhow::Result<()> {
        for op in &feature.ops {
            engine.write(op, &feature.id)?;
        }
        assert_eq!(engine.check(&feature).state, Status::Done);
        for op in feature.ops.iter().filter(|op| op.kind == "Registry") {
            registry::set(&op.path, &op.name, 1, "DWord")?;
        }
        assert_eq!(engine.check(&feature).state, Status::Ready);
        for op in feature.ops.iter().filter(|op| op.kind == "Registry") {
            assert_eq!(registry::read(&op.path, &op.name)?, json!(1));
        }
        Ok(())
    })();
    registry::key(&path, false)?;
    result
}

#[test]
fn widgets_require_the_disable_policy_and_taskbar_preference() -> anyhow::Result<()> {
    let engine = Engine::new()?;
    let root = format!(
        r"HKCU:\Software\ZeroSecurityWindowsWidgetsTest{}",
        std::process::id()
    );
    let mut feature = engine.feature("widgets")?.clone();
    feature.id = "test-widget-preferences".into();
    for op in &mut feature.ops {
        let (hive, path) = op.path.split_once(":\\").unwrap();
        op.path = format!(r"{root}\{hive}\{path}");
    }
    let policy = format!(r"{root}\HKLM\SOFTWARE\Policies\Microsoft\Dsh");
    let preference =
        format!(r"{root}\HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\Advanced");
    let result = (|| -> anyhow::Result<()> {
        registry::set(&policy, "AllowNewsAndInterests", 0, "DWord")?;
        registry::set(&preference, "TaskbarDa", 0, "DWord")?;
        assert_eq!(engine.check(&feature).state, Status::Ready);
        for op in &feature.ops {
            engine.write(op, &feature.id)?;
        }
        assert_eq!(registry::read(&policy, "AllowNewsAndInterests")?, json!(0));
        assert_eq!(engine.check(&feature).state, Status::Done);
        registry::set(&preference, "TaskbarDa", 1, "DWord")?;
        assert_eq!(engine.check(&feature).state, Status::Ready);
        assert_eq!(registry::read(&preference, "TaskbarDa")?, json!(1));
        Ok(())
    })();
    registry::key(&root, false)?;
    result
}

#[test]
fn start_menu_disables_frequent_apps_and_tracking_together() -> anyhow::Result<()> {
    let engine = Engine::new()?;
    let root = format!(
        r"HKCU:\Software\ZeroSecurityWindowsStartTest{}",
        std::process::id()
    );
    let mut feature = engine.feature("start-menu")?.clone();
    feature.id = "test-start-preferences".into();
    feature.ops.retain(|op| {
        matches!(
            op.kind.as_str(),
            "Registry" | "RegistryDelete" | "RegistryKey"
        )
    });
    for op in &mut feature.ops {
        op.path = format!(r"{root}\{}", op.path.split_once('\\').unwrap().1);
    }
    let tracking = feature
        .ops
        .iter()
        .find(|op| op.name == "Start_TrackProgs")
        .unwrap();
    let frequent = feature
        .ops
        .iter()
        .find(|op| op.name == "ShowFrequentList")
        .unwrap();
    let result = (|| -> anyhow::Result<()> {
        registry::set(&tracking.path, &tracking.name, 0, "DWord")?;
        for op in &feature.ops {
            engine.write(op, &feature.id)?;
        }
        assert_eq!(registry::read(&tracking.path, &tracking.name)?, json!(0));
        assert_eq!(registry::read(&frequent.path, &frequent.name)?, json!(0));
        assert_eq!(engine.check(&feature).state, Status::Done);
        registry::set(&frequent.path, &frequent.name, 1, "DWord")?;
        assert_eq!(engine.check(&feature).state, Status::Ready);
        assert_eq!(registry::read(&frequent.path, &frequent.name)?, json!(1));
        assert_eq!(registry::read(&tracking.path, &tracking.name)?, json!(0));
        Ok(())
    })();
    registry::key(&root, false)?;
    result
}

#[test]
fn firewall_group_detects_partial_enabling_and_switches_only_its_rules() -> anyhow::Result<()> {
    use windows::{
        Win32::{NetworkManagement::WindowsFirewall::*, System::Com::*},
        core::BSTR,
    };
    native::com()?;
    let group = format!("ZeroSecurityWindowsTest{}", std::process::id());
    let policy: INetFwPolicy2 =
        unsafe { CoCreateInstance(&NetFwPolicy2, None, CLSCTX_INPROC_SERVER)? };
    let rules = unsafe { policy.Rules()? };
    let names = [format!("{group}-one"), format!("{group}-two")];
    let mut created = vec![];
    let result = (|| -> anyhow::Result<()> {
        for name in &names {
            let rule: INetFwRule =
                unsafe { CoCreateInstance(&NetFwRule, None, CLSCTX_INPROC_SERVER)? };
            unsafe {
                rule.SetName(&BSTR::from(name))?;
                rule.SetGrouping(&BSTR::from(&group))?;
                rule.SetProfiles(7)?;
                rule.SetProtocol(6)?;
                rule.SetLocalPorts(&BSTR::from("9"))?;
                rule.SetRemoteAddresses(&BSTR::from("127.0.0.1"))?;
                rule.SetDirection(NET_FW_RULE_DIR_IN)?;
                rule.SetAction(NET_FW_ACTION_BLOCK)?;
                rules.Add(&rule)?;
            }
            created.push(name);
        }
        anyhow::ensure!(!native::firewall_group(&group, None)?);
        unsafe {
            rules
                .Item(&BSTR::from(&names[0]))?
                .SetEnabled(true.into())?
        };
        anyhow::ensure!(native::firewall_group(&group, None)?);
        anyhow::ensure!(!native::firewall_group_all(&group)?);
        anyhow::ensure!(!native::firewall_group(&group, Some(false))?);
        for name in &names {
            anyhow::ensure!(!unsafe { rules.Item(&BSTR::from(name))?.Enabled()? }.as_bool());
        }
        anyhow::ensure!(native::firewall_group(&group, Some(true))?);
        anyhow::ensure!(native::firewall_group_all(&group)?);
        for name in &names {
            anyhow::ensure!(unsafe { rules.Item(&BSTR::from(name))?.Enabled()? }.as_bool());
        }
        Ok(())
    })();
    for name in created {
        unsafe { rules.Remove(&BSTR::from(name))? };
    }
    result
}

#[test]
fn per_user_service_repairs_keep_the_sign_in_requirement_across_retries() -> anyhow::Result<()> {
    let engine = Engine::new()?;
    let path = format!(
        r"HKCU:\Software\ZeroSecurityWindowsLogonTest{}",
        std::process::id()
    );
    let feature = Feature {
        id: "test-logon-repair".into(),
        ops: vec![Operation {
            kind: "UserServiceStart".into(),
            ..Operation::reg(&path, "Start", 2)
        }],
        ..Default::default()
    };
    engine.store.initialize()?;
    let result = (|| -> anyhow::Result<()> {
        registry::set(&path, "Start", 4, "DWord")?;
        let first = engine.execute(&feature, false, false, false)?;
        assert_eq!(first.logon, native::logon()?);
        assert_eq!(engine.check(&feature).state, Status::SignIn);
        let repeated = engine.execute(&feature, false, false, false)?;
        assert!(!repeated.changed);
        assert_eq!(repeated.logon, first.logon);
        assert_eq!(engine.check(&feature).state, Status::SignIn);
        engine
            .store
            .results
            .lock()
            .unwrap()
            .get_mut(&feature.id)
            .unwrap()
            .logon += 1;
        assert_eq!(engine.check(&feature).state, Status::Done);
        Ok(())
    })();
    registry::key(&path, false)?;
    engine.store.results.lock().unwrap().remove(&feature.id);
    engine.store.save()?;
    result
}

#[test]
fn repeated_execution_keeps_restart_pending_until_a_different_boot() -> anyhow::Result<()> {
    let mut engine = Engine::new()?;
    let root = format!(
        r"HKCU:\Software\ZeroSecurityWindowsStartupTest{}",
        std::process::id()
    );
    let f = Feature {
        id: "test-restart-retention".into(),
        ops: vec![Operation::reg(&root, "Desired", 0)],
        restart: true,
        ..Default::default()
    };
    let result = (|| -> anyhow::Result<()> {
        assert_eq!(engine.check(&f).state, Status::Ready);
        assert!(engine.execute(&f, false, false, false)?.restart);
        assert_eq!(engine.check(&f).state, Status::Restart);
        let repeated = engine.execute(&f, false, false, false)?;
        assert!(!repeated.changed);
        assert!(repeated.restart);
        assert_eq!(engine.check(&f).state, Status::Restart);
        engine.boot += 1;
        assert_eq!(engine.check(&f).state, Status::Done);
        Ok(())
    })();
    registry::key(&root, false)?;
    engine.store.results.lock().unwrap().remove(&f.id);
    engine.store.save()?;
    result
}

#[test]
fn driver_updates_require_both_update_exclusion_and_device_search_off() -> anyhow::Result<()> {
    let engine = Engine::new()?;
    let root = format!(
        r"HKCU:\Software\ZeroSecurityWindowsDriverTest{}",
        std::process::id()
    );
    let mut feature = engine.feature("driver-updates")?.clone();
    feature.id = "test-driver-updates".into();
    for op in &mut feature.ops {
        op.path = format!(r"{root}\{}", op.path.split_once('\\').unwrap().1);
    }
    let result = (|| -> anyhow::Result<()> {
        let search = feature
            .ops
            .iter()
            .find(|op| op.name == "SearchOrderConfig")
            .unwrap();
        let exclusion = feature
            .ops
            .iter()
            .find(|op| op.name == "ExcludeWUDriversInQualityUpdate")
            .unwrap();
        registry::write(search)?;
        assert_eq!(engine.check(&feature).state, Status::Ready);
        for op in &feature.ops {
            registry::write(op)?;
        }
        assert_eq!(engine.check(&feature).state, Status::Done);
        registry::set(&search.path, &search.name, 1, "DWord")?;
        assert_eq!(engine.check(&feature).state, Status::Ready);
        registry::write(search)?;
        registry::set(&exclusion.path, &exclusion.name, 0, "DWord")?;
        assert_eq!(engine.check(&feature).state, Status::Ready);
        assert_eq!(registry::read(&exclusion.path, &exclusion.name)?, json!(0));
        Ok(())
    })();
    registry::key(&root, false)?;
    result
}

#[test]
fn hibernation_off_requires_the_file_to_be_removed() {
    let op = Operation {
        kind: "Hibernate".into(),
        value: json!(0),
        ..Default::default()
    };
    assert!(crate::engine::satisfied(
        &op,
        &json!({"enabled": 0, "file": false})
    ));
    assert!(!crate::engine::satisfied(
        &op,
        &json!({"enabled": 0, "file": true})
    ));
    assert!(!crate::engine::satisfied(
        &op,
        &json!({"enabled": 1, "file": false})
    ));
}
