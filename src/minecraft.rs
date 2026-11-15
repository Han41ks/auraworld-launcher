//! Установка и запуск Minecraft: version.json, Fabric, библиотеки, ассеты, аргументы.
use crate::http::{self, Task};
use crate::Log;
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

fn manifest_url() -> String {
    http::service("AW_MOJANG_MANIFEST", "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json")
}
fn resources_url() -> String {
    http::service("AW_RESOURCES_URL", "https://resources.download.minecraft.net")
}
fn fabric_meta() -> String {
    http::service("AW_FABRIC_META", "https://meta.fabricmc.net/v2")
}

pub const OS_NAME: &str = if cfg!(windows) {
    "windows"
} else if cfg!(target_os = "macos") {
    "osx"
} else {
    "linux"
};
const IS_X86: bool = cfg!(target_arch = "x86");

// ---------------- Правила (rules) из version.json ----------------

fn rule_matches(rule: &Value, features: &HashMap<&str, bool>) -> bool {
    if let Some(os) = rule.get("os") {
        if let Some(name) = os.get("name").and_then(Value::as_str) {
            if name != OS_NAME {
                return false;
            }
        }
        if os.get("arch").and_then(Value::as_str) == Some("x86") && !IS_X86 {
            return false;
        }
    }
    if let Some(f) = rule.get("features").and_then(Value::as_object) {
        for (k, v) in f {
            let have = features.get(k.as_str()).copied().unwrap_or(false);
            if Some(have) != v.as_bool() {
                return false;
            }
        }
    }
    true
}

pub fn allowed(rules: Option<&Value>, features: &HashMap<&str, bool>) -> bool {
    let Some(rules) = rules.and_then(Value::as_array) else { return true };
    if rules.is_empty() {
        return true;
    }
    let mut result = false;
    for r in rules {
        if rule_matches(r, features) {
            result = r.get("action").and_then(Value::as_str) == Some("allow");
        }
    }
    result
}

// ---------------- Maven ----------------

pub fn maven_path(name: &str) -> String {
    let (coord, ext) = match name.split_once('@') {
        Some((c, e)) => (c, e),
        None => (name, "jar"),
    };
    let p: Vec<&str> = coord.split(':').collect();
    let (group, artifact, version) = (p.first().copied().unwrap_or(""), p.get(1).copied().unwrap_or(""), p.get(2).copied().unwrap_or(""));
    let classifier = p.get(3).map(|c| format!("-{c}")).unwrap_or_default();
    format!("{}/{artifact}/{version}/{artifact}-{version}{classifier}.{ext}", group.replace('.', "/"))
}

fn lib_key(name: &str) -> String {
    let p: Vec<&str> = name.split('@').next().unwrap_or("").split(':').collect();
    if p.len() > 3 {
        format!("{}:{}:{}", p[0], p[1], p[3])
    } else {
        format!("{}:{}", p.first().unwrap_or(&""), p.get(1).unwrap_or(&""))
    }
}

// ---------------- Версии ----------------

async fn read_json(file: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&tokio::fs::read(file).await?)?)
}

async fn load_vanilla(root: &Path, mc: &str) -> Result<Value> {
    let file = root.join("versions").join(mc).join(format!("{mc}.json"));
    if let Ok(v) = read_json(&file).await {
        return Ok(v);
    }
    let manifest = http::get_json(&manifest_url()).await?;
    let entry = manifest["versions"]
        .as_array()
        .and_then(|a| a.iter().find(|v| v["id"] == mc))
        .ok_or_else(|| anyhow!("Версия {mc} не найдена у Mojang"))?;
    let t = Task {
        url: entry["url"].as_str().unwrap_or_default().to_string(),
        dest: file.clone(),
        sha1: entry["sha1"].as_str().map(String::from),
        size: None,
    };
    http::download(&t, None).await?;
    read_json(&file).await
}

