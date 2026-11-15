//! Modrinth, манифест модов сервера и синхронизация папок сборки.
use crate::http::{self, Task};
use crate::Log;
use anyhow::{anyhow, bail, Result};
use include_dir::{include_dir, Dir};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub fn modrinth() -> String {
    http::service("AW_MODRINTH_URL", "https://api.modrinth.com/v2")
}
const MANAGED_FILE: &str = ".auraworld-managed.json";
/// Моды, вшитые в сам лаунчер (bundled/mandatory).
static BUNDLED: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../bundled/mandatory");

fn enc(s: &str) -> String {
    urlencoding::encode(s).into_owned()
}

/// Тип контента → загрузчики версий на Modrinth и ключ в сборке.
pub fn loaders(kind: &str) -> &'static [&'static str] {
    match kind {
        "resourcepack" => &["minecraft"],
        "shader" => &["iris", "optifine"],
        _ => &["fabric"],
    }
}

pub fn content_key(kind: &str) -> &'static str {
    match kind {
        "resourcepack" => "resourcepacks",
        "shader" => "shaders",
        _ => "mods",
    }
}

// ---------------- Modrinth ----------------

/// Поиск как на сайте Modrinth. opts: { query, type, mcVersion, categories[], sort, offset }
pub async fn search(opts: &Value) -> Result<Value> {
    let kind = opts["type"].as_str().unwrap_or("mod");
    let mut facets = vec![
        json!([format!("project_type:{kind}")]),
        json!([format!("versions:{}", opts["mcVersion"].as_str().unwrap_or(""))]),
    ];
    if kind == "mod" {
        facets.push(json!(["categories:fabric"]));
    }
    for c in opts["categories"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        facets.push(json!([format!("categories:{c}")]));
    }
    let q = http::query(&[
        ("query", opts["query"].as_str().unwrap_or("").to_string()),
        ("limit", "20".into()),
        ("offset", opts["offset"].as_u64().unwrap_or(0).to_string()),
        ("index", opts["sort"].as_str().unwrap_or("relevance").to_string()),
        ("facets", Value::Array(facets).to_string()),
    ]);
    let res = http::get_json(&format!("{}/search?{q}", modrinth())).await?;
    let hits: Vec<Value> = res["hits"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|h| {
            let cats = if h["display_categories"].is_array() { &h["display_categories"] } else { &h["categories"] };
            let gallery = if h["featured_gallery"].is_string() { h["featured_gallery"].clone() } else { h["gallery"][0].clone() };
            json!({
                "projectId": h["project_id"], "slug": h["slug"], "title": h["title"], "description": h["description"],
                "icon": h["icon_url"], "downloads": h["downloads"], "follows": h["follows"], "author": h["author"],
                "updated": h["date_modified"], "categories": if cats.is_array() { cats.clone() } else { json!([]) }, "gallery": gallery,
            })
        })
        .collect();
    Ok(json!({ "total": res["total_hits"], "hits": hits }))
}

pub async fn categories(kind: &str) -> Result<Value> {
    static CACHE: Mutex<Option<Value>> = Mutex::new(None);
    let cached = CACHE.lock().unwrap().clone();
    let all = match cached {
        Some(v) => v,
        None => {
            let v = http::get_json(&format!("{}/tag/category", modrinth())).await?;
            *CACHE.lock().unwrap() = Some(v.clone());
            v
        }
    };
    let out: Vec<Value> = all
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["project_type"] == kind && c["header"] != "resolutions")
        .map(|c| json!({ "name": c["name"], "header": c["header"], "icon": c["icon"] }))
        .collect();
    Ok(Value::Array(out))
}

