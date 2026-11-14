//! Гардероб скинов: <данные лаунчера>/skins/index.json + PNG-файлы.
//! Запись: { id, name, variant: 'classic'|'slim', file, sha1, textureUrl?, uploadedSha1? }
use crate::http;
use crate::Log;
use anyhow::{anyhow, bail, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn dir(user_data: &Path) -> PathBuf {
    user_data.join("skins")
}

fn load(user_data: &Path) -> Value {
    std::fs::read(dir(user_data).join("index.json"))
        .ok()
        .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
        .filter(|v| v["skins"].is_array())
        .unwrap_or_else(|| json!({ "active": null, "skins": [] }))
}

fn store(user_data: &Path, idx: &Value) -> Result<()> {
    std::fs::create_dir_all(dir(user_data))?;
    std::fs::write(dir(user_data).join("index.json"), serde_json::to_vec_pretty(idx)?)?;
    Ok(())
}

pub fn data_url(png: &[u8]) -> String {
    format!("data:image/png;base64,{}", B64.encode(png))
}

pub fn png_from_data_url(data: &str) -> Result<Vec<u8>> {
    let b64 = data.strip_prefix("data:image/png;base64,").ok_or_else(|| anyhow!("Скин должен быть PNG"))?;
    let buf = B64.decode(b64.trim()).map_err(|_| anyhow!("Скин должен быть PNG"))?;
    if buf.len() < 24 || &buf[..8] != b"\x89PNG\r\n\x1a\n" {
        bail!("Скин должен быть PNG");
    }
    // PNG: ширина/высота в заголовке IHDR
    let w = u32::from_be_bytes(buf[16..20].try_into().unwrap());
    let h = u32::from_be_bytes(buf[20..24].try_into().unwrap());
    if w != 64 || (h != 64 && h != 32) {
        bail!("Скин должен быть 64×64 (а этот {w}×{h})");
    }
    Ok(buf)
}

/// Имя файла скина из индекса — только внутри папки скинов.
fn skin_file(user_data: &Path, s: &Value) -> Option<PathBuf> {
    let f = s["file"].as_str()?;
    (!f.is_empty() && !f.contains(['/', '\\']) && f != "..").then(|| dir(user_data).join(f))
}

/// Список скинов с картинками для интерфейса.
pub fn list(user_data: &Path) -> Value {
    let idx = load(user_data);
    let skins: Vec<Value> = idx["skins"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let png = std::fs::read(skin_file(user_data, s)?).ok()?; // файл пропал — не показываем
            Some(json!({
                "id": s["id"], "name": s["name"], "variant": s["variant"], "data": data_url(&png),
                "uploaded": s["textureUrl"].is_string() && s["uploadedSha1"] == s["sha1"],
            }))
        })
        .collect();
    json!({ "active": idx["active"], "skins": skins })
}

/// Сохранить новый или обновить существующий скин.
pub fn save(user_data: &Path, entry: &Value) -> Result<String> {
    let buf = png_from_data_url(entry["dataUrl"].as_str().unwrap_or(""))?;
    let mut idx = load(user_data);
    let skins = idx["skins"].as_array_mut().unwrap();
    let pos = entry["id"].as_str().and_then(|id| skins.iter().position(|x| x["id"] == id));
    let pos = match pos {
        Some(p) => p,
        None => {
            skins.push(json!({ "id": uuid::Uuid::new_v4().to_string() }));
            skins.len() - 1
        }
    };
    let s = &mut skins[pos];
    let id = s["id"].as_str().unwrap_or_default().to_string();
    let name: String = entry["name"].as_str().filter(|n| !n.is_empty()).unwrap_or("Мой скин").chars().take(32).collect();
    s["name"] = json!(name);
    s["variant"] = json!(if entry["variant"] == "slim" { "slim" } else { "classic" });
    s["file"] = json!(format!("{id}.png"));
    s["sha1"] = json!(http::sha1_hex(&buf));
    std::fs::create_dir_all(dir(user_data))?;
    std::fs::write(dir(user_data).join(format!("{id}.png")), &buf)?;
    if !idx["active"].is_string() {
        idx["active"] = json!(id);
    }
    store(user_data, &idx)?;
    Ok(id)
}

pub fn remove(user_data: &Path, id: &str) -> Result<()> {
    let mut idx = load(user_data);
    let skins = idx["skins"].as_array_mut().unwrap();
    if let Some(s) = skins.iter().find(|x| x["id"] == id) {
        if let Some(f) = skin_file(user_data, s) {
            let _ = std::fs::remove_file(f);
        }
    }
    skins.retain(|x| x["id"] != id);
    let first = skins.first().map(|s| s["id"].clone()).unwrap_or(Value::Null);
    if idx["active"] == id {
        idx["active"] = first;
    }
    store(user_data, &idx)
}

pub fn set_active(user_data: &Path, id: &str) -> Result<()> {
    let mut idx = load(user_data);
    idx["active"] = json!(id);
    store(user_data, &idx)
}

