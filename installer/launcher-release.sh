#!/bin/sh
# Signs launcher releases and client manifests for the launcher
# (crates/p5099-launcher/src/self_update.rs, updater.rs). Needs OpenSSL 3.
#
#   installer/launcher-release.sh keygen <private key out>
#       Creates the Ed25519 signing key and prints the public key for
#       P5099_RELEASE_PUBLIC_KEY (the launcher's build-time setting; the
#       GitHub repository variable of the same name). Keep the private key
#       secret (GitHub secret LAUNCHER_SIGNING_KEY); launchers already
#       shipped only trust it.
#
#   installer/launcher-release.sh pubkey <private key>
#       Prints the public key of <private key>.
#
#   installer/launcher-release.sh sign <private key> <version> <min version> \
#       <windows exe> <macos app zip> <output dir>
#       Writes <output dir>/p5099.exe, <output dir>/p5099.app.zip and the
#       signed <output dir>/latest.txt, to be uploaded as assets of the
#       GitHub release tagged v<version>; launchers find them through
#       releases/latest/download/latest.txt. Launchers older than
#       <min version> must update before they can log in (0.0.0: none).
#
#   installer/launcher-release.sh sign-manifest <private key> <manifest>
#       Writes p5099-manifest.sig next to a p5099-manifest.txt written by
#       `p5099-assets client-manifest`. Launchers refuse an unsigned or
#       changed manifest, so sign again after every client-manifest.
set -eu

hex() {
    od -An -v -tx1 | tr -d ' \n'
}

sha256() {
    if command -v sha256sum >/dev/null; then
        sha256sum "$1" | cut -d' ' -f1
    else
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

asset() {
    printf '{"path": "%s", "size": %s, "sha256": "%s"}' \
        "$1" "$(wc -c < "$2" | tr -d ' ')" "$(sha256 "$2")"
}

sign_hex() {
    raw="$(mktemp)"
    openssl pkeyutl -sign -inkey "$1" -rawin -in "$2" -out "$raw" || { rm -f "$raw"; exit 1; }
    hex < "$raw"
    rm -f "$raw"
}

version_ok() {
    printf '%s' "$1" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'
}

case "${1:-}" in
keygen)
    [ $# -eq 2 ] || { echo "usage: $0 keygen <private key out>" >&2; exit 2; }
    [ ! -e "$2" ] || { echo "$2 already exists" >&2; exit 1; }
    (umask 077 && openssl genpkey -algorithm ed25519 -out "$2")
    echo "public key (P5099_RELEASE_PUBLIC_KEY):"
    openssl pkey -in "$2" -pubout -outform DER | tail -c 32 | hex
    echo
    ;;
pubkey)
    [ $# -eq 2 ] || { echo "usage: $0 pubkey <private key>" >&2; exit 2; }
    openssl pkey -in "$2" -pubout -outform DER | tail -c 32 | hex
    echo
    ;;
sign)
    [ $# -eq 7 ] || {
        echo "usage: $0 sign <private key> <version> <min version> <windows exe> <macos app zip> <output dir>" >&2
        exit 2
    }
    key="$2" version="$3" min_version="$4" windows="$5" macos="$6" out="$7"
    version_ok "$version" || { echo "bad version: $version" >&2; exit 1; }
    version_ok "$min_version" || { echo "bad min version: $min_version" >&2; exit 1; }
    # Asset paths are relative to https://github.com/<repo>/releases.
    folder="download/v$version"
    mkdir -p "$out"
    cp "$windows" "$out/p5099.exe"
    cp "$macos" "$out/p5099.app.zip"
    body="$(mktemp)"
    trap 'rm -f "$body"' EXIT
    cat > "$body" <<JSON
{
  "version": "$version",
  "min_version": "$min_version",
  "windows": $(asset "$folder/p5099.exe" "$out/p5099.exe"),
  "macos": $(asset "$folder/p5099.app.zip" "$out/p5099.app.zip")
}
JSON
    signature="$(sign_hex "$key" "$body")"
    { printf '%s\n' "$signature"; cat "$body"; } > "$out/latest.txt"
    echo "wrote $out/latest.txt"
    ;;
sign-manifest)
    [ $# -eq 3 ] || { echo "usage: $0 sign-manifest <private key> <manifest>" >&2; exit 2; }
    key="$2" manifest="$3"
    signature="$(dirname "$manifest")/p5099-manifest.sig"
    signed="$(sign_hex "$key" "$manifest")"
    printf '%s' "$signed" > "$signature"
    echo "wrote $signature"
    ;;
*)
    sed -n '2,26p' "$0" >&2
    exit 2
    ;;
esac
