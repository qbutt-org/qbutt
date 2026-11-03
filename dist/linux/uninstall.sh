#!/bin/sh
set -eu

data=${XDG_DATA_HOME:-"$HOME/.local/share"}
base="$data/qbutt"
app="$base/app"
case "$data" in
    /*) ;;
    *)
        printf '%s\n' 'XDG_DATA_HOME must be absolute.' >&2
        exit 1
        ;;
esac
test ! -L "$base" && test ! -L "$app"
test "$(realpath -- "$base")" = "$base"
test -f "$app/bundle.sha256" && test -x "$app/qbutt"
exec 9>"$base/.install.lock"
flock -n 9 || {
    printf '%s\n' 'Another qbutt installation is in progress.' >&2
    exit 1
}
for process in /proc/[0-9]*/exe; do
    executable=$(readlink -- "$process" 2>/dev/null || true)
    case "$executable" in
        "$app/"*)
            printf '%s\n' 'Close qbutt before uninstalling.' >&2
            exit 1
            ;;
    esac
done
while IFS= read -r entry; do
    path=$(printf '%s' "$entry" | cut -c67-)
    case "$path" in
        ''|/*|../*|*/../*|*/..)
            printf '%s\n' 'The installation inventory contains an unsafe path.' >&2
            exit 1
            ;;
    esac
    parent=$(realpath -- "$app/$(dirname -- "$path")")
    case "$parent" in
        "$app"|"$app/"*) ;;
        *)
            printf '%s\n' 'The installation contains a directory link outside the application.' >&2
            exit 1
            ;;
    esac
    rm -f -- "$app/$path"
done <"$app/bundle.sha256"
rm -f -- "$app/bundle.sha256"
find "$app" -depth -type d -empty -delete
if test -L "$HOME/.local/bin/qbutt" && test "$(readlink -- "$HOME/.local/bin/qbutt")" = "$app/qbutt"; then
    rm -- "$HOME/.local/bin/qbutt"
fi
rm -f -- "$data/applications/org.qbutt.qbutt.desktop" "$data/icons/hicolor/scalable/apps/org.qbutt.qbutt.svg"
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$data/applications"
fi
printf '%s\n' 'qbutt was uninstalled. Settings and downloaded files were preserved.'
