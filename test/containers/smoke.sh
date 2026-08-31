#!/bin/sh
# test/containers/smoke.sh — what every distro container runs: install through the public
# installer from a local file:// base, then the non-interactive CLI smoke. POSIX sh only:
# this runs on busybox ash (Alpine) as well as dash and bash.
set -eu

fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }

# shellcheck disable=SC1091  # exists only inside the container
distro=$(. /etc/os-release 2>/dev/null && printf '%s %s' "${ID:-?}" "${VERSION_ID:-}")
printf '== %s: %s\n' "$distro" "$(uname -m)"

# 1. The public installer, resolving from the mounted release directory.
SMARTOPEN_DOWNLOAD_BASE=file:///dist sh /install.sh --prefix=/usr/local \
    || fail "install.sh failed"
command -v smartopen >/dev/null || fail "smartopen not on PATH after install"
command -v opn >/dev/null || fail "opn not on PATH after install"

# 2. Static: the musl archive must need no shared libraries at all. Rust's musl target is
#    static-PIE, and musl's own ldd prints the loader line for any PIE executable, so the
#    test is "no `=>` line" — a resolved shared library — not "ldd says nothing".
if command -v ldd >/dev/null 2>&1; then
    if ldd /usr/local/bin/smartopen 2>&1 | grep -q '=>'; then
        fail "smartopen is dynamically linked: $(ldd /usr/local/bin/smartopen 2>&1 | head -3)"
    fi
fi

# 3. Both binaries run and agree.
v1=$(smartopen --version) || fail "smartopen --version"
v2=$(opn --version) || fail "opn --version"
[ "$v1" = "$v2" ] || fail "versions differ: $v1 / $v2"
printf '%s\n' "$v1"

# 4. Config round trip and the doctor, in a sandboxed home.
export HOME=/tmp/home XDG_CONFIG_HOME=/tmp/home/.config XDG_STATE_HOME=/tmp/home/.state
mkdir -p "$HOME"
smartopen config sample > /tmp/c.toml || fail "config sample"
smartopen --config-path /tmp/c.toml config list | grep -q 'Shortcuts:' || fail "config list"
smartopen --config-path /tmp/c.toml config doctor >/dev/null || fail "config doctor must exit 0"

# 5. Quoting of a spaced path, the dry-run way.
mkdir -p '/tmp/space dir'
printf 'a,b\n' > '/tmp/space dir/my file.csv'
printf '[[extension]]\nextensions = ["csv"]\n[[extension.command]]\nlabel = "Echo"\nrun = "echo {path}"\n' > /tmp/q.toml
out=$(smartopen --config-path /tmp/q.toml --dry-run '/tmp/space dir/my file.csv')
case "$out" in
    *"echo '/tmp/space dir/my file.csv'"*) ;;
    *) fail "spaced path not quoted: $out" ;;
esac

# 6. The launched command's exit code is ours.
printf '[[shortcut]]\nlabel = "Seven"\nrun = "sh -c '\''exit 7'\''"\n' > /tmp/e.toml
set +e
smartopen --config-path /tmp/e.toml --command Seven --no-history
code=$?
set -e
[ "$code" = 7 ] || fail "expected exit 7, got $code"

# 7. The wizard reviews without a terminal and touches nothing.
smartopen --config-path /tmp/home/w.toml wizard --yes --dry-run 2>/tmp/wizard.log >/dev/null \
    || fail "wizard --yes --dry-run: $(cat /tmp/wizard.log)"
grep -q 'nothing written, nothing run' /tmp/wizard.log || fail "wizard did not reach the review"
[ ! -e /tmp/home/w.toml ] || fail "wizard --dry-run wrote a config"
printf 'wizard would use: %s\n' "$(grep -oE '^  (sudo )?[a-z-]+ (install|-S|add)' /tmp/wizard.log | sort -u | tr '\n' ';')"

# 8. The catalogue lists, and a pipe ends quietly.
smartopen tools list | head -1 >/dev/null || fail "tools list | head"

printf 'PASS %s\n' "$distro"