pub async fn project(id: &str) -> Result<Value> {
    let p = http::get_json(&format!("{}/project/{}", modrinth(), enc(id))).await?;
    let gallery: Vec<Value> = p["gallery"].as_array().into_iter().flatten().map(|g| json!({ "url": g["url"], "title": g["title"] })).collect();
    Ok(json!({
        "id": p["id"], "slug": p["slug"], "type": p["project_type"], "title": p["title"], "description": p["description"],
        "body": p["body"], "icon": p["icon_url"], "downloads": p["downloads"], "follows": p["followers"],
        "categories": p["categories"], "gallery": gallery,
        "links": {
            "source": p["source_url"], "issues": p["issues_url"], "wiki": p["wiki_url"], "discord": p["discord_url"],
            "page": format!("https://modrinth.com/{}/{}", p["project_type"].as_str().unwrap_or("mod"), p["slug"].as_str().unwrap_or("")),
        },
        "license": p["license"]["id"], "updated": p["updated"],
    }))
}

fn version_info(v: &Value) -> Option<Value> {
    let files = v["files"].as_array()?;
    let file = files.iter().find(|f| f["primary"] == true).or(files.first())?;
    let deps: Vec<Value> = v["dependencies"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|d| d["dependency_type"] == "required" && d["project_id"].is_string())
        .map(|d| d["project_id"].clone())
        .collect();
    Some(json!({
        "projectId": v["project_id"], "versionId": v["id"], "name": v["name"], "versionNumber": v["version_number"],
        "type": v["version_type"], "date": v["date_published"], "downloads": v["downloads"],
        "file": file["filename"], "url": file["url"], "sha1": file["hashes"]["sha1"], "size": file["size"], "deps": deps,
    }))
}

/// Все версии проекта, подходящие под версию игры и тип.
pub async fn versions(project_id: &str, mc: &str, kind: &str) -> Result<Vec<Value>> {
    let q = http::query(&[("game_versions", json!([mc]).to_string()), ("loaders", json!(loaders(kind)).to_string())]);
    let list = http::get_json(&format!("{}/project/{}/version?{q}", modrinth(), enc(project_id))).await?;
    Ok(list.as_array().into_iter().flatten().filter_map(version_info).collect())
}

/// Лучшая версия проекта под нашу версию игры.
async fn best_version(project: &str, mc: &str, kind: &str) -> Result<Option<Value>> {
    let list = versions(project, mc, kind).await?;
    Ok(list.iter().find(|x| x["type"] == "release").or(list.first()).cloned())
}

/// Конкретная версия (или лучшая) + все обязательные зависимости (рекурсивно). Первой идёт сам проект.
pub async fn resolve_with_deps(project: &str, mc: &str, kind: &str, version_id: Option<&str>) -> Result<Vec<Value>> {
    let mut out: Vec<Value> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    // очередь: (проект, тип, закреплённая версия, обязателен ли)
    let mut queue: Vec<(String, String, Option<String>, bool)> = vec![(project.to_string(), kind.to_string(), version_id.map(String::from), true)];
    while !queue.is_empty() {
        let (proj, kind, pinned, required) = queue.remove(0);
        if seen.contains(&proj) {
            continue;
        }
        let mut v = None;
        if let Some(id) = &pinned {
            // Версию можно указать и идентификатором, и номером
            if let Ok(j) = http::get_json(&format!("{}/project/{}/version/{}", modrinth(), enc(&proj), enc(id))).await {
                v = version_info(&j);
            }
        }
        if v.is_none() {
            v = match best_version(&proj, mc, &kind).await {
                Ok(v) => v,
                Err(e) if required => return Err(e),
                Err(_) => None,
            };
        }
        let Some(v) = v else {
            if required {
                bail!("Нет версии «{proj}» для {mc}");
            }
            continue; // зависимость без версии — пропускаем
        };
        let pid = v["projectId"].as_str().unwrap_or(&proj).to_string();
        seen.insert(proj);
        if out.iter().any(|x| x["projectId"] == pid.as_str()) {
            continue;
        }
        seen.insert(pid);
        for d in v["deps"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            if !seen.contains(d) {
                queue.push((d.to_string(), "mod".into(), None, false));
            }
        }
        out.push(v);
    }
    Ok(out)
}

