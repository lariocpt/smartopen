# The website

`docs/` is served as-is by GitHub Pages (`.github/workflows/pages.yml`): no build step, no
generator, no dependency. What is committed is byte-for-byte what is served.

- `index.html` — the whole site, one self-contained file. Google Fonts is its only external
  request; everything else is inline. It renders in light and dark (`prefers-color-scheme`,
  overridable with `data-theme="light|dark"` on `<html>`), on a phone, and with no JS.
- `install.sh` — the public installer. **POSIX sh**, not bash: it is piped to whatever
  `/bin/sh` a stranger has. CI runs `sh -n`, `dash -n` and `shellcheck -s sh` on it; do
  not add it to a bash shellcheck list, which would accept every bashism.
- `.nojekyll` — tells Pages not to run Jekyll over these files.

Hard rules for `index.html`: every command shown on the page must exist in the README;
nothing on the page claims a platform result the CI does not verify; the install one-liner
is the same string in two places (site, README) and `release.yml` fetches it from the
live site after a release — change both or none.
