#!/bin/sh
# clio-memory installer: downloads the right prebuilt binary, checks it, puts it in place and
# hands over to `clio-memory setup --from-installer`. The binary or nothing: it is verified
# before it touches the destination, and put there with a single rename.
#
#   curl -fsSL https://raw.githubusercontent.com/i-montes/clio-memory/main/install.sh | sh
#   curl -fsSL .../install.sh | sh -s -- --agents codex     # arguments go to `setup` as they are
#
# Environment:
#   CLIO_MEMORY_VERSION=vX.Y.Z        pin a version (default: the latest release)
#   CLIO_MEMORY_BIN_DIR               where the binary goes (default: $HOME/.local/bin)
#   CLIO_MEMORY_DOWNLOAD_URL          releases base URL (default: the GitHub releases of the repo)
#   CLIO_MEMORY_SKIP_ATTESTATION=1    do not try `gh attestation verify`
#   CLIO_MEMORY_HAND_OVER=0           install only, do not run setup (for the release smoke test)
#   CLIO_MEMORY_TTY                   test hook: the file read as the terminal (default: /dev/tty)
set -eu

REPO="i-montes/clio-memory"
BASE="${CLIO_MEMORY_DOWNLOAD_URL:-https://github.com/$REPO/releases}"
BASE="${BASE%/}"
BIN_DIR="${CLIO_MEMORY_BIN_DIR:-${HOME:?HOME is not set}/.local/bin}"
TTY="${CLIO_MEMORY_TTY:-/dev/tty}"

if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
    AMBER=$(printf '\033[38;2;255;177;61m')
    ASH=$(printf '\033[38;2;138;143;152m')
    RED=$(printf '\033[38;2;180;85;75m')
    RESET=$(printf '\033[0m')
else
    AMBER='' ASH='' RED='' RESET=''
fi

TMP=''
STAGE=''
ASSET=''
URL=''

# Removes whatever this run left behind: its temporary folder and a half-staged binary.
cleanup() {
    [ -z "$TMP" ] || rm -rf "$TMP"
    [ -z "$STAGE" ] || rm -f "$STAGE"
}

die() {
    printf '%s✖%s  %s\n' "$RED" "$RESET" "$*" >&2
    exit 1
}

# Sets ASSET from the machine, or says there is no binary for it.
platform() {
    os=$(uname -s)
    arch=$(uname -m)
    case "$os $arch" in
        "Linux x86_64" | "Linux amd64") ASSET=clio-memory-x86_64-unknown-linux-gnu ;;
        "Linux aarch64" | "Linux arm64") ASSET=clio-memory-aarch64-unknown-linux-gnu ;;
        "Darwin arm64") ASSET=clio-memory-aarch64-apple-darwin ;;
        "Darwin x86_64")
            # An Intel shell under Rosetta on an Apple silicon Mac: the native binary is better.
            if [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || true)" = 1 ]; then
                ASSET=clio-memory-aarch64-apple-darwin
            else
                ASSET=clio-memory-x86_64-apple-darwin
            fi
            ;;
        *)
            printf '%s✖%s  No hay binario para esta máquina · %s %s\n' "$RED" "$RESET" "$os" "$arch" >&2
            printf '   Hay para: Linux x86_64, Linux aarch64, macOS arm64, macOS x86_64.\n' >&2
            printf '   No se instaló nada.\n' >&2
            exit 1
            ;;
    esac
    if [ -n "${CLIO_MEMORY_VERSION:-}" ]; then
        URL="$BASE/download/$CLIO_MEMORY_VERSION"
    else
        URL="$BASE/latest/download"
    fi
}

# Downloads the sums and the binary of the same release into the temporary folder. Over https,
# a redirect may not leave https (`file://`, for the tests and the release smoke test, has none).
fetch() {
    case "$URL" in
        https://*) set -- --proto-redir =https ;;
        *) set -- ;;
    esac
    for file in SHA256SUMS "$ASSET"; do
        curl -fsSL --retry 3 "$@" -o "$TMP/$file" "$URL/$file" ||
            die "No se pudo bajar $URL/$file: no se instaló nada."
    done
}

