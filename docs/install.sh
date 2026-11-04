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
            version=1.6.4
            file=qbutt-1.6.4-linux-x64.tar.gz
            size=535090118
            sha256=e843846c04b171a34baab170238c91fd470750a0bc6e132b92ae65606dab257e
            ;;
        qplay)
            version=1.4.0
            file=qplay-linux-x64.tar.gz
            size=188806676
            sha256=ad497516286194efb304afd51a669be66bf36fa98121f5d1c7b20d8e6a52129c
            ;;
        qcatch)
            version=1.3.4
            file=qcatch-linux-x64.tar.gz
            size=118748174
            sha256=55a90612a11a30bcf302b6c35f2c0c709de591a38bf2b943a34b86216e6a6da8
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
