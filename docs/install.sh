#!/bin/sh
# install.sh — the public installer for smartopen.
#
#     curl -fsSL https://lariocpt.github.io/smartopen/install.sh | sh
#
# It does four things and nothing else: resolve a GitHub Release, download the archive
# for this machine and the release's SHA256SUMS, verify it, and put `smartopen` and its
# alias `opn` on your PATH. There is no bootstrap left behind: the binaries are the
# whole install.
#
# POSIX sh on purpose: this is run by whatever /bin/sh the reader has — dash, ash,
# busybox, bash in posix mode. No arrays, no [[ ]], no `local`, no pipefail.
#
# EVERYTHING LIVES IN FUNCTIONS AND THE LAST LINE IS `main "$@"`. That is not style.
# The script is executed straight off a socket; if the connection dies halfway, a
# top-to-bottom script would run the half that arrived. sh cannot call main until it
# has read the closing brace, so a truncated download does nothing at all.
#
# Options (env var in brackets):
#   vX.Y.Z, --version=vX.Y.Z   install a specific release          [SMARTOPEN_VERSION]
#   --prefix=DIR               install root, default ~/.local      [SMARTOPEN_PREFIX]
#   --target=TRIPLE            override the detected Rust target   [SMARTOPEN_TARGET]
#   --source=github|apps       where to resolve from               [SMARTOPEN_SOURCE]
#   --download-only=DIR        fetch and verify into DIR, install nothing
#   --help
#
# Also honoured: SMARTOPEN_REPO (owner/repo), SMARTOPEN_DOWNLOAD_BASE (a URL or file://
# directory holding the archives and SHA256SUMS — how the container tests rehearse a
# release before it exists), APPS_URL (the LAN mirror).
set -eu

SO_REPO_DEFAULT='lariocpt/smartopen'
SO_SUMS='SHA256SUMS'

so_say()  { printf 'smartopen: %s\n' "$*"; }
so_warn() { printf 'smartopen: %s\n' "$*" >&2; }
so_die()  { so_warn "$*"; exit 1; }
so_have() { command -v "$1" >/dev/null 2>&1; }

# A heredoc, not `sed … "$0"`: piped through `sh`, $0 is the shell, not this file, and
# `--help` printed nothing.
so_usage() {
    cat <<'EOF'
install.sh — the public installer for smartopen.

    curl -fsSL https://lariocpt.github.io/smartopen/install.sh | sh

Options (env var in brackets):
  vX.Y.Z, --version=vX.Y.Z   install a specific release          [SMARTOPEN_VERSION]
  --prefix=DIR               install root, default ~/.local      [SMARTOPEN_PREFIX]
  --target=TRIPLE            override the detected Rust target   [SMARTOPEN_TARGET]
  --source=github|apps       where to resolve from               [SMARTOPEN_SOURCE]
  --download-only=DIR        fetch and verify into DIR, install nothing
  --help

Also honoured: SMARTOPEN_REPO (owner/repo), SMARTOPEN_DOWNLOAD_BASE (a URL or file://
directory holding the archives and SHA256SUMS), APPS_URL (the LAN mirror).
EOF
}

# The scheme allowlist is narrowed to https for the published path and widened only
# when the caller has already opted out of it by setting a non-https base themselves —
# a LAN mirror, a local file:// rehearsal. What makes the transport safe to relax is
# the sha256 check below, which is never skipped for any source.
so_fetch() {                                  # <url> <dest>
    case "$1" in
        file://*)
            # No network tool needed for a local rehearsal, which is what lets the
            # distro containers run this script on images that ship neither curl nor wget.
            cp -- "${1#file://}" "$2"
            return
            ;;
        https://*) proto='=https' ;;
        *)         proto='=https,http,file' ;;
    esac
    if so_have curl; then
        curl -fsSL --proto "$proto" --retry 3 --retry-delay 1 -o "$2" "$1"
    elif so_have wget; then
        # busybox wget has no --https-only and rejects the whole command line when it
        # sees one, so ask before using it. Alpine ships busybox wget and no curl, which
        # makes this the only route in on the very platform the musl build is for: the
        # flag went out unguarded and every stock-Alpine install died on it.
        if [ "$(so_wget_https_only)" = yes ]; then
            case "$1" in
                https://*) wget -q --https-only -O "$2" "$1" ;;
                *)         wget -q -O "$2" "$1" ;;
            esac
        else
            wget -q -O "$2" "$1"
        fi
    else
        so_die 'need curl or wget'
    fi
}

# Does this wget understand --https-only? Asked once, then remembered. Without it a
# redirect to http would be followed; the sha256 check below is what makes that safe,
# and it is never skipped.
so_wget_https_only() {
    if [ -z "${SO_WGET_HTTPS_ONLY:-}" ]; then
        if wget --help 2>&1 | grep -q -- '--https-only'; then
            SO_WGET_HTTPS_ONLY=yes
        else
            SO_WGET_HTTPS_ONLY=no
        fi
    fi
    printf '%s' "$SO_WGET_HTTPS_ONLY"
}

