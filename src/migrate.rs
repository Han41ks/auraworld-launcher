//! Перенос сборки на другую версию игры.
use crate::{http, instances, minecraft, mods};
use anyhow::{bail, Result};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

// Категории, которые не помогают искать замену (загрузчики и слишком общие)
const GENERIC: [&str; 10] = ["fabric", "forge", "neoforge", "quilt", "library", "minecraft", "iris", "optifine", "canvas", "vanilla"];
const KEY_TYPE: [(&str, &str); 3] = [("mods", "mod"), ("resourcepacks", "resourcepack"), ("shaders", "shader")];

/// Что переносим: свои моды/ресурспаки/шейдеры (без автоматических зависимостей) + официальная сборка.
fn collect(inst: &Value, manifest: &Value) -> Vec<Value> {
    let mut items = Vec::new();
    for (key, kind) in KEY_TYPE {
        for m in inst[key].as_array().into_iter().flatten() {
            if m["dependency"] != true {
                items.push(json!({ "id": m["projectId"], "title": m["title"], "from": m["version"].as_str().unwrap_or(""), "key": key, "type": kind }));
            }
        }
    }
    if inst["official"] == true {
        for e in manifest["officialPack"].as_array().into_iter().flatten() {
            if e["modrinth"].is_string() {
                items.push(json!({ "id": e["modrinth"], "title": e["name"], "from": "", "key": "mods", "type": "mod" }));
            }
        }
    }
    items
}

async fn alternatives(project_id: &str, kind: &str, target: &str) -> Vec<Value> {
    let r: Result<Vec<Value>> = async {
        let p = http::get_json(&format!("{}/project/{}", mods::modrinth(), urlencoding::encode(project_id))).await?;
        let cats: Vec<&str> = p["categories"].as_array().into_iter().flatten().filter_map(Value::as_str).filter(|c| !GENERIC.contains(c)).take(2).collect();
        let res = mods::search(&json!({ "type": kind, "mcVersion": target, "categories": cats, "sort": "downloads", "query": "" })).await?;
        Ok(res["hits"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|h| h["projectId"] != p["id"])
            .take(3)
            .map(|h| json!({ "projectId": h["projectId"], "title": h["title"], "icon": h["icon"], "downloads": h["downloads"], "description": h["description"] }))
            .collect())
    }
    .await;
    r.unwrap_or_default()
}

/// Анализ: для каждого мода — есть ли версия под target, и замены для отсутствующих.
pub async fn analyze(
    root: &Path,
    manifest: &Value,
    instance_id: &str,
    target: &str,
    on_progress: &(dyn Fn(usize, usize) + Send + Sync),
) -> Result<Value> {
    let inst = instances::read(root, instance_id).await?;
    let items = collect(&inst, manifest);
    let fabric = inst["loader"] != "vanilla";
    let loader_version = if fabric { minecraft::latest_fabric_loader(target).await.ok().flatten() } else { None };
    let fabric_ok = !fabric || loader_version.is_some();

    let total = items.len();
    let done = AtomicUsize::new(0);
    let done = &done;
    let results: Vec<Value> = futures_util::stream::iter(items.into_iter().map(|mut it| async move {
        let (id, kind) = (it["id"].as_str().unwrap_or("").to_string(), it["type"].as_str().unwrap_or("mod").to_string());
        let mut best = None;
        if fabric_ok || kind != "mod" {
            if let Ok(list) = mods::versions(&id, target, &kind).await {
                best = list.iter().find(|v| v["type"] == "release").or(list.first()).cloned();
            }
        }
        match &best {
            Some(b) => {
                it["status"] = json!("ok");
                it["to"] = b["versionNumber"].clone();
                it["versionId"] = b["versionId"].clone();
            }
            None => {
                it["status"] = json!("missing");
                it["to"] = Value::Null;
                it["versionId"] = Value::Null;
                it["alternatives"] = Value::Array(alternatives(&id, &kind, target).await);
            }
        }
        on_progress(done.fetch_add(1, Ordering::SeqCst) + 1, total);
        it
    }))
    .buffered(4) // не заваливаем Modrinth запросами; порядок сохраняется
    .collect()
    .await;

    Ok(json!({ "target": target, "loader": inst["loader"], "loaderVersion": loader_version, "fabricOk": fabric_ok, "items": results, "name": inst["name"] }))
}

/// Перенос: создать копию сборки на target с найденными версиями и выбранными заменами.
pub async fn apply(
    root: &Path,
    instance_id: &str,
    target: &str,
    plan: &Value,
    on_progress: &(dyn Fn(&str, Option<f64>) + Send + Sync),
) -> Result<Value> {
    let inst = instances::read(root, instance_id).await?;
    let loader = if inst["loader"] == "vanilla" { "vanilla" } else { "fabric" };
    let loader_version = if loader == "fabric" { minecraft::latest_fabric_loader(target).await? } else { None };
    if loader == "fabric" && loader_version.is_none() {
        bail!("Fabric пока не поддерживает {target}");
    }

    let name = format!("{} ({target})", inst["name"].as_str().unwrap_or("Сборка"));
    let copy = instances::create(root, &name, target, loader, loader_version.as_deref()).await?;
    let copy_id = copy["id"].as_str().unwrap_or_default().to_string();

    // (проект, ключ, тип, версия)
    let mut install: Vec<(String, String, String, Option<String>)> = Vec::new();
    let s = |v: &Value| v.as_str().unwrap_or("").to_string();
    for i in plan["items"].as_array().into_iter().flatten().filter(|i| i["status"] == "ok") {
        install.push((s(&i["id"]), s(&i["key"]), s(&i["type"]), i["versionId"].as_str().map(String::from)));
    }
    for c in plan["chosen"].as_array().into_iter().flatten() {
        install.push((s(&c["projectId"]), s(&c["key"]), s(&c["type"]), None));
    }

    let mut failed = Vec::new();
    let total = install.len();
    for (n, (id, key, kind, version)) in install.iter().enumerate() {
        on_progress(&format!("Ставлю {}/{total}", n + 1), Some((n + 1) as f64 / total as f64));
        let r: Result<()> = async {
            let resolved = mods::resolve_with_deps(id, target, kind, version.as_deref()).await?;
            instances::add_content(root, &copy_id, key, &resolved).await?;
            Ok(())
        }
        .await;
        if r.is_err() {
            failed.push(json!(id));
        }
    }

    // Миры и настройки — копией, оригинал остаётся как был
    if plan["copyWorlds"] == true {
        let from = instances::game_dir(root, instance_id);
        let to = instances::game_dir(root, &copy_id);
        for name in ["saves", "config", "options.txt", "servers.dat", "screenshots", "schematics"] {
            let src = from.join(name);
            if src.exists() {
                on_progress(&format!("Копирую {name}..."), None);
                let dst = to.join(name);
                tokio::task::spawn_blocking(move || http::copy_recursive(&src, &dst)).await??;
            }
        }
    }
    Ok(json!({ "instance": instances::read(root, &copy_id).await?, "failed": failed }))
}