pub async fn project_title(project_id: &str) -> String {
    match http::get_json(&format!("{}/project/{}", modrinth(), enc(project_id))).await {
        Ok(p) => p["title"].as_str().unwrap_or(project_id).to_string(),
        Err(_) => project_id.to_string(),
    }
}

// ---------------- Манифест сервера ----------------

/// Список модов сервера: из сети (manifestUrl), иначе последний удачно скачанный, иначе встроенный.
pub async fn load_manifest(config: &Value, user_data: &Path) -> Value {
    load_manifest_info(config, user_data).await.0
}

/// То же, но вместе с пояснением, откуда взят список (для журнала лаунчера).
pub async fn load_manifest_info(config: &Value, user_data: &Path) -> (Value, String) {
    let cache = user_data.join("manifest-cache.json");
    let Some(url) = config["manifestUrl"].as_str().filter(|u| !u.is_empty()) else {
        return (config["defaultManifest"].clone(), "вшитый в лаунчер (manifestUrl не задан)".into());
    };
    let why = match http::get_json(url).await {
        Ok(m) if m["mandatory"].is_array() => {
            let _ = tokio::fs::create_dir_all(user_data).await;
            let _ = tokio::fs::write(&cache, m.to_string()).await;
            return (m, format!("скачан из сети ({url})"));
        }
        Ok(_) => "в файле нет раздела mandatory".to_string(),
        Err(e) => e.to_string(),
    };
    if let Ok(raw) = tokio::fs::read(&cache).await {
        if let Ok(m) = serde_json::from_slice::<Value>(&raw) {
            if m["mandatory"].is_array() {
                return (m, format!("из кэша — не удалось скачать {url}: {why}"));
            }
        }
    }
    (config["defaultManifest"].clone(), format!("вшитый в лаунчер — не удалось скачать {url}: {why}"))
}

/// Файл, который должен лежать в папке сборки.
#[derive(Clone, Debug)]
pub struct Wanted {
    pub name: String,
    pub file: String,
    pub url: Option<String>,
    pub sha1: Option<String>,
    pub size: Option<u64>,
    pub project_id: Option<String>,
    pub bundled: Option<&'static [u8]>,
}

impl Wanted {
    pub fn key(&self) -> String {
        self.project_id.clone().unwrap_or_else(|| self.file.clone())
    }

    /// Из записи сборки (instance.json).
    pub fn from_content(m: &Value) -> Option<Wanted> {
        Some(Wanted {
            name: m["title"].as_str().or(m["file"].as_str())?.to_string(),
            file: safe_name(m["file"].as_str()?)?,
            url: m["url"].as_str().map(String::from),
            sha1: m["sha1"].as_str().map(String::from),
            size: m["size"].as_u64(),
            project_id: m["projectId"].as_str().map(String::from),
            bundled: None,
        })
    }
}

/// Имя файла без путей: манифест не должен писать за пределы папки сборки.
fn safe_name(name: &str) -> Option<String> {
    let n = name.rsplit(['/', '\\']).next()?.trim();
    (!n.is_empty() && n != "." && n != "..").then(|| n.to_string())
}

/// Превратить запись манифеста в список файлов.
pub async fn resolve_entry(entry: &Value, mc: &str) -> Result<Vec<Wanted>> {
    let name = entry["name"].as_str().unwrap_or("мод").to_string();
    if let Some(b) = entry["bundled"].as_str() {
        let f = BUNDLED.get_file(b).ok_or_else(|| anyhow!("В лаунчере нет файла {b}"))?;
        return Ok(vec![Wanted { name, file: b.to_string(), url: None, sha1: None, size: None, project_id: None, bundled: Some(f.contents()) }]);
    }
    if let Some(url) = entry["url"].as_str() {
        let from_url = url.split(['?', '#']).next().unwrap_or(url).rsplit('/').next().unwrap_or("");
        let from_url = urlencoding::decode(from_url).map(|s| s.into_owned()).unwrap_or_default();
        let file = entry["file"].as_str().and_then(safe_name).or_else(|| safe_name(&from_url)).ok_or_else(|| anyhow!("У мода «{name}» не указано имя файла"))?;
        return Ok(vec![Wanted {
            name,
            file,
            url: Some(url.to_string()),
            sha1: entry["sha1"].as_str().map(String::from),
            size: entry["size"].as_u64(),
            project_id: None,
            bundled: None,
        }]);
    }
    if let Some(slug) = entry["modrinth"].as_str() {
        let list = resolve_with_deps(slug, mc, "mod", entry["version"].as_str()).await?;
        return Ok(list
            .iter()
            .filter_map(|v| {
                Some(Wanted {
                    name: name.clone(),
                    file: safe_name(v["file"].as_str()?)?,
                    url: v["url"].as_str().map(String::from),
                    sha1: v["sha1"].as_str().map(String::from),
                    size: v["size"].as_u64(),
                    project_id: v["projectId"].as_str().map(String::from),
                    bundled: None,
                })
            })
            .collect());
    }
    Ok(vec![])
}

