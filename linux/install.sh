#!/bin/sh
# Установка AuraWorld Launcher для текущего пользователя (без sudo).
set -e
cd "$(dirname "$0")"
mkdir -p "$HOME/.local/bin" "$HOME/.local/share/applications" "$HOME/.local/share/icons/hicolor/256x256/apps"
install -m 755 auraworld-launcher "$HOME/.local/bin/auraworld-launcher"
install -m 644 auraworld-launcher.png "$HOME/.local/share/icons/hicolor/256x256/apps/auraworld-launcher.png"
cat > "$HOME/.local/share/applications/auraworld-launcher.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=AuraWorld Launcher
Comment=Лаунчер сервера AuraWorld
Exec=$HOME/.local/bin/auraworld-launcher
Icon=auraworld-launcher
Categories=Game;
Terminal=false
DESKTOP
echo "Готово. Лаунчер появился в меню приложений; из терминала: ~/.local/bin/auraworld-launcher"
if ! ldconfig -p 2>/dev/null | grep -q libwebkit2gtk-4.1; then
	echo "Не найдена библиотека WebKitGTK. В Arch: sudo pacman -S webkit2gtk-4.1"
fi
