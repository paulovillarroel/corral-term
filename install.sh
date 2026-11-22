#!/bin/sh
# Corral's installer.
#
#   curl -LsSf https://corral-term.dev/install.sh | sh
#
# From a copy of the repository it installs that copy, without downloading anything:
#   sh install.sh
# To only check the requirements, without installing anything:
#   sh install.sh --check          (or, with the curl line: ... | sh -s -- --check)
#
# Before changing anything it checks that everything needed is there and says
# what to do about whatever is missing. It never uses sudo: it only writes
# inside your home folder. This script only gets the `corral` command (already
# built, or it builds it with cargo); `corral install` does the checking and
# the installing. It speaks English; CORRAL_LANG=es (or `corral config
# language es` from an earlier install) makes it speak Spanish.
set -eu

REPO=paulovillarroel/corral-term
LINE="curl -LsSf https://corral-term.dev/install.sh | sh"

if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
    red=$(printf '\033[31m'); bold=$(printf '\033[1m'); off=$(printf '\033[0m')
else
    red=""; bold=""; off=""
fi

# The language: CORRAL_LANG, else `language` in Corral's config (on an update), else English.
lang=${CORRAL_LANG:-}
config="${XDG_CONFIG_HOME:-${HOME:-}/.config}/corral/config.json"
if [ -z "$lang" ] && [ -f "$config" ] && grep -q '"language"[[:space:]]*:[[:space:]]*"es"' "$config" 2>/dev/null; then
    lang=es
fi

# t ENGLISH SPANISH: the text in the user's language.
t() {
    case "$lang" in
        es | ES | Es | spanish | Spanish | español | Español | espanol | Espanol) printf '%s' "$2" ;;
        *) printf '%s' "$1" ;;
    esac
}

# stop TITLE LINE...: what is missing and what to do, then exit without changes.
stop() {
    printf '\n%s%s✗ %s%s\n\n' "$red" "$bold" "$1" "$off" >&2
    shift
    for line in "$@"; do printf '  %s\n' "$line" >&2; done
    printf '\n%s\n' "$(t 'Nothing was changed.' 'No cambié nada.')" >&2
    exit 1
}

case "$(uname -s)" in
    Linux) ;;
    Darwin) stop "$(t "Corral doesn't work on macOS yet." "Corral todavía no funciona en macOS.")" \
                 "$(t "herdr does work on a Mac, but several parts of Corral use features only Linux has." \
                      "herdr sí funciona en Mac, pero varias piezas de Corral usan funciones propias de Linux.")" \
                 "$(t "For now Corral works on Linux and on Windows with WSL." \
                      "Por ahora Corral funciona en Linux y en Windows con WSL.")" ;;
    *) stop "$(t "Corral works on Linux and on Windows with WSL, and this system is $(uname -s)." \
                 "Corral funciona en Linux y en Windows con WSL, y este sistema es $(uname -s).")" \
            "$(t "On Windows, open a WSL (Ubuntu) terminal and run the installation there." \
                 "En Windows, abre una terminal de WSL (Ubuntu) y corre ahí la instalación.")" ;;
esac
case "$(uname -m)" in
    x86_64 | amd64) ;;
    *) stop "$(t "For now Corral only installs on x86_64 machines (Intel or AMD), and this one is $(uname -m)." \
                 "Corral por ahora se instala solo en equipos x86_64 (Intel o AMD), y este es $(uname -m).")" ;;
esac

missing=""
for tool in curl tar; do
    command -v "$tool" >/dev/null 2>&1 || missing="$missing $tool"
done
if [ -n "$missing" ]; then
    stop "$(t "Some basic programs are missing:" "Faltan programas básicos:")$missing" \
         "$(t "On Ubuntu or Debian:" "En Ubuntu o Debian:")  ${bold}sudo apt install$missing$off" \
         "" "$(t "Then run the installation again:" "Después vuelve a correr la instalación:")" "  $LINE"
fi

# corral_of DIR AGAIN: make sure DIR has the `corral` command, compiling it if needed.
corral_of() {
    exe="$1/rs/target/release/corral"
    if [ -x "$exe" ] && "$exe" --version >/dev/null 2>&1; then
        return 0
    fi
    if ! command -v cargo >/dev/null 2>&1; then
        stop "$(t "This copy of Corral doesn't come with the command built, and Rust isn't here to build it." \
                  "Esta copia de Corral no trae el comando compilado, y no está Rust para compilarlo.")" \
             "$(t "Install Rust (a few minutes; it goes in ~/.cargo and ~/.rustup):" \
                  "Instala Rust (unos minutos; queda en ~/.cargo y ~/.rustup):")" \
             "  ${bold}curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh${off}" \
             "" "$(t "Then open a new terminal and run the installation again:" \
                     "Después abre una terminal nueva y vuelve a correr la instalación:")" "  $2"
    fi
    printf '%s\n' "$(t 'Building corral (a few seconds, nothing to download)...' 'Compilando corral (unos segundos, sin descargar nada)...')"
    if ! (cd "$1/rs" && cargo build --release --offline --quiet); then
        stop "$(t "Couldn't build corral." "No pude compilar corral.")" \
             "$(t "Update Rust (rustup update) and run the installation again:" \
                  "Actualiza Rust (rustup update) y vuelve a correr la instalación:")" "  $2"
    fi
}

# From a clone (sh install.sh): install that copy where it is.
here=""
case "$0" in
    *install.sh) here=$(cd "$(dirname "$0")" 2>/dev/null && pwd || true) ;;
esac
if [ -n "$here" ] && [ -f "$here/rs/Cargo.toml" ] && [ -f "$here/herdr/herdr-plugin.toml" ]; then
    corral_of "$here" "sh $here/install.sh"
    exec "$here/rs/target/release/corral" install --from "$here" --in-place --rerun "sh $here/install.sh" "$@"
fi

# Piped from curl: download Corral to a temporary folder; `corral install`
# checks everything else and copies it in place only if nothing is missing.
tmp=$(mktemp -d "${TMPDIR:-/tmp}/corral.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
trap 'exit 130' INT TERM
printf '%s\n' "$(t 'Downloading Corral...' 'Descargando Corral...')"
release="${CORRAL_URL:-https://github.com/$REPO/releases/latest/download/corral-term-x86_64-linux.tar.gz}"
if ! curl -fsSL "$release" -o "$tmp/corral.tar.gz" 2>/dev/null &&
   ! curl -fsSL "https://github.com/$REPO/archive/refs/heads/main.tar.gz" -o "$tmp/corral.tar.gz" 2>/dev/null; then
    stop "$(t "Couldn't download Corral from GitHub." "No pude descargar Corral de GitHub.")" \
         "$(t "Check the internet connection and try again." "Revisa la conexión a internet y vuelve a intentarlo.")" \
         "$(t "If the repository is private, the download needs access: clone it with git" \
              "Si el repositorio es privado, la descarga necesita acceso: clónalo con git")" \
         "$(t "and run ${bold}sh install.sh${off} inside the copy." "y corre ${bold}sh install.sh${off} dentro de la copia.")"
fi
tar -xzf "$tmp/corral.tar.gz" -C "$tmp"
found=$(find "$tmp" -maxdepth 3 -path '*/rs/Cargo.toml' | head -n 1)
[ -n "$found" ] || stop "$(t "What I downloaded isn't Corral." "Lo que descargué no es Corral.")" \
                        "$(t "Let whoever gave you the installation know." "Avísale a quien te pasó la instalación.")"
src=$(dirname "$(dirname "$found")")
corral_of "$src" "$LINE"
"$src/rs/target/release/corral" install --from "$src" --rerun "$LINE" "$@"
