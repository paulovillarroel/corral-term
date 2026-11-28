#!/bin/sh
# Corral's installer.
#
#   curl -LsSf https://corral-term.dev/install.sh | sh
#
# It installs the latest published version (CORRAL_VERSION=v0.1.3 picks another),
# checks the package's SHA-256 before opening it and, when the GitHub CLI (gh
# 2.49 or later, logged in) is there, its build provenance attestation; and it
# says which version it is. It never falls back to an unreleased copy: if
# something fails, it stops and says why. From a copy of the repository it
# installs that copy, without downloading anything:
#   sh install.sh
# To only check the requirements, without installing anything:
#   sh install.sh --check          (or, with the curl line: ... | sh -s -- --check)
#
# Before changing anything it checks that everything needed is there and says
# what to do about whatever is missing. It never uses sudo: it only writes
# inside your home folder. This script only gets the `corral` command (already
# built, or it builds it with cargo from a clone); `corral install` does the
# checking and the installing. It speaks English; CORRAL_LANG=es (or `corral
# config language es` from an earlier install) makes it speak Spanish.
#
# Everything runs from main() at the very end, so a download cut halfway runs
# nothing.
set -eu

REPO=paulovillarroel/corral-term
LINE="curl -LsSf https://corral-term.dev/install.sh | sh"
# The package for this system and processor (main picks it): corral-term-x86_64-linux,
# corral-term-aarch64-macos or corral-term-x86_64-macos, .tar.gz.
PACKAGE=""

if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
    red=$(printf '\033[31m'); green=$(printf '\033[32m'); bold=$(printf '\033[1m'); off=$(printf '\033[0m')
else
    red=""; green=""; bold=""; off=""
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
    if [ "$counted" = 1 ]; then
        printf '\n%s\n' "$(t 'Counted this stop anonymously (the system, the version and why it stopped). CORRAL_NO_TELEMETRY=1 turns it off.' \
                              'Conté esta detención de forma anónima (el sistema, la versión y por qué se detuvo). CORRAL_NO_TELEMETRY=1 lo desactiva.')" >&2
    fi
    exit 1
}

# count_stop SYSTEM MISSING [VERSION]: one anonymous ping when the installer stops before
# Corral is on this machine (another system or processor, no tar, a version without a
# macOS package), so the
# project learns who it leaves out: the system and what was missing, nothing else
# (api/installed.js). Also when a download doesn't check out (sha256, sha256_missing,
# attestation), with VERSION: that one is an alarm about a published package.
# CORRAL_NO_TELEMETRY=1, DO_NOT_TRACK=1 or `corral config telemetry off` turn it
# off, and --check never sends it. 5 seconds at most.
counted=0
check_only=0
count_stop() {
    case "${CORRAL_NO_TELEMETRY:-0}${DO_NOT_TRACK:-0}" in 00 | 0 | "") ;; *) return 0 ;; esac
    if [ "$check_only" = 1 ] || ! command -v curl >/dev/null 2>&1; then return 0; fi
    # The settings on one line: JSON may put a key and its value on separate ones.
    flat=""
    if [ -f "$config" ]; then flat=$(tr '\n\r' '  ' < "$config" 2>/dev/null || true); fi
    if printf '%s' "$flat" | grep -q '"telemetry"[[:space:]]*:[[:space:]]*"off"'; then return 0; fi
    # Nor with the sensitive profile: results without values, and agents asking (the default).
    if printf '%s' "$flat" | grep -q '"agent_output"[[:space:]]*:[[:space:]]*"structure"' &&
        ! printf '%s' "$flat" | grep -Eq '"agents"[[:space:]]*:[[:space:]]*"(on|off)"'; then return 0; fi
    v=${3:-unknown}
    case "$v" in "" | *[!0-9.]*) v=unknown ;; esac
    # net: install-count
    if curl -fsS -m 5 -X POST -H 'Content-Type: application/json' \
        --data "{\"version\":\"$v\",\"system\":\"$1\",\"kind\":\"stopped\",\"source\":\"release\",\"missing\":[\"$2\"]}" \
        "${CORRAL_TELEMETRY_URL:-https://corral-term.dev/api/installed}" >/dev/null 2>&1; then
        counted=1
    fi
}

