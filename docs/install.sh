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
            version=1.6.2
            file=qbutt-1.6.2-linux-x64.tar.gz
            size=531504517
            sha256=6fec1d95b0138ccece9d652d5a88f4c34b367ec3f9bff47e57903233af48e7f3
            ;;
        qplay)
            version=1.3.0
            file=qplay-linux-x64.tar.gz
            size=185230917
            sha256=53e8a11e785e13707badbdd49906a37e6483915bdf6d05c6f167967e64063cc3
            ;;
        qcatch)
            version=1.3.1
            file=qcatch-linux-x64.tar.gz
            size=118744811
            sha256=b6df4503840b2115c58c781657f84700a532fe71fb5e2536216ade8764da8bd0
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