# Prints the sha256 of a file, with whichever tool the machine has.
sha_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        die "No hay sha256sum ni shasum para verificar la descarga: no se instaló nada."
    fi
}

# The downloaded binary must be the one `SHA256SUMS` lists for it.
check_sum() {
    want=$(awk -v a="$ASSET" '{ n = $2; sub(/^\*/, "", n); if (n == a) { print $1; exit } }' "$TMP/SHA256SUMS")
    [ -n "$want" ] || die "$ASSET no está en SHA256SUMS: no se instaló nada."
    have=$(sha_of "$TMP/$ASSET")
    [ "$have" = "$want" ] || die "La suma sha256 no coincide: no se instaló nada."
    printf '%s◇%s  clio-memory %s · sha256 ✓\n' "$AMBER" "$RESET" "$ASSET"
}

# With a signed-in `gh` that has `gh attestation` (2.49 or newer), a binary whose provenance
# does not verify stops the install, with what `gh` said under it. Without one, there is nothing
# to check against and it says so.
check_attestation() {
    [ -z "${CLIO_MEMORY_SKIP_ATTESTATION:-}" ] || return 0
    if ! command -v gh >/dev/null 2>&1 || ! gh auth status >/dev/null 2>&1 </dev/null; then
        printf '%s· sin verificación de atestación (gh no está o no tiene sesión)%s\n' "$ASH" "$RESET"
        return 0
    fi
    if ! gh attestation verify --help >/dev/null 2>&1 </dev/null; then
        printf '%s· sin verificación de atestación (este gh no tiene «gh attestation»: hace falta gh 2.49 o más nuevo)%s\n' "$ASH" "$RESET"
        return 0
    fi
    if ! why=$(gh attestation verify "$TMP/$ASSET" --repo "$REPO" 2>&1 >/dev/null </dev/null); then
        printf '%s✖%s  No se pudo verificar la atestación de GitHub de este binario: no se instaló nada.\n' "$RED" "$RESET" >&2
        printf '%s\n' "$why" | sed 's/^/   /' >&2
        printf '   Si fue la red, vuelve a correr la línea; CLIO_MEMORY_SKIP_ATTESTATION=1 instala sin esta verificación.\n' >&2
        exit 1
    fi
    printf '%s◇%s  atestación de GitHub ✓\n' "$AMBER" "$RESET"
}

# Puts the verified binary at its destination: same sum, nothing to do; otherwise it is staged
# next to the destination (same filesystem) and renamed over it, so the destination is always
# either the old binary or the new one.
put() {
    BIN="$BIN_DIR/clio-memory"
    if [ -f "$BIN" ] && [ "$(sha_of "$BIN")" = "$have" ]; then
        printf '%s◇%s  clio-memory ya estaba instalado · %s\n' "$AMBER" "$RESET" "$BIN"
        return 0
    fi
    mkdir -p "$BIN_DIR"
    STAGE="$BIN_DIR/.clio-memory.$$"
    cp "$TMP/$ASSET" "$STAGE"
    chmod 755 "$STAGE"
    mv -f "$STAGE" "$BIN"
    STAGE=''
    printf '%s◇%s  instalado en %s\n' "$AMBER" "$RESET" "$BIN"
}

# Passes control to `setup`, reading the keyboard from the terminal (stdin is the script under
# `curl | sh`). With no terminal at all it runs without questions.
hand_over() {
    if [ "${CLIO_MEMORY_HAND_OVER:-1}" = 0 ]; then
        return 0
    fi
    if (exec <"$TTY") 2>/dev/null; then
        exec "$BIN" setup --from-installer "$@" <"$TTY"
    else
        exec "$BIN" setup --from-installer --yes "$@"
    fi
}

main() {
    printf '%s┌  Clio memory · instalador%s\n' "$AMBER" "$RESET"
    platform
    TMP=$(mktemp -d)
    trap cleanup EXIT
    trap 'exit 1' HUP INT TERM
    fetch
    check_sum
    check_attestation
    put
    # `exec` skips the EXIT trap, so the temporary folder goes before it.
    cleanup
    TMP=''
    hand_over "$@"
}

main "$@"
