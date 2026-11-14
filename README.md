# AuraWorld Launcher (Rust + Tauri)

Лаунчер сервера AuraWorld. Ядро написано на Rust, окно — системный WebView
(WebView2 в Windows, WebKitGTK в Linux), интерфейс — HTML/CSS/JS из папки `ui`.

## Что где лежит

| Путь | Что это |
|---|---|
| `src-tauri/src/` | ядро: `minecraft.rs` (установка и запуск), `java.rs`, `mods.rs` (Modrinth, манифест, синхронизация), `instances.rs`, `skins.rs`, `migrate.rs`, `accounts.rs`, `microsoft.rs`, `http.rs`, `main.rs` (команды и окно) |
| `src-tauri/launcher-config.json` | настройки сервера: версия игры, адрес, ссылка на манифест модов. Вшивается в программу при сборке |
| `ui/` | интерфейс; `bridge.js` связывает его с ядром |
| `bundled/mandatory/` | моды, вшитые в сам лаунчер (запасной вариант, если манифест недоступен) |
| `auraworld-mods.json`, `mods/` | список модов сервера и свои jar-файлы — их лаунчер читает из интернета |
| `.github/workflows/build.yml` | автоматическая сборка для Windows и Linux на GitHub |

## Как выложить на GitHub и получить .exe

1. Создай на github.com новый репозиторий `auraworld-launcher` (Public — иначе лаунчер не сможет читать манифест).
2. В `src-tauri/launcher-config.json` впиши:
   - `"serverAddress"` — адрес сервера (для кнопки «Сразу зайти на сервер»);
   - `"manifestUrl": "https://raw.githubusercontent.com/ТВОЙ_НИК/auraworld-launcher/main/auraworld-mods.json"`.
3. В `auraworld-mods.json` замени `ТВОЙ_НИК` на свой ник GitHub.
4. Загрузи в репозиторий всё содержимое этой папки (кнопка «uploading an existing file» или `git push`).
   Папку `.github` тоже — без неё сборка не запустится.
5. Открой вкладку **Actions** → «Сборка лаунчера». Минут через 10–15 внизу страницы запуска
   появятся файлы **AuraWorld-Launcher-windows** (один `auraworld-launcher.exe`) и **AuraWorld-Launcher-linux**.

## Как обновлять моды у игроков без новой версии лаунчера

Лаунчер при каждом запуске игры скачивает `auraworld-mods.json` и приводит папку `mods`
официальной сборки к этому списку: докачивает новое, удаляет то, что сам ставил раньше и чего
в списке больше нет. Файлы, положенные игроком вручную, не трогает. Если GitHub недоступен —
берёт последний удачно скачанный список.

Запись в списке — одного из трёх видов:

```json
{ "name": "Sodium", "modrinth": "sodium" }
{ "name": "Lithium", "modrinth": "lithium", "version": "mc26.3-0.20.0" }
{ "name": "AuraWorld", "url": "https://raw.githubusercontent.com/НИК/auraworld-launcher/main/mods/auraworld_client-0.7.6.jar", "sha1": "..." }
```

- `modrinth` — мод с Modrinth, берётся свежая версия под версию игры вместе с обязательными зависимостями;
- `version` — закрепить конкретную версию (номер или идентификатор версии с Modrinth);
- `url` + `sha1` — свой файл. Положи jar в папку `mods/` репозитория; контрольную сумму покажет
  `certutil -hashfile файл.jar SHA1` (Windows) или `sha1sum файл.jar` (Linux).

`mandatory` ставится во все сборки на версии сервера, `officialPack` — только в официальную.
Поле `mcVersion` — версия игры, для которой действует список.

## Сборка у себя

Нужен Rust (rustup.rs). В Linux дополнительно: `webkit2gtk-4.1` и `gtk3` с заголовками
(Arch: `sudo pacman -S webkit2gtk-4.1 base-devel`).

```
cd src-tauri
cargo test --release
cargo build --release --features custom-protocol
```

Готовая программа: `src-tauri/target/release/auraworld-launcher` (`.exe` в Windows).

## Данные

Настройки, скины и фон лежат там же, где у прежней версии (Electron): `%APPDATA%\AuraWorld Launcher`
в Windows, `~/.config/AuraWorld Launcher` в Linux; игра — в `.auraworld` рядом. Обновление ничего не сбрасывает.
Файл `launcher-config.json`, положенный рядом с программой или в папку настроек, переопределяет вшитый.
