use crate::{engine, native, preferences, registry as reg, store};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
struct MovedFile {
    original: PathBuf,
    saved: PathBuf,
}
fn folder(name: &str) -> PathBuf {
    store::root()
        .join("recovery")
        .join(native::sid().unwrap())
        .join(name)
}
fn move_path(source: &Path, target: &Path) -> Result<()> {
    let directory = source.is_dir();
    let (from, to, filter) = if directory {
        (source, target, "*")
    } else {
        (
            source.parent().unwrap(),
            target.parent().unwrap(),
            source.file_name().unwrap().to_str().unwrap(),
        )
    };
    let mut args = vec![
        from.to_str().unwrap(),
        to.to_str().unwrap(),
        filter,
        "/R:0",
        "/W:0",
        "/COPY:DAT",
        "/SL",
        "/SJ",
    ];
    if directory {
        args.extend(["/E", "/MOVE"]);
    } else {
        args.push("/MOV");
    }
    let (code, output) = native::run("robocopy.exe", &args)?;
    ensure!(
        code < 8 && !source.exists() && target.exists(),
        "File move did not complete: {}: {output}",
        source.display()
    );
    Ok(())
}
fn move_files(name: &str, paths: &[PathBuf]) -> Result<()> {
    let root = folder(name);
    fs::create_dir_all(&root)?;
    let manifest = root.join("files.json");
    let mut files: Vec<MovedFile> = if manifest.exists() {
        serde_json::from_slice(&fs::read(&manifest)?)?
    } else {
        vec![]
    };
    for original in paths {
        let saved = root
            .join(files.len().to_string())
            .join(original.file_name().unwrap());
        fs::create_dir_all(saved.parent().unwrap())?;
        files.push(MovedFile {
            original: original.clone(),
            saved: saved.clone(),
        });
        fs::write(&manifest, serde_json::to_vec_pretty(&files)?)?;
        move_path(original, &saved)?;
    }
    Ok(())
}
pub fn files_restored(name: &str) -> Result<bool> {
    files_restored_at(&folder(name))
}
fn files_restored_at(root: &Path) -> Result<bool> {
    let manifest = root.join("files.json");
    if !manifest.exists() {
        return Ok(true);
    }
    let files: Vec<MovedFile> = serde_json::from_slice(&fs::read(manifest)?)?;
    Ok(files.iter().all(|f| !f.saved.exists()))
}
pub fn taskbar_restored() -> Result<bool> {
    taskbar_restored_at(&folder("taskbar"))
}
fn taskbar_restored_at(root: &Path) -> Result<bool> {
    Ok(!root.join("values.json").exists() && files_restored_at(root)?)
}
pub fn start_pins_restored() -> Result<bool> {
    Ok(start_pins_restored_at(&folder("start")))
}
fn start_pins_restored_at(root: &Path) -> bool {
    ["start.bin", "start2.bin"]
        .iter()
        .all(|name| !root.join(name).exists())
}
fn restore_files(name: &str) -> Result<()> {
    let manifest = folder(name).join("files.json");
    if !manifest.exists() {
        return Ok(());
    }
    let files: Vec<MovedFile> = serde_json::from_slice(&fs::read(&manifest)?)?;
    for file in files {
        if !file.saved.exists() {
            continue;
        }
        ensure!(
            !file.original.exists(),
            "Restore destination already exists: {}",
            file.original.display()
        );
        fs::create_dir_all(file.original.parent().unwrap())?;
        move_path(&file.saved, &file.original)?;
    }
    fs::remove_file(manifest)?;
    Ok(())
}
pub fn desktop_files(clear: bool) -> Result<()> {
    if clear {
        move_files("desktop", &engine::desktop_items()?)
    } else {
        restore_files("desktop")
    }
}
const TASKBAND: &str = r"HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\Taskband";
pub fn taskbar_pins(clear: bool) -> Result<()> {
    let root = folder("taskbar");
    let values = root.join("values.json");
    if clear {
        fs::create_dir_all(&root)?;
        if !values.exists() {
            fs::write(
                &values,
                serde_json::to_vec(&[
                    reg::read(TASKBAND, "Favorites")?,
                    reg::read(TASKBAND, "FavoritesResolve")?,
                ])?,
            )?;
        }
        let links = PathBuf::from(std::env::var_os("APPDATA").unwrap())
            .join(r"Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar");
        let mut paths = vec![];
        if links.exists() {
            for entry in fs::read_dir(links)? {
                let path = entry?.path();
                if path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("lnk"))
                {
                    paths.push(path);
                }
            }
        }
        move_files("taskbar", &paths)?;
        for name in ["Favorites", "FavoritesResolve"] {
            reg::delete(TASKBAND, name)?;
        }
    } else {
        if values.exists() && !preferences::taskbar_pins(false)? {
            let data: Vec<serde_json::Value> = serde_json::from_slice(&fs::read(&values)?)?;
            for (name, value) in ["Favorites", "FavoritesResolve"].into_iter().zip(data) {
                ensure!(
                    crate::model::same(&reg::read(TASKBAND, name)?, &value),
                    "New taskbar pins exist; saved pins remain in {}",
                    root.display()
                );
            }
        }
        restore_files("taskbar")?;
        if values.exists() {
            let data: Vec<serde_json::Value> = serde_json::from_slice(&fs::read(&values)?)?;
            for (name, value) in ["Favorites", "FavoritesResolve"].into_iter().zip(data) {
                if value.is_null() {
                    reg::delete(TASKBAND, name)?;
                } else {
                    reg::set(TASKBAND, name, value, "Binary")?;
                }
            }
            fs::remove_file(values)?;
        }
    }
    preferences::explorer();
    Ok(())
}
pub fn start_pins(clear: bool) -> Result<()> {
    let root = folder("start");
    let local = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap())
        .join(r"Packages\Microsoft.Windows.StartMenuExperienceHost_cw5n1h2txyewy\LocalState");
    if clear {
        fs::create_dir_all(&root)?;
        for name in ["start.bin", "start2.bin"] {
            let original = local.join(name);
            let saved = root.join(name);
            if original.exists() && !saved.exists() {
                fs::copy(original, saved)?;
            }
        }
    } else {
        native::stop_process("StartMenuExperienceHost.exe")?;
        for name in ["start.bin", "start2.bin"] {
            let saved = root.join(name);
            if saved.exists() {
                fs::copy(&saved, local.join(name))?;
                fs::remove_file(saved)?;
            }
        }
    }
    Ok(())
}
pub fn compatibility(id: &str, enabled: bool) -> Result<()> {
    let saved = folder("compatibility").join(format!("{id}.sdb"));
    if enabled {
        if saved.exists() {
            native::command("sdbinst.exe", &["-q", saved.to_str().unwrap()])?;
        }
    } else {
        let path = reg::read(
            &format!(
                r"HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\InstalledSDB\{id}"
            ),
            "DatabasePath",
        )?;
        if let Some(path) = path.as_str() {
            fs::create_dir_all(saved.parent().unwrap())?;
            if !saved.exists() {
                fs::copy(Path::new(path), &saved)?;
            }
            native::command("sdbinst.exe", &["-q", "-u", "-g", id])?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pending_pin_materials_prevent_a_completed_restore() -> Result<()> {
        let root = std::env::temp_dir().join(format!("zsw-pin-state-{}", std::process::id()));
        fs::create_dir(&root)?;
        let result = (|| -> Result<()> {
            assert!(taskbar_restored_at(&root)?);
            assert!(start_pins_restored_at(&root));
            fs::write(root.join("values.json"), b"[]")?;
            assert!(!taskbar_restored_at(&root)?);
            fs::remove_file(root.join("values.json"))?;
            for name in ["start.bin", "start2.bin"] {
                fs::write(root.join(name), b"saved")?;
                assert!(!start_pins_restored_at(&root));
                fs::remove_file(root.join(name))?;
            }
            assert!(taskbar_restored_at(&root)?);
            assert!(start_pins_restored_at(&root));
            Ok(())
        })();
        for name in ["values.json", "start.bin", "start2.bin"] {
            let file = root.join(name);
            if file.exists() {
                fs::remove_file(file)?;
            }
        }
        fs::remove_dir(root)?;
        result
    }
}
