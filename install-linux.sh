#!/usr/bin/env bash
# Build and install Gitr and its desktop icon for the current Linux user.
set -euo pipefail

if [[ "$(uname)" != "Linux" ]]; then
    echo "install-linux.sh only runs on Linux" >&2
    exit 1
fi

cd "$(dirname "$0")"
cargo build --release

data_dir="${XDG_DATA_HOME:-$HOME/.local/share}"
bin_dir="$HOME/.local/bin"
binary="${CARGO_TARGET_DIR:-target}/release/gitr"
install -Dm755 "$binary" "$bin_dir/gitr"
install -Dm644 assets/icon-256.png "$data_dir/icons/hicolor/256x256/apps/gitr.png"
install -Dm644 assets/icon-1024.png "$data_dir/icons/hicolor/1024x1024/apps/gitr.png"
install -Dm644 assets/gitr.desktop "$data_dir/applications/gitr.desktop"

# Use an absolute, quoted path so launching does not depend on the desktop's PATH.
# Desktop Entry values escape backslashes twice (value parsing, then Exec parsing).
exec_path="${bin_dir//\\/\\\\\\\\}/gitr"
exec_path="${exec_path//\"/\\\\\"}"
exec_path="${exec_path//\$/\\\\\$}"
exec_path="${exec_path//\`/\\\\\`}"
exec_path="${exec_path//%/%%}"
while IFS= read -r line; do
    if [[ "$line" == Exec=* ]]; then
        printf 'Exec="%s"\n' "$exec_path"
    else
        printf '%s\n' "$line"
    fi
done < assets/gitr.desktop > "$data_dir/applications/gitr.desktop.tmp"
mv "$data_dir/applications/gitr.desktop.tmp" "$data_dir/applications/gitr.desktop"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$data_dir/applications"
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache --ignore-theme-index "$data_dir/icons/hicolor"
fi
echo "Installed Gitr. Launch it from your application menu, or run $bin_dir/gitr."