async fn load_fabric(root: &Path, mc: &str, loader: &str) -> Result<Value> {
    let id = format!("fabric-loader-{loader}-{mc}");
    let file = root.join("versions").join(&id).join(format!("{id}.json"));
    if let Ok(v) = read_json(&file).await {
        return Ok(v);
    }
    let url = format!(
        "{}/versions/loader/{}/{}/profile/json",
        fabric_meta(),
        urlencoding::encode(mc),
        urlencoding::encode(loader)
    );
    let json = http::get_json(&url).await?;
    tokio::fs::create_dir_all(file.parent().unwrap()).await?;
    tokio::fs::write(&file, serde_json::to_vec_pretty(&json)?).await?;
    Ok(json)
}

fn arr(v: &Value) -> Vec<Value> {
    v.as_array().cloned().unwrap_or_default()
}

/// Профиль Fabric поверх ванильной версии (как inheritsFrom в официальном лаунчере).
fn merge(vanilla: &Value, fabric: &Value) -> Value {
    let mut seen = HashSet::new();
    let mut libraries = Vec::new();
    for lib in arr(&fabric["libraries"]).into_iter().chain(arr(&vanilla["libraries"])) {
        let key = lib_key(lib["name"].as_str().unwrap_or(""));
        if seen.insert(key) {
            libraries.push(lib);
        }
    }
    let mut out = vanilla.clone();
    out["id"] = fabric["id"].clone();
    out["jar"] = vanilla["id"].clone();
    if fabric["mainClass"].is_string() {
        out["mainClass"] = fabric["mainClass"].clone();
    }
    out["libraries"] = Value::Array(libraries);
    let cat = |k: &str| {
        let mut a = arr(&vanilla["arguments"][k]);
        a.extend(arr(&fabric["arguments"][k]));
        Value::Array(a)
    };
    out["arguments"] = json!({ "game": cat("game"), "jvm": cat("jvm") });
    out
}

// ---------------- Список версий ----------------

pub async fn list_versions(include_snapshots: bool) -> Result<Value> {
    let manifest = http::get_json(&manifest_url()).await?;
    let mut fabric: HashSet<String> = HashSet::new();
    if let Ok(games) = http::get_json(&format!("{}/versions/game", fabric_meta())).await {
        for g in arr(&games) {
            if let Some(v) = g["version"].as_str() {
                fabric.insert(v.to_string());
            }
        }
    }
    let out: Vec<Value> = arr(&manifest["versions"])
        .iter()
        .filter(|v| v["type"] == "release" || (include_snapshots && v["type"] == "snapshot"))
        .map(|v| json!({ "id": v["id"], "type": v["type"], "fabric": fabric.contains(v["id"].as_str().unwrap_or("")) }))
        .collect();
    Ok(Value::Array(out))
}

/// Последняя стабильная версия Fabric Loader для версии игры.
pub async fn latest_fabric_loader(mc: &str) -> Result<Option<String>> {
    let list = http::get_json(&format!("{}/versions/loader/{}", fabric_meta(), urlencoding::encode(mc))).await?;
    let list = arr(&list);
    let pick = list.iter().find(|x| x["loader"]["stable"] == true).or(list.first());
    Ok(pick.and_then(|x| x["loader"]["version"].as_str()).map(String::from))
}

// ---------------- Установка ----------------

pub struct Installed {
    pub version: Value,
    pub client_jar: PathBuf,
    pub classpath: Vec<PathBuf>,
    pub natives: Vec<PathBuf>,
    pub logging_arg: Option<String>,
    pub asset_index: String,
}

fn task(url: &str, dest: PathBuf, v: &Value) -> Task {
    Task { url: url.to_string(), dest, sha1: v["sha1"].as_str().map(String::from), size: v["size"].as_u64() }
}

/// Относительный путь из version.json — только внутрь нужной папки.
fn safe_join(base: &Path, rel: &str) -> PathBuf {
    let mut out = base.to_path_buf();
    for part in rel.split(['/', '\\']) {
        if !part.is_empty() && part != "." && part != ".." {
            out.push(part);
        }
    }
    out
}

