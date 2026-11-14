//! AuraWorld Launcher — ядро на Rust (Tauri). Интерфейс лежит в ../ui и вызывает команду `call`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod accounts;
mod http;
mod instances;
mod java;
mod microsoft;
mod migrate;
mod minecraft;
mod mods;
mod skins;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::DialogExt;
use tokio::io::AsyncBufReadExt;

/// Сообщение о ходе работы: текст и доля выполнения (0..1).
pub type Log<'a> = &'a (dyn Fn(&str, Option<f64>) + Send + Sync);

struct Ctx {
    app: AppHandle,
    config: Value,
    user_data: PathBuf,
}
static CTX: OnceLock<Ctx> = OnceLock::new();
static RUNNING: AtomicBool = AtomicBool::new(false);

fn ctx() -> &'static Ctx {
    CTX.get().expect("ядро не запущено")
}

fn send(channel: &str, payload: Value) {
    let _ = ctx().app.emit(channel, payload);
}

fn progress(channel: &'static str) -> impl Fn(&str, Option<f64>) + Send + Sync {
    move |text, fraction| send(channel, json!({ "text": text, "fraction": fraction }))
}

// ================= Настройки =================

/// Встроенные настройки можно переопределить файлом launcher-config.json:
/// рядом с AppImage, рядом с исполняемым файлом или в папке данных лаунчера.
fn load_config(user_data: &Path) -> Value {
    let mut config: Value = serde_json::from_str(include_str!("../launcher-config.json")).expect("launcher-config.json");
    let mut places: Vec<PathBuf> = Vec::new();
    if let Some(p) = std::env::var_os("APPIMAGE") {
        places.extend(Path::new(&p).parent().map(Path::to_path_buf));
    }
    if let Ok(exe) = std::env::current_exe() {
        places.extend(exe.parent().map(Path::to_path_buf));
    }
    places.push(user_data.to_path_buf());
    for dir in places {
        let file = dir.join("launcher-config.json");
        let Ok(raw) = std::fs::read(&file) else { continue };
        match serde_json::from_slice::<Value>(&raw) {
            Ok(Value::Object(over)) => {
                for (k, v) in over {
                    config[k] = v;
                }
                break;
            }
            _ => eprintln!("Ошибка в {}", file.display()),
        }
    }
    config
}

fn settings_file() -> PathBuf {
    ctx().user_data.join("settings.json")
}

fn load_settings() -> Value {
    let default_root = dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join(".auraworld");
    let mut s = json!({
        "root": default_root.to_string_lossy(),
        "memoryMb": 4096,
        "lastInstance": instances::OFFICIAL_ID,
        "hideOnLaunch": true,
        "accounts": [],
        "activeAccount": null,
        "theme": "default",
        "customBg": null,
        "onboarded": false,
        "autoJoin": true,
    });
    if let Some(Value::Object(saved)) = std::fs::read(settings_file()).ok().and_then(|raw| serde_json::from_slice(&raw).ok()) {
        for (k, v) in saved {
            s[k] = v;
        }
    }
    if !s["accounts"].is_array() {
        s["accounts"] = json!([]);
    }
    // Перенос старого ника из v0.1
    if let Some(nick) = s["nick"].as_str().map(String::from) {
        if s["accounts"].as_array().unwrap().is_empty() {
            let id = uuid::Uuid::new_v4().to_string();
            s["accounts"] = json!([{ "id": id, "type": "offline", "name": nick }]);
            s["activeAccount"] = json!(id);
        }
        s.as_object_mut().unwrap().remove("nick");
    }
    s
}