# linux, wsl or macos: the system field of the ping.
system_name() {
    if [ "$(uname -s)" = Darwin ]; then printf macos
    elif grep -qi microsoft /proc/version 2>/dev/null; then printf wsl
    else printf linux; fi
}

# corral_of DIR AGAIN: make sure DIR has the `corral` command, compiling it if needed:
# when there is none, or it is another version than the code (after a git pull).
corral_of() {
    exe="$1/rs/target/release/corral"
    want=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$1/rs/Cargo.toml" | head -n 1)
    if [ -x "$exe" ] && "$exe" --version 2>/dev/null | grep -q "^corral $want "; then
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

# sha256 FILE: the file's SHA-256, with whichever tool the system has.
sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d' ' -f1
    else
        stop "$(t "There's no tool here to check the download's SHA-256 (sha256sum or shasum)." \
                  "No hay una herramienta para comprobar el SHA-256 de la descarga (sha256sum o shasum).")" \
             "$(t "On Ubuntu or Debian:" "En Ubuntu o Debian:")  ${bold}sudo apt install coreutils${off}"
    fi
}

# attestation FILE: check that the package was built by this project's GitHub
# workflow, with the GitHub CLI. The SHA-256 comes from the same release as the
# package, so it catches a broken download; the attestation is signed by GitHub
# when the workflow builds the package, so it also catches a package replaced in
# the release. Without gh (2.49 or later, logged in) it says so and goes on.
attestation() {
    if [ -n "${CORRAL_NO_ATTESTATION:-}" ]; then
        printf '%s\n' "$(t "! Attestation not checked (CORRAL_NO_ATTESTATION)." "! Atestación sin comprobar (CORRAL_NO_ATTESTATION).")"
        return 0
    fi
    # net: local: only whether gh can check
    if ! command -v gh >/dev/null 2>&1 || ! gh attestation verify --help >/dev/null 2>&1; then
        printf '  %s\n' "$(t "Attestation not checked: that needs the GitHub CLI (gh 2.49 or later)." \
                             "Atestación sin comprobar: hace falta la CLI de GitHub (gh 2.49 o posterior).")"
        return 0
    fi
    # net: attestation: gh checks its login with GitHub
    if ! gh auth status >/dev/null 2>&1; then
        printf '  %s\n' "$(t "Attestation not checked: gh isn't logged in (gh auth login)." \
                             "Atestación sin comprobar: gh no tiene una sesión iniciada (gh auth login).")"
        return 0
    fi
    # net: attestation
    if why=$(gh attestation verify "$1" --repo "$REPO" </dev/null 2>&1); then
        printf '%s✓%s %s\n' "$green" "$off" "$(t "Attestation checked: built by $REPO's GitHub workflow" \
                                                  "Atestación comprobada: la compiló el flujo de GitHub de $REPO")"
        return 0
    fi
    printf '%s\n' "$why" | tail -n 5 >&2
    count_stop "$(system_name)" attestation "${tag#v}"
    stop "$(t "The package's build provenance attestation didn't check out, so I didn't open it." \
              "La atestación de origen del paquete no se pudo comprobar, así que no lo abrí.")" \
         "$(t "Try again; if it keeps happening, tell the project: https://github.com/$REPO/issues" \
              "Vuelve a intentarlo; si se repite, avísale al proyecto: https://github.com/$REPO/issues")" \
         "$(t "If gh itself doesn't work here, CORRAL_NO_ATTESTATION=1 installs with only the SHA-256 check." \
              "Si es gh el que no funciona aquí, CORRAL_NO_ATTESTATION=1 instala solo con la comprobación del SHA-256.")"
}

# latest_tag: the tag of the latest published version (v0.1.3), from GitHub's redirect.
latest_tag() {
    # net: corral-package: GitHub's "latest" link
    url=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest" 2>/dev/null) || return 0
    case "$url" in
        */releases/tag/v*) printf '%s' "${url##*/tag/}" ;;
    esac
}

