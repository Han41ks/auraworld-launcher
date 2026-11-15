//! Сборки: <root>/instances/<id>/instance.json + папка game.
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const OFFICIAL_ID: &str = "auraworld";
pub const CONTENT_KEYS: [&str; 3] = ["mods", "resourcepacks", "shaders"];

fn dir(root: &Path) -> PathBuf {
    root.join("instances")
}

/// Идентификатор сборки — одно имя папки, без путей.
fn check_id(id: &str) -> Result<()> {
    if id.is_empty() || id == "." || id == ".." || id.contains(['/', '\\', ':']) {
        bail!("Неверный идентификатор сборки");
    }
    Ok(())
}

pub fn game_dir(root: &Path, id: &str) -> PathBuf {
    dir(root).join(id).join("game")
}

fn normalize(mut inst: Value) -> Value {
    for k in CONTENT_KEYS {
        if !inst[k].is_array() {
            inst[k] = json!([]);
        }
    }
    if !inst["loader"].is_string() {
        inst["loader"] = json!("fabric");
    }
    inst
}

pub async fn read(root: &Path, id: &str) -> Result<Value> {
    check_id(id)?;
    let raw = tokio::fs::read(dir(root).join(id).join("instance.json")).await?;
    Ok(normalize(serde_json::from_slice(&raw)?))
}

pub async fn save(root: &Path, inst: Value) -> Result<Value> {
    let id = inst["id"].as_str().unwrap_or_default().to_string();
    check_id(&id)?;
    let d = dir(root).join(&id);
    tokio::fs::create_dir_all(&d).await?;
    tokio::fs::write(d.join("instance.json"), serde_json::to_vec_pretty(&inst)?).await?;
    Ok(inst)
}

/// Официальная сборка всегда на версии сервера.
async fn ensure_official(root: &Path, config: &Value) -> Result<Value> {
    let mut inst = match read(root, OFFICIAL_ID).await {
        Ok(i) => i,
        Err(_) => normalize(json!({
            "id": OFFICIAL_ID,
            "name": format!("{} — официальная", config["serverName"].as_str().unwrap_or("AuraWorld")),
            "official": true,
        })),
    };
    inst["mcVersion"] = config["mcVersion"].clone();
    inst["loader"] = json!("fabric");
    inst["loaderVersion"] = config["fabricLoader"].clone();
    save(root, inst).await
}

pub async fn list(root: &Path, config: &Value) -> Result<Value> {
    ensure_official(root, config).await?;
    let mut out = Vec::new();
    let mut rd = tokio::fs::read_dir(dir(root)).await?;
    while let Some(e) = rd.next_entry().await? {
        if !e.path().is_dir() {
            continue;
        }
        if let Ok(i) = read(root, &e.file_name().to_string_lossy()).await {
            out.push(i);
        }
    }
    out.sort_by_key(|i| (i["official"] != true, i["name"].as_str().unwrap_or("").to_lowercase()));
    Ok(Value::Array(out))
}

/// opts: { name, mcVersion, loader: 'fabric'|'vanilla' }
pub async fn create(root: &Path, name: &str, mc: &str, loader: &str, loader_version: Option<&str>) -> Result<Value> {
    let name = if name.trim().is_empty() { "Моя сборка" } else { name.trim() };
    let mut base = String::new();
    for c in name.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() || ('а'..='я').contains(&c) || c == 'ё' {
            base.push(c);
        } else if !base.ends_with('-') {
            base.push('-');
        }
    }
    let base = base.trim_matches('-');
    let base = if base.is_empty() { "build" } else { base };
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let id = format!("{base}-{}", base36(ms));
    save(
        root,
        normalize(json!({
            "id": id, "name": name, "official": false, "mcVersion": mc,
            "loader": if loader == "vanilla" { "vanilla" } else { "fabric" },
            "loaderVersion": loader_version,
        })),
    )
    .await
}

fn base36(mut n: u128) -> String {
    let digits = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = Vec::new();
    loop {
        out.push(digits[(n % 36) as usize]);
        n /= 36;
        if n == 0 {
            break;
        }
    }
    out.reverse();
    String::from_utf8(out).unwrap()
}

pub async fn remove(root: &Path, id: &str) -> Result<()> {
    check_id(id)?;
    if id == OFFICIAL_ID {
        bail!("Официальную сборку удалить нельзя");
    }
    let _ = tokio::fs::remove_dir_all(dir(root).join(id)).await;
    Ok(())
}

fn content_key(key: &str) -> Result<&'static str> {
    match CONTENT_KEYS.iter().find(|k| **k == key) {
        Some(k) => Ok(k),
        None => bail!("Неизвестный тип содержимого"),
    }
}

/// Добавить контент из Modrinth (с зависимостями) в сборку. key: mods | resourcepacks | shaders
pub async fn add_content(root: &Path, id: &str, key: &str, resolved: &[Value]) -> Result<Value> {
    let key = content_key(key)?;
    let mut inst = read(root, id).await?;
    let main = resolved.first().map(|v| v["projectId"].clone()).unwrap_or(Value::Null);
    for v in resolved {
        let is_main = v["projectId"] == main;
        let target = if is_main { key } else { "mods" }; // зависимости — всегда моды
        let title = crate::mods::project_title(v["projectId"].as_str().unwrap_or("")).await;
        let list = inst[target].as_array_mut().unwrap();
        list.retain(|m| m["projectId"] != v["projectId"]);
        list.push(json!({
            "projectId": v["projectId"], "title": title, "version": v["versionNumber"], "file": v["file"],
            "url": v["url"], "sha1": v["sha1"], "size": v["size"], "dependency": !is_main,
        }));
    }
    save(root, inst).await
}

pub async fn remove_content(root: &Path, id: &str, key: &str, project_id: &str) -> Result<Value> {
    let key = content_key(key)?;
    let mut inst = read(root, id).await?;
    inst[key].as_array_mut().unwrap().retain(|m| m["projectId"] != project_id);
    save(root, inst).await
}
