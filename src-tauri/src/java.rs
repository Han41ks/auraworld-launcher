//! Java для игры: поиск установленной, загрузка с Adoptium.
use crate::http::{self, Task};
use crate::Log;
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const OS: &str = if cfg!(windows) {
    "windows"
} else if cfg!(target_os = "macos") {
    "mac"
} else {
    "linux"
};
const ARCH: &str = if cfg!(target_arch = "aarch64") { "aarch64" } else { "x64" };
pub const OFFERED: [u64; 5] = [8, 11, 17, 21, 25];

fn find_java_binary(dir: &Path) -> Option<PathBuf> {
    let want = if cfg!(windows) { "javaw.exe" } else { "java" };
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if e.file_name() == want && d.file_name().map(|n| n == "bin").unwrap_or(false) {
                return Some(p);
            }
        }
    }
    None
}

fn java_dir(root: &Path, major: u64) -> PathBuf {
    root.join("runtime").join(format!("java-{major}"))
}

/// Установленные версии Java.
pub fn list_installed(root: &Path) -> Value {
    Value::Array(
        OFFERED
            .iter()
            .map(|&major| {
                let bin = find_java_binary(&java_dir(root, major));
                json!({ "major": major, "installed": bin.is_some(), "path": bin.map(|p| p.to_string_lossy().to_string()) })
            })
            .collect(),
    )
}

pub async fn remove_java(root: &Path, major: u64) -> Result<()> {
    let _ = tokio::fs::remove_dir_all(java_dir(root, major)).await;
    Ok(())
}

/// Нужная версия, а если её нет у Adoptium (например, 16) — ближайшая более новая.
pub async fn ensure_java_for(root: &Path, major: u64, log: Log<'_>) -> Result<PathBuf> {
    let mut last = anyhow!("Java не найдена");
    for m in std::iter::once(major).chain(OFFERED.into_iter().filter(|&m| m > major)) {
        match ensure_java(root, m, log).await {
            Ok(p) => return Ok(p),
            Err(e) => last = e,
        }
    }
    Err(last)
}

fn unpack(archive: &Path, dir: &Path) -> Result<()> {
    let file = std::fs::File::open(archive)?;
    if archive.extension().map(|e| e == "zip").unwrap_or(false) {
        zip::ZipArchive::new(file)?.extract(dir)?;
    } else {
        tar::Archive::new(flate2::read::GzDecoder::new(file)).unpack(dir)?;
    }
    Ok(())
}

/// Вернуть путь к Java нужной версии, скачав её при необходимости.
pub async fn ensure_java(root: &Path, major: u64, log: Log<'_>) -> Result<PathBuf> {
    let dir = java_dir(root, major);
    if let Some(p) = find_java_binary(&dir) {
        return Ok(p);
    }

    log(&format!("Скачиваю Java {major}..."), None);
    let mut asset = None;
    for image in ["jre", "jdk"] {
        let url = format!(
            "{}/assets/latest/{major}/hotspot?architecture={ARCH}&image_type={image}&os={OS}&vendor=eclipse",
            http::service("AW_ADOPTIUM_URL", "https://api.adoptium.net/v3")
        );
        if let Ok(list) = http::get_json(&url).await {
            let pkg = &list[0]["binary"]["package"];
            if pkg["link"].is_string() {
                asset = Some(pkg.clone());
                break;
            }
        }
    }
    let Some(asset) = asset else { bail!("Не удалось найти Java {major} для {OS}/{ARCH}") };
    let name = asset["name"].as_str().unwrap_or("java.tar.gz");
    let name = Path::new(name).file_name().map(|n| n.to_os_string()).unwrap_or_else(|| "java.tar.gz".into());
    let size = asset["size"].as_u64().unwrap_or(0);
    let archive = root.join("runtime").join(name);

    let got = AtomicU64::new(0);
    let task = Task {
        url: asset["link"].as_str().unwrap_or_default().to_string(),
        dest: archive.clone(),
        sha1: None,
        size: (size > 0).then_some(size),
    };
    http::download(
        &task,
        Some(&|n| {
            let g = got.fetch_add(n, Ordering::SeqCst) + n;
            if size > 0 {
                let f = (g as f64 / size as f64).min(1.0);
                log(&format!("Скачиваю Java {major}: {}%", (f * 100.0).floor()), Some(f));
            }
        }),
    )
    .await?;

    log(&format!("Распаковываю Java {major}..."), None);
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(&dir).await?;
    {
        let (a, d) = (archive.clone(), dir.clone());
        tokio::task::spawn_blocking(move || unpack(&a, &d)).await??;
    }
    let _ = tokio::fs::remove_file(&archive).await;

    let bin = find_java_binary(&dir).ok_or_else(|| anyhow!("Java распакована, но исполняемый файл не найден"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755));
    }
    Ok(bin)
}
