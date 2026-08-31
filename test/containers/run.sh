#!/usr/bin/env bash
# test/containers/run.sh — L4: prove "cross-Linux-distro" against real distributions.
#
# Builds the static musl release, packages it exactly the way release.yml does, then in
# each container installs it through docs/install.sh (from a file:// base, so this runs
# before a release exists) and runs test/containers/smoke.sh. Alpine proves the musl
# claim — no glibc at all; bookworm is the oldest glibc the LAN plane serves; ubuntu
# 22.04 the oldest GitHub runner image; fedora the rpm family; arch the developer's box.
#
# Needs podman (or docker via CONTAINER_TOOL=docker). Exit 1 if any distro fails.
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.." || exit 1
REPO=$PWD

TOOL=${CONTAINER_TOOL:-podman}
TARGET=${SMARTOPEN_TARGET:-x86_64-unknown-linux-musl}
read -ra IMAGES <<< "${SMARTOPEN_IMAGES:-docker.io/library/alpine:3 docker.io/library/debian:bookworm docker.io/library/ubuntu:22.04 docker.io/library/fedora:latest docker.io/library/archlinux:latest}"

if ! command -v "$TOOL" >/dev/null 2>&1; then
    echo "$TOOL is not installed — skipping the distro container tests."
    exit 0
fi

# Package like the release: smartopen, opn, README.md, LICENSE at the archive root, one
# SHA256SUMS generated from inside the directory so the names carry no path component.
version=$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' Cargo.toml | head -1)
tag="v${version}-local"
DIST=$REPO/dist
rm -rf "$DIST"; mkdir -p "$DIST/stage"
echo "building $TARGET ..."
cargo build --release --locked --target "$TARGET" >/dev/null || { echo "build failed"; exit 1; }
cp "target/$TARGET/release/smartopen" "target/$TARGET/release/opn" README.md LICENSE "$DIST/stage/"
archive="smartopen-$TARGET-$tag.tar.gz"
tar -C "$DIST/stage" -czf "$DIST/$archive" smartopen opn README.md LICENSE
( cd "$DIST" && sha256sum "$archive" > SHA256SUMS && sha256sum -c SHA256SUMS >/dev/null )
rm -rf "$DIST/stage"
echo "packaged $DIST/$archive"

printf '\n%-32s %s\n' IMAGE RESULT
printf '%.0s-' {1..48}; printf '\n'
FAIL=0
for image in "${IMAGES[@]}"; do
    log=$REPO/test/containers/last-$(basename "${image%%:*}").log
    rm -f "$log"
    if "$TOOL" run --rm \
        -v "$DIST:/dist:ro" \
        -v "$REPO/docs/install.sh:/install.sh:ro" \
        -v "$REPO/test/containers/smoke.sh:/smoke.sh:ro" \
        "$image" sh /smoke.sh >"$log" 2>&1
    then
        printf '%-32s PASS  %s\n' "$image" "$(grep -m1 'wizard would use' "$log" | sed 's/wizard would use: //')"
        rm -f "$log"
    else
        printf '%-32s FAIL  see %s\n' "$image" "${log#"$REPO"/}"
        tail -5 "$log" | sed 's/^/    /'
        FAIL=1
    fi
done
exit $FAIL
