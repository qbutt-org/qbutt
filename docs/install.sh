#!/bin/sh
set -eu

main() {
    if [ "$#" -gt 1 ]; then
        printf '%s\n' 'Choose one product: qbutt, qplay or qcatch.' >&2
        exit 2
    fi

    product=${1-qbutt}
    case "$product" in
        qbutt)
            version=1.6.11
            file=qbutt-1.6.11-linux-x64.tar.gz
            size=535638384
            sha256=4744e2d36ffc8b6c6ba98f0749ea401e373ab67f21cb5d9bac98069f1955faa7
            ;;
        qplay)
            version=1.4.2
            file=qplay-linux-x64.tar.gz
            size=188807959
            sha256=31446afc049a994c7bac5856ae89b53f9c57743fbc16ad4321724fba046b46f9
            ;;
        qcatch)
            version=1.3.8
            file=qcatch-linux-x64.tar.gz
            size=118764844
            sha256=bf36a44e58250546f69458132117162d12c3e7da5e425abfa82f4718ddd9d941
            ;;
        *)
            printf '%s\n' 'Choose qbutt, qplay or qcatch.' >&2
            exit 2
            ;;
    esac

    if [ "$(uname -s)" != Linux ] || [ "$(uname -m)" != x86_64 ]; then
        printf '%s\n' 'Linux x86-64 is required.' >&2
        exit 1
    fi
    for tool in curl tar gzip sha256sum wc mktemp mkdir rm; do
        if ! command -v "$tool" >/dev/null 2>&1; then
            printf 'Required command not found: %s\n' "$tool" >&2
            exit 1
        fi
    done

    work=$(mktemp -d)
    trap 'rm -rf -- "$work"' 0
    trap 'exit 1' HUP INT TERM
    url="https://github.com/qbutt-org/$product/releases/download/v$version/$file"
    printf 'Downloading %s %s\n' "$product" "$version" >&2
    curl --fail --show-error --location --progress-bar \
        --proto '=https' --proto-redir '=https' \
        --connect-timeout 15 --max-time 900 --retry 2 --max-filesize "$size" \
        --output "$work/archive.tar.gz" "$url"
    if [ "$(wc -c < "$work/archive.tar.gz")" -ne "$size" ]; then
        printf '%s\n' 'The downloaded archive has an unexpected size.' >&2
        exit 1
    fi
    if ! printf '%s  %s\n' "$sha256" "$work/archive.tar.gz" | sha256sum --check --status; then
        printf '%s\n' 'Download verification failed.' >&2
        exit 1
    fi

    mkdir "$work/package"
    tar --extract --gzip --no-same-owner --no-same-permissions \
        --file "$work/archive.tar.gz" --directory "$work/package"
    case "$product" in
        qbutt) (cd "$work/package" && ./install.sh) ;;
        *) (cd "$work/package" && "./$product" --install) ;;
    esac
}

main "$@"
