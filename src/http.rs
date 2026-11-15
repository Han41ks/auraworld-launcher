//! Сеть и файлы: запросы JSON, скачивание с проверкой SHA1, параллельная загрузка.
use anyhow::{anyhow, bail, Result};
use futures_util::StreamExt;
use serde_json::Value;
use sha1::{Digest, Sha1};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

pub const USER_AGENT: &str = concat!("AuraWorldLauncher/", env!("CARGO_PKG_VERSION"), " (auraworld)");
const NO_NET: &str = "Нет связи с сервером загрузки. Проверь интернет (или антивирус/фаервол) и попробуй ещё раз.";

pub fn client() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(20))
            .build()
            .expect("http client")
    })
}

/// Адрес сервиса; переменная окружения позволяет подменить его тестовым сервером.
pub fn service(env: &str, default: &str) -> String {
    std::env::var(env).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| default.to_string())
}

/// Понятный текст вместо технической ошибки сети.
pub fn net_err(e: reqwest::Error) -> anyhow::Error {
    if e.is_connect() || e.is_timeout() || e.is_request() || e.is_body() {
        anyhow!(NO_NET)
    } else {
        anyhow!(e)
    }
}

pub fn sha1_hex(data: &[u8]) -> String {
    hex::encode(Sha1::digest(data))
}

fn sha1_file_sync(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha1::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

pub async fn sha1_file(path: &Path) -> Result<String> {
    let p = path.to_path_buf();
    Ok(tokio::task::spawn_blocking(move || sha1_file_sync(&p)).await??)
}

/// Файл уже на месте и цел?
pub async fn file_ok(path: &Path, sha1: Option<&str>, size: Option<u64>) -> bool {
    let Ok(meta) = tokio::fs::metadata(path).await else { return false };
    if let Some(s) = size {
        if s > 0 && meta.len() != s {
            return false;
        }
    }
    match sha1 {
        Some(want) => sha1_file(path).await.map(|h| h == want.to_lowercase()).unwrap_or(false),
        None => meta.len() > 0,
    }
}

pub async fn get_json(url: &str) -> Result<Value> {
    let mut last = anyhow!("нет ответа");
    for attempt in 0..3u64 {
        match client().get(url).send().await {
            Ok(res) if res.status().is_success() => match res.json::<Value>().await {
                Ok(v) => return Ok(v),
                Err(e) => last = net_err(e),
            },
            Ok(res) => {
                let code = res.status().as_u16();
                last = anyhow!("HTTP {code} для {url}");
                // 4xx (кроме 429) повторять бессмысленно
                if (400..500).contains(&code) && code != 429 {
                    break;
                }
            }
            Err(e) => last = net_err(e),
        }
        tokio::time::sleep(Duration::from_millis(800 * (attempt + 1))).await;
    }
    Err(last)
}

pub async fn get_bytes(url: &str) -> Result<Vec<u8>> {
    let res = client().get(url).send().await.map_err(net_err)?;
    if !res.status().is_success() {
        bail!("HTTP {} для {url}", res.status().as_u16());
    }
    Ok(res.bytes().await.map_err(net_err)?.to_vec())
}

#[derive(Clone, Debug)]
pub struct Task {
    pub url: String,
    pub dest: PathBuf,
    pub sha1: Option<String>,
    pub size: Option<u64>,
}

async fn fetch_to(url: &str, tmp: &Path, on_bytes: Option<&(dyn Fn(u64) + Send + Sync)>) -> Result<String> {
    let res = client().get(url).send().await.map_err(net_err)?;
    if !res.status().is_success() {
        bail!("HTTP {} для {url}", res.status().as_u16());
    }
    let mut file = tokio::fs::File::create(tmp).await?;
    let mut hasher = Sha1::new();
    let mut stream = res.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(net_err)?;
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
        if let Some(f) = on_bytes {
            f(chunk.len() as u64);
        }
    }
    file.flush().await?;
    Ok(hex::encode(hasher.finalize()))
}

/// Скачать файл (с проверкой SHA1, если задан). Уже скачанные целые файлы пропускает.
pub async fn download(t: &Task, on_bytes: Option<&(dyn Fn(u64) + Send + Sync)>) -> Result<bool> {
    if file_ok(&t.dest, t.sha1.as_deref(), t.size).await {
        return Ok(false);
    }
    if let Some(dir) = t.dest.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let mut tmp = t.dest.clone().into_os_string();
    tmp.push(".part");
    let tmp = PathBuf::from(tmp);
    let mut last = anyhow!("нет ответа");
    for attempt in 0..4u64 {
        let r = async {
            let got = fetch_to(&t.url, &tmp, on_bytes).await?;
            if let Some(want) = &t.sha1 {
                if got != want.to_lowercase() {
                    bail!(
                        "Повреждённый файл (SHA1 не совпал): {}",
                        t.dest.file_name().unwrap_or_default().to_string_lossy()
                    );
                }
            }
            tokio::fs::rename(&tmp, &t.dest).await?;
            Ok(())
        }
        .await;
        match r {
            Ok(()) => return Ok(true),
            Err(e) => {
                last = e;
                let _ = tokio::fs::remove_file(&tmp).await;
                tokio::time::sleep(Duration::from_millis(1000 * (attempt + 1))).await;
            }
        }
    }
    Err(last)
}

/// Скачать много файлов параллельно.
pub async fn download_all(
    tasks: Vec<Task>,
    concurrency: usize,
    on_progress: &(dyn Fn(usize, usize) + Send + Sync),
) -> Result<()> {
    let total = tasks.len();
    let done = AtomicUsize::new(0);
    let done = &done;
    let mut stream = futures_util::stream::iter(tasks.into_iter().map(|t| async move {
        download(&t, None).await?;
        on_progress(done.fetch_add(1, Ordering::SeqCst) + 1, total);
        anyhow::Ok(())
    }))
    .buffer_unordered(concurrency.max(1));
    while let Some(r) = stream.next().await {
        r?;
    }
    Ok(())
}

/// Строка запроса из пар ключ—значение.
pub fn query(pairs: &[(&str, String)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Рекурсивная копия файла или папки.
pub fn copy_recursive(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for e in std::fs::read_dir(from)? {
            let e = e?;
            copy_recursive(&e.path(), &to.join(e.file_name()))?;
        }
    } else {
        if let Some(p) = to.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::copy(from, to)?;
    }
    Ok(())
}