fn collect_libraries(root: &Path, version: &Value) -> (Vec<PathBuf>, Vec<Task>, Vec<PathBuf>) {
    let lib_dir = root.join("libraries");
    let no_features = HashMap::new();
    let (mut classpath, mut tasks, mut natives) = (Vec::new(), Vec::new(), Vec::new());
    for lib in arr(&version["libraries"]) {
        if !allowed(lib.get("rules"), &no_features) {
            continue;
        }
        let art = &lib["downloads"]["artifact"];
        if let Some(p) = art["path"].as_str() {
            let dest = safe_join(&lib_dir, p);
            tasks.push(task(art["url"].as_str().unwrap_or(""), dest.clone(), art));
            classpath.push(dest);
        } else if lib.get("downloads").is_none() {
            if let Some(name) = lib["name"].as_str() {
                let p = maven_path(name);
                let mut base = lib["url"].as_str().unwrap_or("https://libraries.minecraft.net/").to_string();
                if !base.ends_with('/') {
                    base.push('/');
                }
                let dest = safe_join(&lib_dir, &p);
                tasks.push(task(&format!("{base}{p}"), dest.clone(), &lib));
                classpath.push(dest);
            }
        }
        // Старый формат нативных библиотек (classifiers)
        if let Some(cls) = lib["natives"][OS_NAME].as_str() {
            let cls = cls.replace("${arch}", if IS_X86 { "32" } else { "64" });
            let n = &lib["downloads"]["classifiers"][cls.as_str()];
            if let Some(p) = n["path"].as_str() {
                let dest = safe_join(&lib_dir, p);
                tasks.push(task(n["url"].as_str().unwrap_or(""), dest.clone(), n));
                natives.push(dest);
            }
        }
    }
    (classpath, tasks, natives)
}