# Fail closed. A checksum you cannot compute is not a checksum, and the job of this
# script is to put an executable on somebody's PATH. There is deliberately no branch
# that skips the check.
so_sha256() {                                 # <file> -> hex on stdout
    if   so_have sha256sum; then sha256sum "$1" | cut -d' ' -f1
    elif so_have shasum;    then shasum -a 256 "$1" | cut -d' ' -f1
    elif so_have openssl;   then openssl dgst -sha256 "$1" | sed 's/.*= *//'
    else return 1
    fi
}

# The Rust target triple for this machine. Linux gets the static musl build: it has no
# libc floor at all, so one archive runs on Alpine and on a ten-year-old enterprise
# distro alike.
so_detect_target() {
    os=$(uname -s 2>/dev/null || echo unknown)
    arch=$(uname -m 2>/dev/null || echo unknown)
    case "$arch" in
        x86_64|amd64)  arch=x86_64 ;;
        aarch64|arm64) arch=aarch64 ;;
        *) so_die "no prebuilt binary for the $arch architecture; try: cargo install smartopen" ;;
    esac
    case "$os" in
        Linux)  printf '%s-unknown-linux-musl' "$arch" ;;
        Darwin) printf '%s-apple-darwin' "$arch" ;;
        MINGW*|MSYS*|CYGWIN*|Windows_NT)
            so_die 'on Windows use: cargo install smartopen, or the .zip from the Releases page' ;;
        *) so_die "no prebuilt binary for $os; try: cargo install smartopen" ;;
    esac
}

