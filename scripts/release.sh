#!/usr/bin/env bash
# Prepares a release: sets the version everywhere, commits and tags it.
# Pushes nothing.
#
#   ./scripts/release.sh 0.1.0-beta.2      (or 0.2.0)
#
# Then `git push origin HEAD v0.1.0-beta.2` starts .github/workflows/release.yml,
# which builds the packages, signs them and makes a *draft* release on
# GitHub. Read it there and press Publish; until then nobody sees it and
# no Unix Guardian offers it. A version with a "-" is a pre-release, which
# only users of a pre-release are offered.
set -euo pipefail
v="${1:?usage: release.sh VERSION (e.g. 0.1.0-beta.2 or 0.2.0)}"
v="${v#v}"
[[ "$v" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] || { echo "not a version: $v (like 0.2.0 or 0.2.0-beta.1)" >&2; exit 1; }
cd "$(dirname "$0")/.."
[ -z "$(git status --porcelain --untracked-files=no)" ] || { echo "commit or stash your changes first" >&2; exit 1; }
git rev-parse -q --verify "refs/tags/v$v" >/dev/null && { echo "tag v$v exists already" >&2; exit 1; }
# Arch's pkgver has no "-": 0.1.0-beta.2 → 0.1.0beta2.
pkgver=$(sed -E 's/-//; s/([a-z]+)\./\1/g' <<<"$v")

sed -i -E '/^\[workspace.package\]/,/^\[/ s/^version = ".*"/version = "'"$v"'"/' Cargo.toml
sed -i -E '0,/^version = ".*"/ s//version = "'"$v"'"/' app/src-tauri/Cargo.toml
sed -i -E '0,/"version": ".*"/ s//"version": "'"$v"'"/' app/src-tauri/tauri.conf.json app/package.json
sed -i -E "s/^pkgver=.*/pkgver=$pkgver/; s/^_tag=.*/_tag=$v/; s/^pkgrel=.*/pkgrel=1/" packaging/arch/PKGBUILD
cargo update --workspace --quiet
(cd app/src-tauri && cargo update --package ug-app --quiet)
(cd app && npm install --package-lock-only --silent >/dev/null 2>&1 || true)

git add Cargo.toml Cargo.lock app/src-tauri/Cargo.toml app/src-tauri/Cargo.lock app/src-tauri/tauri.conf.json app/package.json app/package-lock.json packaging/arch/PKGBUILD
git commit -q -m "Release v$v"
git tag -a "v$v" -m "Unix Guardian $v"
echo "Tagged v$v. To build it on GitHub (as a draft release you publish yourself):"
echo "  git push origin HEAD v$v"