pub async fn install(root: &Path, mc: &str, loader: Option<&str>, log: Log<'_>) -> Result<Installed> {
    log("Получаю данные версии...", None);
    let vanilla = load_vanilla(root, mc).await?;
    let vid = vanilla["id"].as_str().unwrap_or(mc).to_string();
    let mut version = match loader {
        Some(l) => merge(&vanilla, &load_fabric(root, mc, l).await?),
        None => {
            let mut v = vanilla.clone();
            v["jar"] = vanilla["id"].clone();
            v
        }
    };
    if !version["arguments"].is_object() {
        // JVM-аргументы для старых версий, где их нет в version.json
        version["arguments"] = json!({ "game": [], "jvm": ["-Djava.library.path=${natives_directory}", "-cp", "${classpath}"] });
    }

    let client_jar = root.join("versions").join(&vid).join(format!("{vid}.jar"));
    let (classpath, lib_tasks, natives) = collect_libraries(root, &version);
    let client = &vanilla["downloads"]["client"];
    let mut tasks = vec![task(client["url"].as_str().context("нет ссылки на клиент")?, client_jar.clone(), client)];
    tasks.extend(lib_tasks);
    http::download_all(tasks, 12, &|d, t| log(&format!("Библиотеки: {d}/{t}"), Some(d as f64 / t as f64))).await?;

    // Ассеты
    let ai = &vanilla["assetIndex"];
    let ai_id = ai["id"].as_str().context("нет индекса ресурсов")?.to_string();
    let index_file = root.join("assets").join("indexes").join(format!("{ai_id}.json"));
    http::download(&task(ai["url"].as_str().unwrap_or(""), index_file.clone(), &json!({ "sha1": ai["sha1"] })), None).await?;
    let index = read_json(&index_file).await?;
    let objects_dir = root.join("assets").join("objects");
    let resources = resources_url();
    let mut seen = HashSet::new();
    let asset_tasks: Vec<Task> = index["objects"]
        .as_object()
        .map(|o| {
            o.values()
                .filter_map(|o| {
                    let hash = o["hash"].as_str()?;
                    if hash.len() < 2 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) || !seen.insert(hash.to_string()) {
                        return None;
                    }
                    Some(Task {
                        url: format!("{resources}/{}/{hash}", &hash[..2]),
                        dest: objects_dir.join(&hash[..2]).join(hash),
                        sha1: Some(hash.to_string()),
                        size: o["size"].as_u64(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    http::download_all(asset_tasks, 24, &|d, t| log(&format!("Ресурсы игры: {d}/{t}"), Some(d as f64 / t as f64))).await?;

    // Конфиг логов
    let mut logging_arg = None;
    let lc = &vanilla["logging"]["client"];
    if let (Some(id), Some(url)) = (lc["file"]["id"].as_str(), lc["file"]["url"].as_str()) {
        let lf = safe_join(&root.join("assets").join("log_configs"), id);
        http::download(&task(url, lf.clone(), &json!({ "sha1": lc["file"]["sha1"] })), None).await?;
        if let Some(a) = lc["argument"].as_str() {
            logging_arg = Some(a.replace("${path}", &lf.to_string_lossy()));
        }
    }

    Ok(Installed { version, client_jar, classpath, natives, logging_arg, asset_index: ai_id })
}

// ---------------- Запуск ----------------

/// Подставить ${переменные}; неизвестные заменяются пустой строкой.
fn subst(s: &str, vars: &HashMap<&str, String>) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("${") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        match after.find('}') {
            Some(j) if after[..j].chars().all(|c| c.is_ascii_alphabetic() || c == '_') && j > 0 => {
                if let Some(v) = vars.get(&after[..j]) {
                    out.push_str(v);
                }
                rest = &after[j + 1..];
            }
            _ => {
                out.push_str("${");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn expand_args(list: &Value, vars: &HashMap<&str, String>, features: &HashMap<&str, bool>) -> Vec<String> {
    let mut out = Vec::new();
    for a in arr(list) {
        if let Some(s) = a.as_str() {
            out.push(s.to_string());
        } else if allowed(a.get("rules"), features) {
            match &a["value"] {
                Value::Array(v) => out.extend(v.iter().filter_map(Value::as_str).map(String::from)),
                Value::String(s) => out.push(s.clone()),
                _ => {}
            }
        }
    }
    out.iter().map(|s| subst(s, vars)).collect()
}

fn extract_natives(jars: &[PathBuf], dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    for jar in jars {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(jar)?)?;
        for i in 0..zip.len() {
            let mut e = zip.by_index(i)?;
            if e.is_dir() || e.name().starts_with("META-INF") {
                continue;
            }
            let Some(name) = Path::new(e.name()).file_name().map(|n| n.to_os_string()) else { continue };
            let mut out = std::fs::File::create(dir.join(name))?;
            std::io::copy(&mut e, &mut out)?;
        }
    }
    Ok(())
}

pub struct Account {
    pub kind: String,
    pub name: String,
    pub uuid: String,
    pub token: String,
    pub xuid: String,
    pub client_id: String,
    pub jvm_extra: Vec<String>,
}

pub struct LaunchOpts<'a> {
    pub root: &'a Path,
    pub game_dir: &'a Path,
    pub java: &'a Path,
    pub installed: &'a Installed,
    pub account: &'a Account,
    pub memory_mb: u64,
    pub server: &'a str,
}

/// Запустить игру. Возвращает процесс; вывод читает вызывающий.
pub async fn launch(o: LaunchOpts<'_>) -> Result<tokio::process::Child> {
    let inst = o.installed;
    let version = &inst.version;
    let sep = if cfg!(windows) { ";" } else { ":" };
    let vid = version["id"].as_str().unwrap_or("game");
    let natives_dir = o.root.join("natives").join(vid);
    {
        let (jars, dir) = (inst.natives.clone(), natives_dir.clone());
        tokio::task::spawn_blocking(move || extract_natives(&jars, &dir)).await??;
    }
    tokio::fs::create_dir_all(o.game_dir).await?;

    let features: HashMap<&str, bool> = HashMap::from([
        ("is_demo_user", false),
        ("has_custom_resolution", false),
        ("is_quick_play_multiplayer", !o.server.is_empty()),
    ]);
    let s = |p: &Path| p.to_string_lossy().to_string();
    let classpath = inst.classpath.iter().chain(std::iter::once(&inst.client_jar)).map(|p| s(p)).collect::<Vec<_>>().join(sep);
    let token = if o.account.token.is_empty() { "0".to_string() } else { o.account.token.clone() };
    let vars: HashMap<&str, String> = HashMap::from([
        ("auth_player_name", o.account.name.clone()),
        ("version_name", vid.to_string()),
        ("game_directory", s(o.game_dir)),
        ("assets_root", s(&o.root.join("assets"))),
        ("assets_index_name", inst.asset_index.clone()),
        ("auth_uuid", o.account.uuid.clone()),
        ("auth_access_token", token),
        ("clientid", o.account.client_id.clone()),
        ("auth_xuid", o.account.xuid.clone()),
        ("user_type", o.account.kind.clone()),
        ("version_type", "release".to_string()),
        ("natives_directory", s(&natives_dir)),
        ("launcher_name", "AuraWorldLauncher".to_string()),
        ("launcher_version", env!("CARGO_PKG_VERSION").to_string()),
        ("classpath", classpath),
        ("classpath_separator", sep.to_string()),
        ("library_directory", s(&o.root.join("libraries"))),
        ("quickPlayMultiplayer", o.server.to_string()),
    ]);

    let mut args = vec![format!("-Xmx{}M", o.memory_mb), format!("-Xms{}M", o.memory_mb.min(1024))];
    args.extend(o.account.jvm_extra.iter().cloned());
    if let Some(l) = &inst.logging_arg {
        args.push(l.clone());
    }
    args.extend(expand_args(&version["arguments"]["jvm"], &vars, &features));
    args.push(version["mainClass"].as_str().context("нет mainClass")?.to_string());
    let game = expand_args(&version["arguments"]["game"], &vars, &features);
    if game.is_empty() {
        let legacy = version["minecraftArguments"].as_str().unwrap_or("");
        args.extend(legacy.split(' ').filter(|x| !x.is_empty()).map(|x| subst(x, &vars)));
    } else {
        args.extend(game);
    }

    let mut cmd = tokio::process::Command::new(o.java);
    cmd.args(&args)
        .current_dir(o.game_dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: без чёрного окна консоли
    #[cfg(unix)]
    cmd.process_group(0); // игра живёт отдельно от лаунчера
    cmd.spawn().map_err(|e| anyhow!("Ошибка запуска: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maven() {
        assert_eq!(maven_path("net.fabricmc:fabric-loader:0.19.5"), "net/fabricmc/fabric-loader/0.19.5/fabric-loader-0.19.5.jar");
        assert_eq!(maven_path("org.lwjgl:lwjgl:3.3.3:natives-linux"), "org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-linux.jar");
        assert_eq!(maven_path("a.b:c:1@zip"), "a/b/c/1/c-1.zip");
    }

    #[test]
    fn substitution() {
        let vars = HashMap::from([("a", "1".to_string())]);
        assert_eq!(subst("x${a}y${b}z", &vars), "x1yz");
        assert_eq!(subst("-Dp=${a}", &vars), "-Dp=1");
        assert_eq!(subst("плохо ${", &vars), "плохо ${");
    }

    #[test]
    fn rules() {
        let f = HashMap::new();
        assert!(allowed(None, &f));
        let other = if OS_NAME == "linux" { "windows" } else { "linux" };
        assert!(allowed(Some(&json!([{ "action": "allow", "os": { "name": OS_NAME } }])), &f));
        assert!(!allowed(Some(&json!([{ "action": "allow", "os": { "name": other } }])), &f));
        assert!(!allowed(Some(&json!([{ "action": "allow" }, { "action": "disallow", "os": { "name": OS_NAME } }])), &f));
        assert!(!allowed(Some(&json!([{ "action": "allow", "features": { "is_demo_user": true } }])), &f));
    }

    #[test]
    fn merging() {
        let v = json!({ "id": "26.3", "mainClass": "V", "libraries": [{ "name": "a:b:1" }, { "name": "c:d:1" }], "arguments": { "game": ["g"], "jvm": ["j"] } });
        let f = json!({ "id": "fabric", "mainClass": "F", "libraries": [{ "name": "a:b:2" }], "arguments": { "game": [], "jvm": ["fj"] } });
        let m = merge(&v, &f);
        assert_eq!(m["mainClass"], "F");
        assert_eq!(m["jar"], "26.3");
        assert_eq!(m["libraries"].as_array().unwrap().len(), 2);
        assert_eq!(m["libraries"][0]["name"], "a:b:2");
        assert_eq!(m["arguments"]["jvm"], json!(["j", "fj"]));
    }
}