/// Скин лицензионного аккаунта по нику (через открытое API Mojang).
pub async fn from_licensed(nick: &str) -> Result<Value> {
    if !crate::accounts::valid_nick(nick) {
        bail!("Неверный ник");
    }
    let profile = http::get_json(&format!("https://api.mojang.com/users/profiles/minecraft/{nick}"))
        .await
        .map_err(|_| anyhow!("Лицензионный аккаунт «{nick}» не найден"))?;
    let id = profile["id"].as_str().unwrap_or_default();
    let full = http::get_json(&format!("https://sessionserver.mojang.com/session/minecraft/profile/{id}")).await?;
    let skin = full["properties"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| p["name"] == "textures")
        .and_then(|p| B64.decode(p["value"].as_str()?).ok())
        .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
        .map(|v| v["textures"]["SKIN"].clone())
        .filter(|s| s["url"].is_string())
        .ok_or_else(|| anyhow!("У этого аккаунта стандартный скин"))?;
    let url = skin["url"].as_str().unwrap().replacen("http://", "https://", 1);
    let png = http::get_bytes(&url).await?;
    Ok(json!({
        "name": profile["name"],
        "variant": if skin["metadata"]["model"] == "slim" { "slim" } else { "classic" },
        "dataUrl": data_url(&png),
    }))
}

/// Шаблоны Стива и Алекс — из уже скачанной игры (сами файлы лаунчер не распространяет).
pub fn templates(root: &Path, mc: &str) -> Value {
    let jar = root.join("versions").join(mc).join(format!("{mc}.jar"));
    let Ok(file) = std::fs::File::open(jar) else { return json!([]) };
    let Ok(mut zip) = zip::ZipArchive::new(file) else { return json!([]) };
    let mut out = Vec::new();
    for (entry, name, variant) in [
        ("assets/minecraft/textures/entity/player/wide/steve.png", "Стив", "classic"),
        ("assets/minecraft/textures/entity/player/slim/alex.png", "Алекс", "slim"),
    ] {
        if let Ok(mut e) = zip.by_name(entry) {
            let mut buf = Vec::new();
            if std::io::Read::read_to_end(&mut e, &mut buf).is_ok() {
                out.push(json!({ "name": name, "variant": variant, "dataUrl": data_url(&buf) }));
            }
        }
    }
    Value::Array(out)
}

/// Найти в ответе сервиса ссылку вида http(s)://textures.minecraft.net/texture/<hex>.
fn find_texture_url(text: &str) -> Option<String> {
    const MARK: &str = "://textures.minecraft.net/texture/";
    let i = text.find(MARK)?;
    let hash: String = text[i + MARK.len()..].chars().take_while(|c| c.is_ascii_hexdigit()).collect();
    (!hash.is_empty()).then(|| format!("http{MARK}{hash}"))
}

/// Загрузить активный скин в MineSkin (подписанная текстура Mojang) и вернуть ссылку
/// http://textures.minecraft.net/texture/... — её примет SkinsRestorer на сервере.
pub async fn ensure_uploaded(user_data: &Path, api_key: &str, log: Log<'_>) -> Result<Option<(String, String)>> {
    let mut idx = load(user_data);
    let active = idx["active"].clone();
    let Some(pos) = idx["skins"].as_array().and_then(|a| a.iter().position(|x| x["id"] == active && !active.is_null())) else {
        return Ok(None);
    };
    let s = idx["skins"][pos].clone();
    let variant = s["variant"].as_str().unwrap_or("classic").to_string();
    if let Some(url) = s["textureUrl"].as_str() {
        if s["uploadedSha1"] == s["sha1"] {
            return Ok(Some((url.to_string(), variant)));
        }
    }

    log("Готовлю скин для сервера...", None);
    let buf = tokio::fs::read(skin_file(user_data, &s).ok_or_else(|| anyhow!("Файл скина не найден"))?).await?;
    let form = reqwest::multipart::Form::new()
        .part("file", reqwest::multipart::Part::bytes(buf).file_name("skin.png").mime_str("image/png")?)
        .text("variant", variant.clone())
        .text("visibility", "unlisted")
        .text("name", "auraworld");
    let mut req = http::client().post("https://api.mineskin.org/v2/generate").multipart(form);
    if !api_key.is_empty() {
        req = req.bearer_auth(api_key);
    }
    let res = req.send().await.map_err(http::net_err)?;
    let status = res.status().as_u16();
    let text = res.text().await.unwrap_or_default();
    if status == 429 {
        bail!("Сервис скинов просит подождать минуту — попробуй ещё раз чуть позже");
    }
    if !(200..300).contains(&status) {
        bail!("Сервис скинов вернул ошибку {status}");
    }
    let url = find_texture_url(&text).ok_or_else(|| anyhow!("Сервис скинов не вернул ссылку на текстуру"))?;
    idx["skins"][pos]["textureUrl"] = json!(url);
    idx["skins"][pos]["uploadedSha1"] = s["sha1"].clone();
    store(user_data, &idx)?;
    Ok(Some((url, variant)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texture_url() {
        let t = r#"{"skin":{"texture":{"url":{"skin":"https://textures.minecraft.net/texture/ab12cd"}}}}"#;
        assert_eq!(find_texture_url(t).unwrap(), "http://textures.minecraft.net/texture/ab12cd");
        assert!(find_texture_url("{}").is_none());
    }
}