# Resolve the LAN mirror through its index rather than guessing a path: the rows whose
# path goes via latest/ are the only ones a client is meant to read, and they carry the
# sha256 inline. The plane serves the two binaries as plain files, not an archive.
so_resolve_apps() {                           # -> SO_APPS_ROWS (file<TAB>sha<TAB>relpath lines)
    apps="${APPS_URL:-https://apps.in.drlario.org}"
    idx="$SO_TMP/index.tsv"
    so_fetch "$apps/index.tsv" "$idx" || so_die "cannot reach the LAN mirror at $apps"
    SO_APPS_ROWS=$(awk -F'\t' -v v="$SO_VERSION" '
        $1=="tool" && $2=="smartopen" && index($7,"/latest/")>0 &&
        (v=="latest" || $3==v || $3==substr(v,2)) { print $4 "\t" $5 "\t" $7 }' "$idx")
    [ -n "$SO_APPS_ROWS" ] || so_die "no smartopen rows for '$SO_VERSION' in $apps/index.tsv"
    SO_APPS_URL="$apps"
}

# The /releases/latest/download/ redirect resolves without a token, so there is no
# credential to arrange and no 60-per-hour API budget for a shared NAT to exhaust.
so_resolve_github() {                         # -> SO_URL, SO_WANT, SO_ASSET
    repo="${SMARTOPEN_REPO:-$SO_REPO_DEFAULT}"
    if [ -n "${SMARTOPEN_DOWNLOAD_BASE:-}" ]; then
        base="${SMARTOPEN_DOWNLOAD_BASE%/}"
    elif [ "$SO_VERSION" = latest ]; then
        base="https://github.com/$repo/releases/latest/download"
    else
        base="https://github.com/$repo/releases/download/$SO_VERSION"
    fi
    so_fetch "$base/$SO_SUMS" "$SO_TMP/$SO_SUMS" || {
        # Two very different failures share this exit: no such release, or the fetch
        # tool itself refused. Say which, or a working release looks unpublished.
        if so_have curl || so_have wget; then
            so_die "cannot fetch $base/$SO_SUMS — is '$SO_VERSION' a published release of $repo, and is this machine online?"
        fi
        so_die "cannot fetch $base/$SO_SUMS — no curl or wget on this machine"
    }
    # Pick the archive for this target by name out of SHA256SUMS: the file names the
    # asset with no directory component. The leading '*' is sha256sum's binary-mode mark.
    SO_ASSET=$(awk -v t="smartopen-$SO_TARGET-" '
        { f=$2; sub(/^\*/, "", f) }
        index(f, t)==1 && f ~ /\.tar\.gz$/ { print f; exit }' "$SO_TMP/$SO_SUMS")
    [ -n "$SO_ASSET" ] || so_die "$SO_SUMS lists no archive for $SO_TARGET (targets: $(awk '{print $2}' "$SO_TMP/$SO_SUMS" | tr '\n' ' '))"
    SO_WANT=$(awk -v a="$SO_ASSET" '$2 == a || $2 == "*" a { print $1; exit }' "$SO_TMP/$SO_SUMS")
    SO_URL="$base/$SO_ASSET"
}

so_verify() {                                 # <file> <want>
    got=$(so_sha256 "$1") \
        || so_die 'no sha256sum, shasum or openssl here — refusing to install an unverified binary'
    [ "$2" = "$got" ] || so_die "CHECKSUM MISMATCH — do not run the downloaded file
  expected $2
  got      $got"
}

# Verified first, executable second, in place third: a truncated or tampered download
# must never be executable at a stable path, not even briefly.
so_install_file() {                           # <src> <dest>
    mkdir -p "$(dirname "$2")" || so_die "cannot create $(dirname "$2")"
    if ! { cp "$1" "$2.tmp.$$" && chmod 0755 "$2.tmp.$$" && mv "$2.tmp.$$" "$2"; }; then
        so_die "cannot write $2"
    fi
}

main() {
    SO_VERSION=${SMARTOPEN_VERSION:-latest}
    so_prefix=${SMARTOPEN_PREFIX:-${PREFIX:-}}
    so_source=${SMARTOPEN_SOURCE:-github}
    so_target=${SMARTOPEN_TARGET:-}
    so_dlonly=

    n=$#
    while [ "$n" -gt 0 ]; do
        arg=$1; shift; n=$((n - 1))
        case "$arg" in
            v[0-9]*)           SO_VERSION=$arg ;;
            --version=*)       SO_VERSION=${arg#--version=} ;;
            --prefix=*)        so_prefix=${arg#--prefix=} ;;
            --target=*)        so_target=${arg#--target=} ;;
            --source=*)        so_source=${arg#--source=} ;;
            --download-only=*) so_dlonly=${arg#--download-only=} ;;
            -h|--help)         so_usage; exit 0 ;;
            *)                 so_die "unknown option '$arg' (try --help)" ;;
        esac
    done

    [ -n "$so_prefix" ] || so_prefix="$HOME/.local"
    case "$so_prefix" in /*) ;; *) so_prefix="$PWD/$so_prefix" ;; esac
    while :; do case "$so_prefix" in */) so_prefix=${so_prefix%/} ;; *) break ;; esac; done

    SO_TMP=$(mktemp -d "${TMPDIR:-/tmp}/smartopen.XXXXXX") || so_die 'mktemp failed'
    trap 'rm -rf "$SO_TMP"' EXIT HUP INT TERM

    case "$so_source" in
        github)
            SO_TARGET=${so_target:-$(so_detect_target)}
            so_resolve_github
            so_say "downloading $SO_URL"
            so_fetch "$SO_URL" "$SO_TMP/$SO_ASSET" || so_die "download failed: $SO_URL"
            so_verify "$SO_TMP/$SO_ASSET" "$SO_WANT"
            if [ -n "$so_dlonly" ]; then
                if ! { mkdir -p "$so_dlonly" && cp "$SO_TMP/$SO_ASSET" "$SO_TMP/$SO_SUMS" "$so_dlonly/"; }; then
                    so_die "cannot write to $so_dlonly"
                fi
                so_say "verified archive written to $so_dlonly/$SO_ASSET"
                exit 0
            fi
            mkdir -p "$SO_TMP/x"
            tar -xzf "$SO_TMP/$SO_ASSET" -C "$SO_TMP/x" || so_die "cannot extract $SO_ASSET"
            for bin in smartopen opn; do
                src=$(find "$SO_TMP/x" -name "$bin" -type f | head -1)
                [ -n "$src" ] || so_die "$SO_ASSET does not contain $bin"
                so_install_file "$src" "$so_prefix/bin/$bin"
            done
            ;;
        apps)
            so_resolve_apps
            [ -z "$so_dlonly" ] || so_die '--download-only is for the github source'
            printf '%s\n' "$SO_APPS_ROWS" | while IFS="$(printf '\t')" read -r file sha relpath; do
                so_say "downloading $SO_APPS_URL/$relpath"
                so_fetch "$SO_APPS_URL/$relpath" "$SO_TMP/$file" || so_die "download failed: $relpath"
                so_verify "$SO_TMP/$file" "$sha"
                so_install_file "$SO_TMP/$file" "$so_prefix/bin/$file"
            done
            ;;
        *) so_die "unknown --source '$so_source' (want: github, apps)" ;;
    esac

    so_say "installed $so_prefix/bin/smartopen and $so_prefix/bin/opn ($("$so_prefix/bin/smartopen" --version 2>/dev/null || echo 'version unknown'))"

    case ":${PATH:-}:" in
        *":$so_prefix/bin:"*) ;;
        *) so_warn "$so_prefix/bin is not on your PATH. Add:
      export PATH=\"$so_prefix/bin:\$PATH\"" ;;
    esac
    so_say "next: \`smartopen wizard\` sets up yazi/broot and your file associations."
}

main "$@"
