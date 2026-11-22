#!/bin/sh
# Instalador de Corral.
#
#   curl -LsSf https://raw.githubusercontent.com/paulovillarroel/corral-term/main/install.sh | sh
#
# Desde una copia del repositorio instala esa copia, sin descargar nada:
#   sh install.sh
# Solo revisar los requisitos, sin instalar nada:
#   sh install.sh --revisar          (o, con la línea de curl: ... | sh -s -- --revisar)
#
# Antes de cambiar nada revisa que esté todo lo necesario y dice qué hacer con
# lo que falte. No usa sudo: solo escribe dentro de tu carpeta personal. Este
# script solo consigue el comando `corral` (ya compilado, o lo compila con
# cargo); la revisión y la instalación las hace `corral instalar`.
set -eu

REPO=paulovillarroel/corral-term
LINE="curl -LsSf https://raw.githubusercontent.com/$REPO/main/install.sh | sh"

if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
    red=$(printf '\033[31m'); bold=$(printf '\033[1m'); off=$(printf '\033[0m')
else
    red=""; bold=""; off=""
fi

# stop TITLE LINE...: what is missing and what to do, then exit without changes.
stop() {
    printf '\n%s%s✗ %s%s\n\n' "$red" "$bold" "$1" "$off" >&2
    shift
    for line in "$@"; do printf '  %s\n' "$line" >&2; done
    printf '\nNo cambié nada.\n' >&2
    exit 1
}

case "$(uname -s)" in
    Linux) ;;
    Darwin) stop "Corral todavía no funciona en macOS." \
                 "herdr sí funciona en Mac, pero varias piezas de Corral usan funciones propias de Linux." \
                 "Por ahora Corral funciona en Linux y en Windows con WSL." ;;
    *) stop "Corral funciona en Linux y en Windows con WSL, y este sistema es $(uname -s)." \
            "En Windows, abre una terminal de WSL (Ubuntu) y corre ahí la instalación." ;;
esac
case "$(uname -m)" in
    x86_64 | amd64) ;;
    *) stop "Corral por ahora se instala solo en equipos x86_64 (Intel o AMD), y este es $(uname -m)." ;;
esac

missing=""
for tool in curl tar; do
    command -v "$tool" >/dev/null 2>&1 || missing="$missing $tool"
done
if [ -n "$missing" ]; then
    stop "Faltan programas básicos:$missing" \
         "En Ubuntu o Debian:  ${bold}sudo apt install$missing$off" \
         "" "Después vuelve a correr la instalación:" "  $LINE"
fi

# corral_of DIR AGAIN: make sure DIR has the `corral` command, compiling it if needed.
corral_of() {
    exe="$1/rs/target/release/corral"
    if [ -x "$exe" ] && "$exe" --version >/dev/null 2>&1; then
        return 0
    fi
    if ! command -v cargo >/dev/null 2>&1; then
        stop "Esta copia de Corral no trae el comando compilado, y no está Rust para compilarlo." \
             "Instala Rust (unos minutos; queda en ~/.cargo y ~/.rustup):" \
             "  ${bold}curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh${off}" \
             "" "Después abre una terminal nueva y vuelve a correr la instalación:" "  $2"
    fi
    printf 'Compilando corral (unos segundos, sin descargar nada)...\n'
    if ! (cd "$1/rs" && cargo build --release --offline --quiet); then
        stop "No pude compilar corral." "Actualiza Rust (rustup update) y vuelve a correr la instalación:" "  $2"
    fi
}

# From a clone (sh install.sh): install that copy where it is.
here=""
case "$0" in
    *install.sh) here=$(cd "$(dirname "$0")" 2>/dev/null && pwd || true) ;;
esac
if [ -n "$here" ] && [ -f "$here/rs/Cargo.toml" ] && [ -f "$here/herdr/herdr-plugin.toml" ]; then
    corral_of "$here" "sh $here/install.sh"
    exec "$here/rs/target/release/corral" instalar --desde "$here" --en-el-lugar --repetir "sh $here/install.sh" "$@"
fi

# Piped from curl: download Corral to a temporary folder; `corral instalar`
# checks everything else and copies it in place only if nothing is missing.
tmp=$(mktemp -d "${TMPDIR:-/tmp}/corral.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
trap 'exit 130' INT TERM
printf 'Descargando Corral...\n'
release="${CORRAL_URL:-https://github.com/$REPO/releases/latest/download/corral-term-x86_64-linux.tar.gz}"
if ! curl -fsSL "$release" -o "$tmp/corral.tar.gz" 2>/dev/null &&
   ! curl -fsSL "https://github.com/$REPO/archive/refs/heads/main.tar.gz" -o "$tmp/corral.tar.gz" 2>/dev/null; then
    stop "No pude descargar Corral de GitHub." \
         "Revisa la conexión a internet y vuelve a intentarlo." \
         "Si el repositorio es privado, la descarga necesita acceso: clónalo con git" \
         "y corre ${bold}sh install.sh${off} dentro de la copia."
fi
tar -xzf "$tmp/corral.tar.gz" -C "$tmp"
found=$(find "$tmp" -maxdepth 3 -path '*/rs/Cargo.toml' | head -n 1)
[ -n "$found" ] || stop "Lo que descargué no es Corral." "Avísale a quien te pasó la instalación."
src=$(dirname "$(dirname "$found")")
corral_of "$src" "$LINE"
"$src/rs/target/release/corral" instalar --desde "$src" --repetir "$LINE" "$@"