main() {
    for arg in "$@"; do case "$arg" in --check) check_only=1 ;; esac; done
    case "$(uname -s)/$(uname -m)" in
        Linux/x86_64 | Linux/amd64) PACKAGE=corral-term-x86_64-linux.tar.gz ;;
        Darwin/arm64 | Darwin/aarch64) PACKAGE=corral-term-aarch64-macos.tar.gz ;;
        Darwin/x86_64) PACKAGE=corral-term-x86_64-macos.tar.gz ;;
        Linux/* | Darwin/*)
           count_stop "$(system_name)" arch
           stop "$(t "Corral installs on Linux x86_64 (Intel or AMD) and on Macs, and this machine is $(uname -s) $(uname -m)." \
                     "Corral se instala en Linux x86_64 (Intel o AMD) y en Mac, y este equipo es $(uname -s) $(uname -m).")" ;;
        *) count_stop other system
           stop "$(t "Corral works on Linux, macOS and Windows with WSL, and this system is $(uname -s)." \
                     "Corral funciona en Linux, macOS y Windows con WSL, y este sistema es $(uname -s).")" \
                "$(t "On Windows, open a WSL (Ubuntu) terminal and run the installation there." \
                     "En Windows, abre una terminal de WSL (Ubuntu) y corre ahí la instalación.")" ;;
    esac

    missing=""
    for tool in curl tar; do
        command -v "$tool" >/dev/null 2>&1 || missing="$missing $tool"
    done
    if [ -n "$missing" ]; then
        count_stop "$(system_name)" basics
        stop "$(t "Some basic programs are missing:" "Faltan programas básicos:")$missing" \
             "$(t "On Ubuntu or Debian:" "En Ubuntu o Debian:")  ${bold}sudo apt install$missing$off" \
             "" "$(t "Then run the installation again:" "Después vuelve a correr la instalación:")" "  $LINE"
    fi

    # From a clone (sh install.sh): install that copy where it is.
    here=""
    case "$0" in
        *install.sh) here=$(cd "$(dirname "$0")" 2>/dev/null && pwd || true) ;;
    esac
    if [ -n "$here" ] && [ -f "$here/rs/Cargo.toml" ] && [ -f "$here/herdr/herdr-plugin.toml" ]; then
        corral_of "$here" "sh $here/install.sh"
        exec "$here/rs/target/release/corral" install --from "$here" --in-place --rerun "sh $here/install.sh" "$@"
    fi

    # Piped from curl: a published version, checked, in a temporary folder; `corral
    # install` checks everything else and copies it in place only if nothing is missing.
    tmp=$(mktemp -d "${TMPDIR:-/tmp}/corral.XXXXXX")
    trap 'rm -rf "$tmp"' EXIT
    trap 'exit 130' INT TERM

    attest=""
    if [ -n "${CORRAL_URL:-}" ]; then
        # A package from elsewhere (tests, a mirror): CORRAL_SHA256 says what it must be;
        # skipping the check takes saying so (CORRAL_SHA256=skip).
        expected=${CORRAL_SHA256:-}
        [ -n "$expected" ] ||
            stop "$(t "CORRAL_URL needs CORRAL_SHA256: the SHA-256 the package must have." \
                      "CORRAL_URL necesita CORRAL_SHA256: el SHA-256 que debe tener el paquete.")" \
                 "$(t "To install it without checking, say so: CORRAL_SHA256=skip" \
                      "Para instalarlo sin comprobarlo, dilo: CORRAL_SHA256=skip")"
        [ "$expected" != skip ] || expected=""
        printf '%s\n' "$(t "Downloading Corral from $CORRAL_URL..." "Descargando Corral desde $CORRAL_URL...")"
        # net: corral-package: a package from elsewhere (CORRAL_URL)
        curl -fsSL "$CORRAL_URL" -o "$tmp/$PACKAGE" ||
            stop "$(t "Couldn't download $CORRAL_URL." "No pude descargar $CORRAL_URL.")"
    else
        attest=yes
        tag=${CORRAL_VERSION:-$(latest_tag)}
        [ -n "$tag" ] || stop "$(t "Couldn't find out which version of Corral is the latest." "No pude averiguar cuál es la última versión de Corral.")" \
                             "$(t "Check the internet connection and try again." "Revisa la conexión a internet y vuelve a intentarlo.")"
        case "$tag" in v*) ;; *) tag="v$tag" ;; esac
        base="https://github.com/$REPO/releases/download/$tag"
        printf '%s\n' "$(t "Downloading Corral ${tag#v}..." "Descargando Corral ${tag#v}...")"
        # net: corral-package
        code=$(curl -fsSL -w '%{http_code}' "$base/$PACKAGE" -o "$tmp/$PACKAGE" 2>/dev/null) || {
            # Versions published before the macOS version have no package for a Mac.
            if [ "$code" = 404 ] && [ "$(uname -s)" = Darwin ]; then
                count_stop macos system "${tag#v}"
                stop "$(t "Corral ${tag#v} has no package for macOS." "Corral ${tag#v} no tiene un paquete para macOS.")" \
                     "$(t "If it's the latest version, the macOS version isn't published yet; when it is, this same line installs it." \
                          "Si es la última versión, la de macOS todavía no está publicada; cuando lo esté, esta misma línea la instala.")" \
                     "$(t "Published versions: https://github.com/$REPO/releases" "Versiones publicadas: https://github.com/$REPO/releases")"
            fi
            stop "$(t "Couldn't download Corral $tag from GitHub." "No pude descargar Corral $tag de GitHub.")" \
                 "$(t "Check the internet connection and try again." "Revisa la conexión a internet y vuelve a intentarlo.")" \
                 "$(t "Published versions: https://github.com/$REPO/releases" "Versiones publicadas: https://github.com/$REPO/releases")"
        }
        # net: corral-package
        curl -fsSL "$base/$PACKAGE.sha256" -o "$tmp/$PACKAGE.sha256" 2>/dev/null || {
            count_stop "$(system_name)" sha256_missing "${tag#v}"
            stop "$(t "Corral $tag has no published SHA-256 to check the download against." "Corral $tag no tiene un SHA-256 publicado para comprobar la descarga.")" \
                 "$(t "Versions from 0.1.3 on have one: install the latest, or pick one with CORRAL_VERSION." \
                      "Las versiones desde la 0.1.3 lo tienen: instala la última, o elige una con CORRAL_VERSION.")"
        }
        expected=$(cut -d' ' -f1 "$tmp/$PACKAGE.sha256")
    fi

    if [ -n "$expected" ]; then
        actual=$(sha256 "$tmp/$PACKAGE")
        [ "$actual" = "$expected" ] || {
            [ -z "$attest" ] || count_stop "$(system_name)" sha256 "${tag#v}"
            stop "$(t "The download doesn't match the SHA-256 it should have, so I didn't open it." "La descarga no coincide con el SHA-256 que debe tener, así que no la abrí.")" \
                 "$(t "expected" "esperado")  $expected" "$(t "got     " "obtenido")  $actual" \
                 "" "$(t "Try again; if it keeps happening, tell the project: https://github.com/$REPO/issues" \
                         "Vuelve a intentarlo; si se repite, avísale al proyecto: https://github.com/$REPO/issues")"
        }
        printf '%s✓%s %s\n' "$green" "$off" "$(t "SHA-256 checked" "SHA-256 comprobado")"
    else
        printf '%s\n' "$(t "! Not checked (CORRAL_SHA256=skip)." "! Sin comprobar (CORRAL_SHA256=skip).")"
    fi
    [ -z "$attest" ] || attestation "$tmp/$PACKAGE"

    tar -xzf "$tmp/$PACKAGE" -C "$tmp"
    found=$(find "$tmp" -maxdepth 3 -path '*/rs/Cargo.toml' | head -n 1)
    [ -n "$found" ] || stop "$(t "What I downloaded isn't Corral." "Lo que descargué no es Corral.")" \
                            "$(t "Let whoever gave you the installation know." "Avísale a quien te pasó la instalación.")"
    src=$(dirname "$(dirname "$found")")
    [ -x "$src/rs/target/release/corral" ] ||
        stop "$(t "This package doesn't come with corral built." "Este paquete no trae corral compilado.")" \
             "$(t "Published versions always do; tell the project: https://github.com/$REPO/issues" \
                  "Las versiones publicadas siempre lo traen; avísale al proyecto: https://github.com/$REPO/issues")"
    "$src/rs/target/release/corral" install --from "$src" --rerun "$LINE" "$@"
}

main "$@"