fn save_settings(s: &Value) -> Result<()> {
    let file = settings_file();
    std::fs::create_dir_all(file.parent().unwrap())?;
    std::fs::write(&file, serde_json::to_vec_pretty(s)?)?;
    #[cfg(unix)]
    {
        // В файле лежат токены входа — читать его может только владелец
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Аккаунты для интерфейса — без токенов.
fn public_accounts(s: &Value) -> Value {
    Value::Array(
        s["accounts"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|a| json!({ "id": a["id"], "type": a["type"], "name": a["name"], "active": a["id"] == s["activeAccount"] }))
            .collect(),
    )
}

/// Настройки для интерфейса.
fn public_settings(s: &Value) -> Value {
    let mut out = s.clone();
    out["accounts"] = public_accounts(s);
    out
}

fn root() -> PathBuf {
    PathBuf::from(load_settings()["root"].as_str().unwrap_or("."))
}

// Токены Microsoft хранятся в settings.json (доступ к файлу — только у владельца).
fn seal(t: &str) -> String {
    format!("raw:{t}")
}
fn unseal(t: &str) -> Result<String> {
    t.strip_prefix("raw:").map(String::from).ok_or_else(|| anyhow!("Вход в Microsoft устарел — войди в аккаунт заново"))
}

fn now_ms() -> u64 {
    microsoft::now_ms()
}

// ================= Аргументы вызова =================

struct Args(Vec<Value>);
impl Args {
    fn get(&self, i: usize) -> &Value {
        self.0.get(i).unwrap_or(&Value::Null)
    }
    fn str(&self, i: usize) -> &str {
        self.get(i).as_str().unwrap_or("")
    }
    fn opt_str(&self, i: usize) -> Option<&str> {
        self.get(i).as_str().filter(|s| !s.is_empty())
    }
}

// ================= Диалоги =================

async fn pick_file(title: &str, filter: &str, exts: &'static [&'static str]) -> Result<Option<PathBuf>> {
    let (app, title, filter) = (ctx().app.clone(), title.to_string(), filter.to_string());
    let r = tokio::task::spawn_blocking(move || app.dialog().file().set_title(title).add_filter(filter, exts).blocking_pick_file()).await?;
    Ok(r.and_then(|p| p.into_path().ok()))
}

async fn pick_folder() -> Result<Option<PathBuf>> {
    let app = ctx().app.clone();
    let r = tokio::task::spawn_blocking(move || app.dialog().file().set_can_create_directories(true).blocking_pick_folder()).await?;
    Ok(r.and_then(|p| p.into_path().ok()))
}

// ================= Команды =================

#[tauri::command]
async fn call(channel: String, args: Vec<Value>) -> Result<Value, String> {
    dispatch(&channel, Args(args)).await.map_err(|e| e.to_string())
}

async fn dispatch(channel: &str, a: Args) -> Result<Value> {
    let c = ctx();
    let config = &c.config;
    let user_data = c.user_data.as_path();
    Ok(match channel {
        // ---------- Общее ----------
        "config:get" => json!({
            "serverName": config["serverName"],
            "mcVersion": config["mcVersion"],
            "serverAddress": config["serverAddress"],
            "version": env!("CARGO_PKG_VERSION"),
            "microsoftReady": config["msClientId"].as_str().map(|s| !s.is_empty()).unwrap_or(false),
        }),
        "settings:get" => public_settings(&load_settings()),
        "settings:set" => {
            // Из интерфейса меняются только простые настройки; аккаунты и папка игры — своими командами
            const ALLOWED: [&str; 6] = ["memoryMb", "lastInstance", "hideOnLaunch", "theme", "onboarded", "autoJoin"];
            let mut s = load_settings();
            for (k, v) in a.get(0).as_object().unwrap_or(&Map::new()) {
                if ALLOWED.contains(&k.as_str()) {
                    s[k.as_str()] = v.clone();
                }
            }
            save_settings(&s)?;
            public_settings(&s)
        }
        "dialog:folder" => json!(pick_folder().await?.map(|p| p.to_string_lossy().to_string())),
        "open:external" => {
            let url = a.str(0);
            if url.starts_with("http://") || url.starts_with("https://") {
                let _ = open::that_detached(url);
            }
            Value::Null
        }

        // ---------- Оформление и первый запуск ----------
        "theme:pickCustom" => {
            let Some(src) = pick_file("Выбери картинку для фона", "Картинки", &["png", "jpg", "jpeg", "webp", "gif"]).await? else {
                return Ok(Value::Null);
            };
            let dir = user_data.join("backgrounds");
            std::fs::create_dir_all(&dir)?;
            for old in std::fs::read_dir(&dir)?.flatten() {
                let _ = std::fs::remove_file(old.path());
            }
            let ext = src.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_else(|| "png".into());
            let dest = dir.join(format!("custom-{}.{ext}", now_ms()));
            std::fs::copy(&src, &dest)?;
            let mut s = load_settings();
            s["theme"] = json!("custom");
            s["customBg"] = json!(dest.to_string_lossy());
            save_settings(&s)?;
            public_settings(&s)
        }
        "settings:moveRoot" => move_root(a.str(0)).await?,

        // ---------- Аккаунты ----------
        "accounts:addOffline" => {
            let name = a.str(0);
            if !accounts::valid_nick(name) {
                bail!("Ник: 3–16 символов, латиница, цифры и _");
            }
            let mut s = load_settings();
            let list = s["accounts"].as_array_mut().unwrap();
            let found = list
                .iter()
                .find(|x| x["type"] == "offline" && x["name"].as_str().map(|n| n.eq_ignore_ascii_case(name)).unwrap_or(false))
                .map(|x| x["id"].clone());
            let id = match found {
                Some(id) => id,
                None => {
                    let id = json!(uuid::Uuid::new_v4().to_string());
                    list.push(json!({ "id": id, "type": "offline", "name": name }));
                    id
                }
            };
            s["activeAccount"] = id;
            save_settings(&s)?;
            public_accounts(&s)
        }
        "accounts:loginMicrosoft" => {
            let r = microsoft::login(&c.app, config["msClientId"].as_str().unwrap_or("")).await?;
            let mut s = load_settings();
            let list = s["accounts"].as_array_mut().unwrap();
            let pos = match list.iter().position(|x| x["type"] == "msa" && x["uuid"] == r.uuid.as_str()) {
                Some(p) => p,
                None => {
                    list.push(json!({ "id": uuid::Uuid::new_v4().to_string(), "type": "msa" }));
                    list.len() - 1
                }
            };
            store_msa(&mut list[pos], &r);
            s["activeAccount"] = list[pos]["id"].clone();
            save_settings(&s)?;
            public_accounts(&s)
        }
        "accounts:select" => {
            let mut s = load_settings();
            s["activeAccount"] = a.get(0).clone();
            save_settings(&s)?;
            public_accounts(&s)
        }
        "accounts:remove" => {
            let mut s = load_settings();
            let id = a.get(0);
            let list = s["accounts"].as_array_mut().unwrap();
            list.retain(|x| &x["id"] != id);
            let first = list.first().map(|x| x["id"].clone()).unwrap_or(Value::Null);
            if &s["activeAccount"] == id {
                s["activeAccount"] = first;
            }
            save_settings(&s)?;
            public_accounts(&s)
        }

        // ---------- Сборки и версии ----------
        "instances:list" => instances::list(&root(), config).await?,
        "instances:create" => {
            let o = a.get(0);
            let mc = o["mcVersion"].as_str().context("Не выбрана версия игры")?;
            let loader = o["loader"].as_str().unwrap_or("fabric");
            let loader_version = if loader == "fabric" {
                Some(minecraft::latest_fabric_loader(mc).await?.ok_or_else(|| anyhow!("Fabric не поддерживает {mc}"))?)
            } else {
                None
            };
            instances::create(&root(), o["name"].as_str().unwrap_or(""), mc, loader, loader_version.as_deref()).await?
        }
        "instances:remove" => {
            instances::remove(&root(), a.str(0)).await?;
            Value::Null
        }
        "instances:open" => {
            let d = instances::game_dir(&root(), a.str(0));
            std::fs::create_dir_all(&d)?;
            let _ = open::that_detached(&d);
            Value::Null
        }
        "instances:removeContent" => instances::remove_content(&root(), a.str(0), a.str(1), a.str(2)).await?,
        "versions:list" => minecraft::list_versions(a.get(0).as_bool().unwrap_or(false)).await?,

        // ---------- Каталог Modrinth ----------
        "catalog:search" => mods::search(a.get(0)).await?,
        "catalog:categories" => mods::categories(a.opt_str(0).unwrap_or("mod")).await?,
        "catalog:project" => mods::project(a.str(0)).await?,
        "catalog:versions" => Value::Array(mods::versions(a.str(0), a.str(1), a.opt_str(2).unwrap_or("mod")).await?),
        "catalog:install" => {
            let (instance_id, project_id, kind) = (a.str(0), a.str(1), a.opt_str(2).unwrap_or("mod"));
            let root = root();
            let inst = instances::read(&root, instance_id).await?;
            if kind == "mod" && inst["loader"] != "fabric" {
                bail!("В сборку без Fabric моды поставить нельзя");
            }
            let mc = inst["mcVersion"].as_str().unwrap_or("");
            let resolved = mods::resolve_with_deps(project_id, mc, kind, a.opt_str(3)).await?;
            instances::add_content(&root, instance_id, mods::content_key(kind), &resolved).await?
        }
        "manifest:get" => {
            let m = mods::load_manifest(config, user_data).await;
            let names = |v: &Value| -> Vec<Value> { v.as_array().into_iter().flatten().map(|x| x["name"].clone()).collect() };
            json!({ "mandatory": names(&m["mandatory"]), "officialPack": names(&m["officialPack"]) })
        }

        // ---------- Скины ----------
        "skins:list" => skins::list(user_data),
        "skins:save" => {
            let id = skins::save(user_data, a.get(0))?;
            let mut out = skins::list(user_data);
            out["id"] = json!(id);
            out
        }
        "skins:remove" => {
            skins::remove(user_data, a.str(0))?;
            skins::list(user_data)
        }
        "skins:setActive" => {
            skins::set_active(user_data, a.str(0))?;
            skins::list(user_data)
        }
        "skins:fromLicensed" => skins::from_licensed(a.str(0)).await?,
        "skins:templates" => skins::templates(&root(), config["mcVersion"].as_str().unwrap_or("")),
        "skins:pickFile" => match pick_file("Выбери файл скина (PNG 64×64)", "Скин", &["png"]).await? {
            Some(p) => json!(skins::data_url(&std::fs::read(p)?)),
            None => Value::Null,
        },
        "skins:export" => {
            let png = skins::png_from_data_url(a.str(0))?;
            let name: String = a.opt_str(1).unwrap_or("skin").chars().map(|ch| if "\\/:*?\"<>|".contains(ch) { '_' } else { ch }).collect();
            let app = c.app.clone();
            let dest = tokio::task::spawn_blocking(move || {
                app.dialog().file().set_title("Сохранить скин").set_file_name(format!("{name}.png")).add_filter("PNG", &["png"]).blocking_save_file()
            })
            .await?;
            match dest.and_then(|p| p.into_path().ok()) {
                Some(p) => {
                    std::fs::write(p, png)?;
                    json!(true)
                }
                None => json!(false),
            }
        }

        // ---------- Перенос сборки на другую версию ----------
        "migrate:analyze" => {
            let manifest = mods::load_manifest(config, user_data).await;
            migrate::analyze(&root(), &manifest, a.str(0), a.str(1), &|d, t| {
                send("migrate:progress", json!({ "text": format!("Проверяю моды: {d}/{t}"), "fraction": d as f64 / t.max(1) as f64 }))
            })
            .await?
        }
        "migrate:apply" => migrate::apply(&root(), a.str(0), a.str(1), a.get(2), &progress("migrate:progress")).await?,

        // ---------- Java ----------
        "java:list" => java::list_installed(&root()),
        "java:install" => {
            let major = a.get(0).as_u64().context("Не указана версия Java")?;
            if !java::OFFERED.contains(&major) {
                bail!("Такой версии Java нет в списке");
            }
            let root = root();
            java::ensure_java(&root, major, &|text, fraction| send("java:progress", json!({ "major": major, "text": text, "fraction": fraction }))).await?;
            java::list_installed(&root)
        }
        "java:remove" => {
            let root = root();
            java::remove_java(&root, a.get(0).as_u64().context("Не указана версия Java")?).await?;
            java::list_installed(&root)
        }

        // ---------- Игра ----------
        "game:play" => {
            if RUNNING.swap(true, Ordering::SeqCst) {
                bail!("Игра уже запущена");
            }
            match play(a.str(0)).await {
                Ok(()) => json!(true),
                Err(e) => {
                    RUNNING.store(false, Ordering::SeqCst);
                    return Err(e);
                }
            }
        }
        other => bail!("Неизвестная команда: {other}"),
    })
}

fn store_msa(acc: &mut Value, r: &microsoft::McAuth) {
    acc["name"] = json!(r.name);
    acc["uuid"] = json!(r.uuid);
    acc["xuid"] = json!(r.xuid);
    acc["mcToken"] = json!(seal(&r.mc_token));
    acc["mcExpires"] = json!(r.mc_expires);
    acc["msRefresh"] = json!(seal(&r.ms_refresh));
}

/// Сменить папку игры, перенеся уже скачанное (если там что-то есть).
async fn move_root(new_root: &str) -> Result<Value> {
    let mut s = load_settings();
    let old = PathBuf::from(s["root"].as_str().unwrap_or(""));
    let new = PathBuf::from(new_root);
    if new_root.is_empty() || new == old {
        return Ok(public_settings(&s));
    }
    if !new.is_absolute() {
        bail!("Нужен полный путь к папке");
    }
    if RUNNING.load(Ordering::SeqCst) {
        bail!("Закрой игру перед переносом папки");
    }
    if new.starts_with(&old) {
        bail!("Нельзя перенести папку внутрь самой себя");
    }
    tokio::fs::create_dir_all(&new).await?;
    if old.is_dir() {
        let entries: Vec<_> = std::fs::read_dir(&old)?.flatten().collect();
        let total = entries.len().max(1);
        for (i, e) in entries.into_iter().enumerate() {
            let name = e.file_name();
            send("move:progress", json!({ "text": format!("Переношу: {}", name.to_string_lossy()), "fraction": (i + 1) as f64 / total as f64 }));
            let (from, to) = (e.path(), new.join(&name));
            // тот же диск — мгновенно; другой диск — копия и удаление
            if tokio::fs::rename(&from, &to).await.is_err() {
                tokio::task::spawn_blocking(move || -> std::io::Result<()> {
                    http::copy_recursive(&from, &to)?;
                    if from.is_dir() { std::fs::remove_dir_all(&from) } else { std::fs::remove_file(&from) }
                })
                .await??;
            }
        }
        let _ = tokio::fs::remove_dir(&old).await; // только если опустела
    }
    s["root"] = json!(new.to_string_lossy());
    save_settings(&s)?;
    Ok(public_settings(&s))
}

/// Данные аккаунта для запуска игры (с обновлением токена Microsoft при необходимости).
async fn launch_account(s: &mut Value, config: &Value) -> Result<minecraft::Account> {
    let active = s["activeAccount"].clone();
    let pos = s["accounts"]
        .as_array()
        .and_then(|l| l.iter().position(|x| x["id"] == active && !active.is_null()))
        .ok_or_else(|| anyhow!("Добавь аккаунт на вкладке «Аккаунты»"))?;
    let acc = s["accounts"][pos].clone();
    let name = acc["name"].as_str().unwrap_or_default().to_string();
    if acc["type"] == "offline" {
        return Ok(minecraft::Account {
            kind: "legacy".into(),
            uuid: accounts::offline_uuid(&name),
            name,
            token: "0".into(),
            xuid: String::new(),
            client_id: String::new(),
            jvm_extra: vec![],
        });
    }
    let client_id = config["msClientId"].as_str().unwrap_or("").to_string();
    if acc["mcExpires"].as_u64().map(|t| now_ms() > t).unwrap_or(true) {
        let r = microsoft::refresh(&client_id, &unseal(acc["msRefresh"].as_str().unwrap_or(""))?).await?;
        store_msa(&mut s["accounts"][pos], &r);
        save_settings(s)?;
    }
    let acc = &s["accounts"][pos];
    Ok(minecraft::Account {
        kind: "msa".into(),
        name: acc["name"].as_str().unwrap_or_default().to_string(),
        uuid: acc["uuid"].as_str().unwrap_or_default().to_string(),
        token: unseal(acc["mcToken"].as_str().unwrap_or(""))?,
        xuid: acc["xuid"].as_str().unwrap_or_default().to_string(),
        client_id,
        jvm_extra: vec![],
    })
}

async fn play(instance_id: &str) -> Result<()> {
    let c = ctx();
    let (config, user_data) = (&c.config, c.user_data.as_path());
    let mut s = load_settings();
    let root = PathBuf::from(s["root"].as_str().unwrap_or("."));
    let log = progress("progress");
    let log: Log = &log;
    tokio::fs::create_dir_all(&root).await?;

    let mut account = launch_account(&mut s, config).await?;
    let manifest = mods::load_manifest(config, user_data).await;
    let inst = instances::read(&root, instance_id).await?;
    let fabric = inst["loader"] == "fabric";
    let mc = inst["mcVersion"].as_str().context("В сборке не указана версия игры")?;

    let installed = minecraft::install(&root, mc, if fabric { inst["loaderVersion"].as_str() } else { None }, log).await?;
    let major = installed.version["javaVersion"]["majorVersion"].as_u64().unwrap_or(8);
    let java_path = java::ensure_java_for(&root, major, log).await?;

    // Моды сервера ставятся во все сборки на версии сервера с Fabric
    let server_mc = manifest["mcVersion"].as_str().or(config["mcVersion"].as_str()).unwrap_or("");
    let server_compatible = fabric && mc == server_mc;
    let game_dir = instances::game_dir(&root, instance_id);
    log("Проверяю моды...", None);
    let mut wanted: Vec<mods::Wanted> = Vec::new();
    if server_compatible {
        // Обязательные моды сервера — строго: без них на сервер не зайти
        for entry in manifest["mandatory"].as_array().into_iter().flatten() {
            wanted.extend(mods::resolve_entry(entry, mc).await?);
        }
        // Моды официальной сборки — мягко: если у мода пока нет версии под игру, пропускаем его
        if inst["official"] == true {
            for entry in manifest["officialPack"].as_array().into_iter().flatten() {
                match mods::resolve_entry(entry, mc).await {
                    Ok(list) => wanted.extend(list),
                    Err(e) => send("gamelog", json!(format!("[Лаунчер] Пропущен мод «{}»: {e}", entry["name"].as_str().unwrap_or("?")))),
                }
            }
        }
    }
    let content = |key: &str| -> Vec<mods::Wanted> { inst[key].as_array().into_iter().flatten().filter_map(mods::Wanted::from_content).collect() };
    wanted.extend(content("mods"));
    let mut seen = HashSet::new();
    wanted.retain(|w| seen.insert(w.key()));
    if fabric {
        mods::sync_mods(&game_dir.join("mods"), &wanted, log).await?;
    }
    mods::sync_mods(&game_dir.join("resourcepacks"), &content("resourcepacks"), log).await?;
    mods::sync_mods(&game_dir.join("shaderpacks"), &content("shaders"), log).await?;

    // Свой скин: загрузить в сервис скинов и передать клиентскому моду (он применит его через SkinsRestorer)
    if server_compatible {
        match skins::ensure_uploaded(user_data, config["mineskinApiKey"].as_str().unwrap_or(""), log).await {
            Ok(Some((url, variant))) => account.jvm_extra.push(format!("-Dauraworld.skin={url}|{variant}")),
            Ok(None) => {}
            Err(e) => send("gamelog", json!(format!("[Лаунчер] Скин не применён: {e}"))),
        }
    }

    log("Запускаю игру...", Some(1.0));
    s["lastInstance"] = json!(instance_id);
    save_settings(&s)?;
    let server = if server_compatible && s["autoJoin"] != false { config["serverAddress"].as_str().unwrap_or("") } else { "" };
    send("gamelog", json!(format!("Запуск: {} {}", java_path.file_name().unwrap_or_default().to_string_lossy(), installed.version["mainClass"].as_str().unwrap_or(""))));
    let mut child = minecraft::launch(minecraft::LaunchOpts {
        root: &root,
        game_dir: &game_dir,
        java: &java_path,
        installed: &installed,
        account: &account,
        memory_mb: s["memoryMb"].as_u64().unwrap_or(4096).max(512),
        server,
    })
    .await?;

    // Вывод игры — в «Журнал игры»
    fn pump<R: tokio::io::AsyncRead + Unpin + Send + 'static>(r: Option<R>) {
        let Some(r) = r else { return };
        tokio::spawn(async move {
            let mut r = tokio::io::BufReader::new(r);
            let mut buf = Vec::new();
            while let Ok(n) = r.read_until(b'\n', &mut buf).await {
                if n == 0 {
                    break;
                }
                send("gamelog", json!(String::from_utf8_lossy(&buf)));
                buf.clear();
            }
        });
    }
    pump(child.stdout.take());
    pump(child.stderr.take());
    tokio::spawn(async move {
        let code = child.wait().await.ok().and_then(|st| st.code());
        RUNNING.store(false, Ordering::SeqCst);
        send("game:exit", json!(code));
        if let Some(w) = ctx().app.get_webview_window("main") {
            let _ = w.show();
            let _ = w.set_focus();
        }
    });
    send("game:started", Value::Null);
    if s["hideOnLaunch"] != false {
        tokio::spawn(async {
            tokio::time::sleep(std::time::Duration::from_secs(4)).await;
            if RUNNING.load(Ordering::SeqCst) {
                if let Some(w) = ctx().app.get_webview_window("main") {
                    let _ = w.hide();
                }
            }
        });
    }
    Ok(())
}

