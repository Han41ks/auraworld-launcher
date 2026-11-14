//! Вход через Microsoft: окно входа → Xbox Live → XSTS → Minecraft.
use crate::http;
use anyhow::{anyhow, bail, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64URL, Engine};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, WebviewUrl, WebviewWindowBuilder, WindowEvent};

const AUTHORIZE: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize";
const TOKEN: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const REDIRECT: &str = "https://login.microsoftonline.com/common/oauth2/nativeclient";
const SCOPE: &str = "XboxLive.signin offline_access";

pub struct McAuth {
    pub name: String,
    pub uuid: String,
    pub xuid: String,
    pub mc_token: String,
    pub mc_expires: u64,
    pub ms_refresh: String,
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

async fn post_json(url: &str, body: Value) -> Result<(u16, Value, String)> {
    let res = http::client().post(url).header("Accept", "application/json").json(&body).send().await.map_err(http::net_err)?;
    let status = res.status().as_u16();
    let text = res.text().await.unwrap_or_default();
    let json = serde_json::from_str(&text).unwrap_or(Value::Null);
    Ok((status, json, text))
}

async fn post_form(url: &str, params: &[(&str, &str)]) -> Result<Value> {
    let body = http::query(&params.iter().map(|(k, v)| (*k, v.to_string())).collect::<Vec<_>>());
    let res = http::client()
        .post(url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(http::net_err)?;
    let status = res.status();
    let json: Value = res.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let why = json["error_description"].as_str().or(json["error"].as_str()).map(String::from).unwrap_or_else(|| status.as_u16().to_string());
        bail!("Microsoft отклонил вход: {why}");
    }
    Ok(json)
}

/// Открыть окно входа Microsoft и вернуть код авторизации.
async fn login_window(app: &AppHandle, client_id: &str, challenge: &str) -> Result<String> {
    let url = format!(
        "{AUTHORIZE}?{}",
        http::query(&[
            ("client_id", client_id.to_string()),
            ("response_type", "code".into()),
            ("redirect_uri", REDIRECT.into()),
            ("scope", SCOPE.into()),
            ("prompt", "select_account".into()),
            ("code_challenge", challenge.into()),
            ("code_challenge_method", "S256".into()),
        ])
    );
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, String>>();
    let tx = Arc::new(Mutex::new(Some(tx)));
    let (tx_nav, tx_close) = (tx.clone(), tx);

    let win = WebviewWindowBuilder::new(app, format!("ms-login-{}", now_ms()), WebviewUrl::External(url.parse()?))
        .title("Вход в Microsoft")
        .inner_size(520.0, 680.0)
        .incognito(true) // чистая сессия: всегда спрашивает аккаунт
        .on_navigation(move |u| {
            if !u.as_str().starts_with(REDIRECT) {
                return true;
            }
            let get = |k: &str| u.query_pairs().find(|(q, _)| q == k).map(|(_, v)| v.into_owned());
            let r = get("code").ok_or_else(|| get("error_description").unwrap_or_else(|| "Вход отменён".into()));
            if let Some(tx) = tx_nav.lock().unwrap().take() {
                let _ = tx.send(r);
            }
            false
        })
        .build()?;
    win.on_window_event(move |e| {
        if matches!(e, WindowEvent::Destroyed) {
            if let Some(tx) = tx_close.lock().unwrap().take() {
                let _ = tx.send(Err("Окно входа закрыто".into()));
            }
        }
    });
    let r = rx.await.unwrap_or_else(|_| Err("Окно входа закрыто".into()));
    let _ = win.close();
    r.map_err(|e| anyhow!(e))
}

/// Цепочка Xbox → Minecraft.
async fn minecraft_from_ms_token(ms_access: &str) -> Result<McAuth> {
    // 1. Xbox Live
    let (st, xbl, _) = post_json(
        "https://user.auth.xboxlive.com/user/authenticate",
        json!({
            "Properties": { "AuthMethod": "RPS", "SiteName": "user.auth.xboxlive.com", "RpsTicket": format!("d={ms_access}") },
            "RelyingParty": "http://auth.xboxlive.com", "TokenType": "JWT",
        }),
    )
    .await?;
    if st != 200 {
        bail!("Ошибка Xbox Live ({st})");
    }
    let uhs = xbl["DisplayClaims"]["xui"][0]["uhs"].as_str().unwrap_or_default().to_string();

    // 2. XSTS
    let (st, xsts, _) = post_json(
        "https://xsts.auth.xboxlive.com/xsts/authorize",
        json!({
            "Properties": { "SandboxId": "RETAIL", "UserTokens": [xbl["Token"]] },
            "RelyingParty": "rp://api.minecraftservices.com/", "TokenType": "JWT",
        }),
    )
    .await?;
    if st != 200 {
        let code = xsts["XErr"].as_u64().unwrap_or(0);
        bail!(match code {
            2148916233 => "У этого аккаунта Microsoft нет профиля Xbox. Зайди на xbox.com, создай профиль и попробуй снова.".to_string(),
            2148916235 => "Xbox Live недоступен в твоей стране.".to_string(),
            2148916236 | 2148916237 => "Аккаунту нужна проверка возраста на xbox.com.".to_string(),
            2148916238 => "Это детский аккаунт: его нужно добавить в семью Microsoft взрослым.".to_string(),
            0 => format!("Ошибка Xbox (XSTS {st})"),
            c => format!("Ошибка Xbox (XSTS {c})"),
        });
    }
    let xsts_token = xsts["Token"].as_str().unwrap_or_default();
    let xuid = xsts["DisplayClaims"]["xui"][0]["xid"].as_str().unwrap_or_default().to_string();

    // 3. Minecraft
    let (st, mc, text) = post_json(
        "https://api.minecraftservices.com/authentication/login_with_xbox",
        json!({ "identityToken": format!("XBL3.0 x={uhs};{xsts_token}") }),
    )
    .await?;
    if st != 200 {
        if st == 403 || text.to_lowercase().contains("invalid app registration") {
            bail!("Mojang ещё не одобрил приложение лаунчера. Пока одобрения нет, вход через Microsoft невозможен (заявка: aka.ms/mce-reviewappid).");
        }
        bail!("Ошибка входа в Minecraft ({st})");
    }
    let mc_token = mc["access_token"].as_str().unwrap_or_default().to_string();

    // 4. Профиль
    let res = http::client()
        .get("https://api.minecraftservices.com/minecraft/profile")
        .bearer_auth(&mc_token)
        .send()
        .await
        .map_err(http::net_err)?;
    let st = res.status().as_u16();
    if st == 404 {
        bail!("На этом аккаунте не куплен Minecraft: Java Edition.");
    }
    if !(200..300).contains(&st) {
        bail!("Не удалось получить профиль Minecraft ({st})");
    }
    let profile: Value = res.json().await.map_err(http::net_err)?;

    Ok(McAuth {
        name: profile["name"].as_str().unwrap_or_default().to_string(),
        uuid: profile["id"].as_str().unwrap_or_default().to_string(),
        xuid,
        mc_token,
        mc_expires: now_ms() + mc["expires_in"].as_u64().unwrap_or(86400) * 1000 - 60000,
        ms_refresh: String::new(),
    })
}

/// Полный вход через окно.
pub async fn login(app: &AppHandle, client_id: &str) -> Result<McAuth> {
    if client_id.is_empty() {
        bail!("В launcher-config.json не указан msClientId (ID приложения Azure).");
    }
    let mut raw = [0u8; 48];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut raw);
    let verifier = B64URL.encode(raw);
    let challenge = B64URL.encode(Sha256::digest(verifier.as_bytes()));
    let code = login_window(app, client_id, &challenge).await?;
    let tok = post_form(
        TOKEN,
        &[
            ("client_id", client_id),
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", REDIRECT),
            ("code_verifier", &verifier),
            ("scope", SCOPE),
        ],
    )
    .await?;
    let mut mc = minecraft_from_ms_token(tok["access_token"].as_str().unwrap_or_default()).await?;
    mc.ms_refresh = tok["refresh_token"].as_str().unwrap_or_default().to_string();
    Ok(mc)
}

/// Тихое обновление по сохранённому refresh-токену.
pub async fn refresh(client_id: &str, ms_refresh: &str) -> Result<McAuth> {
    let tok = post_form(
        TOKEN,
        &[("client_id", client_id), ("grant_type", "refresh_token"), ("refresh_token", ms_refresh), ("scope", SCOPE)],
    )
    .await?;
    let mut mc = minecraft_from_ms_token(tok["access_token"].as_str().unwrap_or_default()).await?;
    mc.ms_refresh = tok["refresh_token"].as_str().unwrap_or(ms_refresh).to_string();
    Ok(mc)
}
