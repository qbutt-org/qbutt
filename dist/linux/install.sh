#!/bin/sh
set -eu

bundle=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
data=${XDG_DATA_HOME:-"$HOME/.local/share"}
base="$data/qbutt"
app="$base/app"
bin="$HOME/.local/bin"

case "$data" in
    /*) ;;
    *)
        printf '%s\n' 'XDG_DATA_HOME must be absolute.' >&2
        exit 1
        ;;
esac
case "$data$HOME" in
    *'\'*|*'"'*|*'`'*|*'$'*|*'%'*)
        printf '%s\n' 'The installation path contains unsupported desktop-entry characters.' >&2
        exit 1
        ;;
esac
test -x "$bundle/qbutt" && test -f "$bundle/bundle.sha256"
test ! -L "$base" && test ! -L "$app"
mkdir -p -- "$base" "$bin" "$data/applications" "$data/icons/hicolor/scalable/apps"
test "$(realpath -- "$base")" = "$base"
exec 9>"$base/.install.lock"
flock -n 9 || {
    printf '%s\n' 'Another qbutt installation is in progress.' >&2
    exit 1
}

managed_directory() {
    test -f "$1/bundle.sha256" && test -x "$1/qbutt"
    expected=$(mktemp)
    actual=$(mktemp)
    cut -c67- "$1/bundle.sha256" | LC_ALL=C sort >"$expected"
    find "$1" -mindepth 1 ! -type d ! -path "$1/bundle.sha256" -printf '%P\n' | LC_ALL=C sort >"$actual"
    if ! cmp -s "$expected" "$actual"; then
        rm -f -- "$expected" "$actual"
        printf '%s\n' 'The application directory contains unrecognized files. Move them before reinstalling.' >&2
        return 1
    fi
    rm -f -- "$expected" "$actual"
}

if test -e "$bin/qbutt" || test -L "$bin/qbutt"; then
    test -L "$bin/qbutt" && test "$(readlink -- "$bin/qbutt")" = "$app/qbutt" || {
        printf '%s\n' 'The existing ~/.local/bin/qbutt does not belong to this installation.' >&2
        exit 1
    }
fi
if test -d "$base/.previous"; then
    test ! -L "$base/.previous"
    managed_directory "$base/.previous"
    if test -e "$app"; then
        rm -r -- "$base/.previous"
    else
        mv -- "$base/.previous" "$app"
    fi
fi
if test -e "$app"; then
    managed_directory "$app"
fi
for process in /proc/[0-9]*/exe; do
    executable=$(readlink -- "$process" 2>/dev/null || true)
    case "$executable" in
        "$app/"*)
            printf '%s\n' 'Close qbutt before installing.' >&2
            exit 1
            ;;
    esac
done
(cd -- "$bundle" && sha256sum --strict --check bundle.sha256 >/dev/null)
if test -d "$bundle/qplay"; then
    test -x "$bundle/qplay/qplay"
    "$bundle/qplay/qplay" --install
fi
stage=$(mktemp -d "$base/.install-XXXXXXXX")
cleanup() {
    if test ! -e "$app" && test -d "$base/.previous"; then
        mv -- "$base/.previous" "$app"
    fi
    if test -d "$stage"; then
        rm -r -- "$stage"
    fi
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
for entry in "$bundle"/* "$bundle"/.[!.]* "$bundle"/..?*; do
    if test ! -e "$entry" && test ! -L "$entry"; then
        continue
    fi
    if test "$(basename -- "$entry")" = qplay; then
        continue
    fi
    cp -a -- "$entry" "$stage/"
done
awk 'substr($0, 67, 6) != "qplay/"' "$bundle/bundle.sha256" >"$stage/bundle.sha256"
(cd -- "$stage" && sha256sum --strict --check bundle.sha256 >/dev/null)
managed_directory "$stage"
if test -d "$app"; then
    mv -- "$app" "$base/.previous"
fi
mv -- "$stage" "$app"
ln -sfn -- "$app/qbutt" "$bin/qbutt"
cp -- "$app/qbutt.svg" "$data/icons/hicolor/scalable/apps/org.qbutt.qbutt.svg"
cat >"$data/applications/org.qbutt.qbutt.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=qbutt
Exec="$app/qbutt" %U
Icon=org.qbutt.qbutt
Terminal=false
Categories=Network;FileTransfer;P2P;
MimeType=application/x-bittorrent;x-scheme-handler/magnet;x-scheme-handler/qbutt;
StartupWMClass=qbutt
EOF
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$data/applications"
fi
if test -d "$base/.previous"; then
    managed_directory "$base/.previous"
    rm -r -- "$base/.previous"
fi
printf 'Installed qbutt: %s\n' "$app/qbutt"