// ================= Окно =================

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![call])
        .setup(|app| {
            // Та же папка данных, что была у прежней версии лаунчера: настройки и скины сохраняются
            let user_data = dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("AuraWorld Launcher");
            std::fs::create_dir_all(&user_data)?;
            let backgrounds = user_data.join("backgrounds");
            std::fs::create_dir_all(&backgrounds)?;
            let _ = app.asset_protocol_scope().allow_directory(&backgrounds, true);
            let config = load_config(&user_data);
            let _ = CTX.set(Ctx { app: app.handle().clone(), config, user_data });

            WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title("AuraWorld Launcher")
                .inner_size(1180.0, 740.0)
                .min_inner_size(980.0, 620.0)
                .background_color(tauri::window::Color(13, 17, 23, 255))
                // Внешние ссылки — в обычном браузере, а не внутри лаунчера
                .on_navigation(|url| {
                    let internal = matches!(url.scheme(), "tauri" | "about" | "asset" | "blob" | "data")
                        || matches!(url.host_str(), Some("tauri.localhost") | Some("asset.localhost") | Some("ipc.localhost"));
                    if !internal && matches!(url.scheme(), "http" | "https") {
                        let _ = open::that_detached(url.as_str());
                    }
                    internal
                })
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("не удалось запустить лаунчер");
}
