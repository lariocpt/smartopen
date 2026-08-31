// smartopen — MIRROR the public GitHub release onto the LAN apps plane.
//
//     curl -fsSL https://lariocpt.github.io/smartopen/install.sh | sh          (everyone)
//     curl -fsSL https://apps.in.drlario.org/install.sh | bash -s -- smartopen  (here)
//
// THIS PIPELINE NO LONGER BUILDS ANYTHING. It used to `cargo build --release` in a
// bookworm container and publish its own artifact under the tool name `opn`, which meant
// two independent builds of one commit could land in two places. The GitHub Release is
// now the single source of truth — eight archives, one attested SHA256SUMS — and
// /srv/apps holds a byte-identical copy of the x86_64 musl build, asserted below, not
// assumed. Same shape as color-terminal's job since its 2.0.0 release.
//
// The plane serves the two binaries as plain files, `smartopen` and `opn`, because
// install.sh installs a tool BY FILE NAME: `install.sh -s -- smartopen` installs both,
// and `install.sh -s -- opn` still resolves — do_install matches an argument against the
// tool name OR a file name — so nothing already set up on the estate breaks.
//
// Static musl means the glibc-floor gate this file used to carry is gone: the binary
// imports no shared library at all, and the Gate stage asserts exactly that.
pipeline {
    agent any
    options {
        disableConcurrentBuilds()
        // Every fifteen minutes is ~100 runs a day; keep enough that the last run that
        // actually mirrored something is still there to read.
        buildDiscarder(logRotator(numToKeepStr: '200'))
        timeout(time: 20, unit: 'MINUTES')
    }
    // GitHub cannot reach this Jenkins, so it asks rather than being told. Polling the
    // /releases/latest redirect needs no token and spends no API rate limit.
    triggers { cron('H/15 * * * *') }
    parameters {
        string(name: 'TAG', defaultValue: '',
               description: 'Release tag to mirror, e.g. v0.2.0 (or v0.3.0-rc.1 to rehearse). Empty = whatever GitHub calls latest.')
        booleanParam(name: 'FORCE', defaultValue: false,
               description: 'Republish even if this version is already on the plane.')
        booleanParam(name: 'PROVENANCE', defaultValue: true,
               description: 'Verify the GitHub build attestation on the archive before publishing.')
    }
    environment {
        TOOL    = 'smartopen'
        GH_REPO = 'lariocpt/smartopen'
        TARGET  = 'x86_64-unknown-linux-musl'
    }
    stages {

        stage('Resolve') {
            steps {
                script {
                    // Egress is checked HERE, before anything depends on it: this stage
                    // runs every fifteen minutes, and a quiet NOT_BUILT beats a red build.
                    def online = sh(returnStatus: true, script: '''
                        curl -fsS -o /dev/null -m 20 https://github.com
                    ''') == 0
                    if (!online) {
                        currentBuild.result = 'NOT_BUILT'
                        currentBuild.description = 'no route to github.com'
                        env.NEEDED = '0'
                        return
                    }

                    // The /releases/latest redirect lands on /releases/tag/vX.Y.Z and
                    // excludes drafts and prereleases — exactly the set to mirror. With no
                    // release yet it lands on /releases and the tag comes back as
                    // "releases"; while the repository is private it is a 404 and comes
                    // back empty. Both are a quiet NOT_BUILT, not an error, for the same
                    // reason as above.
                    env.TAG = params.TAG?.trim() ?: sh(returnStdout: true, script: '''
                        url=$(curl -fsS -o /dev/null -w '%{url_effective}' -L -I \
                              "https://github.com/${GH_REPO}/releases/latest" || true)
                        printf '%s' "${url##*/}"
                    ''').trim()

                    if (!(env.TAG ==~ /^v[0-9][0-9A-Za-z.+-]*$/)) {
                        currentBuild.result = 'NOT_BUILT'
                        currentBuild.description = "no release to mirror (got '${env.TAG}')"
                        env.NEEDED = '0'
                        return
                    }
                    env.VERSION = env.TAG.substring(1)

                    // Already mirrored? Then this poll is a no-op, and the build stops
                    // rather than churning /srv/apps every fifteen minutes. The version
                    // reaches the shell through the environment, never by interpolation:
                    // a Groovy string with a tag name inside shell quotes is an injection
                    // waiting for a tag with a quote in it.
                    def onPlane = sh(returnStatus: true, script: '''
                        awk -F'\t' -v v="$VERSION" \
                            '$1=="tool" && $2=="smartopen" && $3==v && index($7,"/latest/")>0 {x++} END{exit !x}' \
                            /srv/apps/index.tsv
                    ''') == 0
                    env.NEEDED = (onPlane && !params.FORCE) ? '0' : '1'

                    echo "tag=${env.TAG} version=${env.VERSION} onPlane=${onPlane} needed=${env.NEEDED}"
                    if (env.NEEDED == '0') {
                        currentBuild.result = 'NOT_BUILT'
                        currentBuild.description = "${env.TAG} already mirrored"
                    } else {
                        currentBuild.description = "mirroring ${env.TAG}"
                    }
                }
            }
        }

        stage('Preflight') {
            when { environment name: 'NEEDED', value: '1' }
            steps {
                sh '''
                    set -eu
                    test -w /srv/apps || { echo "/srv/apps not writable"; exit 1; }
                    test -x /opt/publish/bin/apps-publish || { echo "apps-publish not mounted"; exit 1; }
                    command -v curl      >/dev/null || { echo "curl missing — needed to fetch the release"; exit 1; }
                    command -v sha256sum >/dev/null || { echo "sha256sum missing — nothing may publish unverified"; exit 1; }
                    command -v tar       >/dev/null || { echo "tar missing — the release is an archive"; exit 1; }
                    command -v objdump   >/dev/null || { echo "objdump missing — the static-link gate needs binutils"; exit 1; }
                    curl -fsS -o /dev/null -m 20 https://github.com \
                        || { echo "no egress to github.com — mirror mode needs it"; exit 1; }
                '''
            }
        }

        // The checkout has to BE the tag: the version string the archive reports is
        // compared against Cargo.toml here, so a green run cannot prove main while
        // publishing bytes built from something else.
        stage('Source') {
            when { environment name: 'NEEDED', value: '1' }
            steps {
                sh '''
                    set -eu
                    git fetch --tags --force origin
                    git -c advice.detachedHead=false checkout --detach "refs/tags/${TAG}"
                    v=$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\\([^"]*\\)".*/\\1/p' Cargo.toml | head -1)
                    base=${VERSION%%-*}
                    [ "$v" = "$base" ] \
                      || { echo "FAIL: $TAG is checked out but Cargo.toml says version = \\"$v\\""; exit 1; }
                    echo "source: $TAG (Cargo.toml version $v)"
                '''
            }
        }

        stage('Fetch') {
            when { environment name: 'NEEDED', value: '1' }
            steps {
                sh '''
                    set -eu
                    rm -rf dist out; mkdir -p dist out
                    base="https://github.com/${GH_REPO}/releases/download/${TAG}"
                    archive="smartopen-${TARGET}-${TAG}.tar.gz"
                    for f in "$archive" SHA256SUMS; do
                        curl -fsSL --proto '=https' --retry 5 --retry-all-errors -o "dist/$f" "$base/$f"
                    done

                    # SHA256SUMS names each archive with no directory component, so it
                    # verifies from inside the directory holding it — the same way
                    # docs/install.sh and a human check it.
                    ( cd dist && grep -F "  $archive" SHA256SUMS | sha256sum -c - )

                    tar -xzf "dist/$archive" -C out smartopen opn
                    # Release archives carry the bit, but never rely on it.
                    chmod 0755 out/smartopen out/opn
                    sha256sum "dist/$archive" | awk '{print $1}' > .released-sha
                    sha256sum out/smartopen | awk '{print $1}' > .smartopen-sha
                    sha256sum out/opn       | awk '{print $1}' > .opn-sha
                    echo "fetched ${TAG}: archive $(cat .released-sha)"
                '''
            }
        }

        // What makes "mirror" mean something: proof the released archive was built by
        // the repository's own release workflow from this tag, without rebuilding it.
        // A Rust build is not byte-reproducible across machines the way color-terminal's
        // normalised tar is, so the proof is GitHub's build attestation, not a rebuild.
        stage('Provenance') {
            when {
                allOf {
                    environment name: 'NEEDED', value: '1'
                    expression { params.PROVENANCE }
                }
            }
            steps {
                withCredentials([usernamePassword(credentialsId: 'github-pat',
                        usernameVariable: 'GH_USER', passwordVariable: 'GH_TOKEN')]) {
                    sh '''
                        set -eu
                        command -v gh >/dev/null \
                          || { echo "FAIL: gh is not installed and PROVENANCE is on; add gh to the Jenkins image or run with PROVENANCE=false"; exit 1; }
                        gh attestation verify "dist/smartopen-${TARGET}-${TAG}.tar.gz" --repo "$GH_REPO" \
                          || { echo "FAIL: the archive carries no valid build attestation from $GH_REPO"; exit 1; }
                        echo "provenance: attested build of $GH_REPO at $TAG"
                    '''
                }
            }
        }

        stage('Gate') {
            when { environment name: 'NEEDED', value: '1' }
            steps {
                sh '''
                    set -eu

                    # Static: the musl build imports no shared library. This replaces the
                    # glibc-floor check that lived here when the job built on bookworm.
                    needed=$(objdump -p out/smartopen | grep -c NEEDED || true)
                    [ "$needed" = 0 ] || { echo "FAIL: out/smartopen has $needed NEEDED entries; the mirror serves the static build"; exit 1; }

                    # An -rc tag is a rehearsal of the version it is a candidate for, and
                    # the binary reports that version: compare the base, as Source does.
                    want="smartopen ${VERSION%%-*}"
                    for bin in smartopen opn; do
                        got=$(./out/$bin --version | tr -d '\\r')
                        [ "$got" = "$want" ] \
                          || { echo "FAIL: out/$bin reports '$got', expected '$want'"; exit 1; }
                    done

                    # The CLI smoke, in a sandboxed home — never the jenkins user's config.
                    tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
                    export HOME="$tmp" XDG_CONFIG_HOME="$tmp/.config" XDG_STATE_HOME="$tmp/.state"
                    ./out/smartopen config sample > "$tmp/c.toml"
                    ./out/smartopen --config-path "$tmp/c.toml" config doctor >/dev/null \
                      || { echo "FAIL: doctor on the sample config exited non-zero"; exit 1; }
                    mkdir -p "$tmp/space dir"; printf 'a,b\\n' > "$tmp/space dir/my file.csv"
                    printf '[[extension]]\\nextensions = ["csv"]\\n[[extension.command]]\\nlabel = "E"\\nrun = "echo {path}"\\n' > "$tmp/q.toml"
                    ./out/smartopen --config-path "$tmp/q.toml" --dry-run "$tmp/space dir/my file.csv" \
                      | grep -F "echo '$tmp/space dir/my file.csv'" >/dev/null \
                      || { echo "FAIL: spaced path not quoted"; exit 1; }
                    printf '[[shortcut]]\\nlabel = "Seven"\\nrun = "sh -c '"'"'exit 7'"'"'"\\n' > "$tmp/e.toml"
                    set +e; ./out/smartopen --config-path "$tmp/e.toml" --command Seven --no-history; code=$?; set -e
                    [ "$code" = 7 ] || { echo "FAIL: expected the command's exit 7, got $code"; exit 1; }
                    ./out/smartopen --config-path "$tmp/w.toml" wizard --yes --dry-run >/dev/null 2>"$tmp/wiz.log" \
                      || { echo "FAIL: wizard --yes --dry-run"; cat "$tmp/wiz.log"; exit 1; }
                    [ ! -e "$tmp/w.toml" ] || { echo "FAIL: wizard --dry-run wrote a config"; exit 1; }

                    # And nothing above touched what we are about to publish.
                    [ "$(sha256sum out/smartopen | awk '{print $1}')" = "$(cat .smartopen-sha)" ] \
                      || { echo "FAIL: out/smartopen changed during the gate"; exit 1; }
                    echo "gate: static, versioned, doctor/quoting/exit-code/wizard ok"
                '''
            }
        }

        stage('Publish') {
            when { environment name: 'NEEDED', value: '1' }
            steps {
                sh '''
                    set -eu
                    # The bare tag version, with no +sha: there is exactly one artifact per
                    # version now and the tag names it uniquely, so the plane's row, the
                    # release page and `smartopen --version` all print one string.
                    /opt/publish/bin/apps-publish bin "$TOOL" "$VERSION" \
                        "$WORKSPACE/out/smartopen" "$WORKSPACE/out/opn"
                '''
            }
        }

        stage('Verify') {
            when { environment name: 'NEEDED', value: '1' }
            steps {
                sh '''
                    set -eu

                    # Assert the /latest/ rows specifically. apps-reindex emits the concrete
                    # version row whether or not `latest` was minted, but install.sh only
                    # reads rows whose path goes through latest/ — so checking the concrete
                    # row can pass while no client can see the artifact.
                    for bin in smartopen opn; do
                        sha=$(awk -F'\\t' -v v="$VERSION" -v f="$bin" \
                            '$1=="tool" && $2=="smartopen" && $3==v && $4==f && index($7,"/latest/")>0 {print $5; exit}' \
                            /srv/apps/index.tsv)
                        [ -n "$sha" ] || { echo "FAIL: no /latest/ row for smartopen $VERSION file $bin"; exit 1; }
                        # Byte identity with the public release is the entire premise of
                        # mirror mode, so assert it rather than trust it.
                        [ "$sha" = "$(cat ".$bin-sha")" ] \
                          || { echo "FAIL: /srv/apps serves $sha for $bin, release $TAG has $(cat ".$bin-sha")"; exit 1; }
                    done
                    echo "byte identity: /srv/apps == release $TAG (smartopen, opn)"

                    # And end to end, the way a machine actually gets it — including the
                    # old name, which resolves by file name.
                    for n in smartopen opn; do
                        curl -fsSL https://apps.in.drlario.org/install.sh | bash -s -- --list | grep -q "$n" \
                            || { echo "FAIL: install.sh does not list $n"; exit 1; }
                    done
                    echo "mirrored smartopen $VERSION from $TAG"
                '''
            }
        }
    }
}