// ---------------- Синхронизация папки ----------------

/// Привести папку к нужному набору. Удаляет только то, что раньше ставил сам лаунчер:
/// файлы, положенные руками, не трогает.
pub async fn sync_mods(dir: &Path, wanted: &[Wanted], log: Log<'_>) -> Result<()> {
    tokio::fs::create_dir_all(dir).await?;
    let managed_path = dir.join(MANAGED_FILE);
    let managed: Vec<String> = tokio::fs::read(&managed_path)
        .await
        .ok()
        .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
        .and_then(|v| v["files"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()))
        .unwrap_or_default();

    let wanted_files: Vec<String> = {
        let mut seen = HashSet::new();
        wanted.iter().filter(|w| seen.insert(w.file.clone())).map(|w| w.file.clone()).collect()
    };
    for old in &managed {
        if !wanted_files.contains(old) {
            if let Some(n) = safe_name(old) {
                let _ = tokio::fs::remove_file(dir.join(n)).await;
            }
        }
    }

    let total = wanted.len();
    for (i, w) in wanted.iter().enumerate() {
        let dest: PathBuf = dir.join(&w.file);
        log(&format!("Моды: {}/{} — {}", i + 1, total, w.name), Some((i + 1) as f64 / total as f64));
        if let Some(bytes) = w.bundled {
            let same = http::sha1_file(&dest).await.map(|h| h == http::sha1_hex(bytes)).unwrap_or(false);
            if !same {
                tokio::fs::write(&dest, bytes).await?;
            }
        } else if let Some(url) = &w.url {
            http::download(&Task { url: url.clone(), dest, sha1: w.sha1.clone(), size: w.size }, None).await?;
        }
    }
    tokio::fs::write(&managed_path, serde_json::to_vec_pretty(&json!({ "files": wanted_files }))?).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(url: &str) -> Value {
        json!({ "manifestUrl": url, "defaultManifest": { "mandatory": [{ "name": "встроенный" }] } })
    }

    #[tokio::test]
    async fn manifest_without_url_uses_builtin() {
        let (m, src) = load_manifest_info(&cfg(""), Path::new("/nonexistent-aw-test")).await;
        assert_eq!(m["mandatory"][0]["name"], "встроенный");
        assert!(src.contains("не задан"), "{src}");
    }

    #[tokio::test]
    async fn manifest_unreachable_falls_back_with_reason() {
        let dir = std::env::temp_dir().join(format!("aw-test-{}", std::process::id()));
        let (m, src) = load_manifest_info(&cfg("http://127.0.0.1:9/none.json"), &dir).await;
        assert_eq!(m["mandatory"][0]["name"], "встроенный");
        assert!(src.contains("не удалось скачать"), "{src}");
        // есть кэш — берём его
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("manifest-cache.json"), r#"{"mandatory":[{"name":"кэш"}]}"#).unwrap();
        let (m, src) = load_manifest_info(&cfg("http://127.0.0.1:9/none.json"), &dir).await;
        assert_eq!(m["mandatory"][0]["name"], "кэш");
        assert!(src.starts_with("из кэша"), "{src}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
