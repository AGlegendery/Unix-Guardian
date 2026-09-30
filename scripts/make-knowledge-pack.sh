#!/usr/bin/env bash
# Regenerates claude-project/source/: the current code as Markdown, one file per
# crate plus one for everything else, ready to upload to a Claude Project.
#
#   ./scripts/make-knowledge-pack.sh
#
# Run it after every change you want the project to know about, then replace
# the source/*.md files in the Project. The folder is git-ignored: it is a
# generated copy of what the repository already holds.
set -euo pipefail
cd "$(dirname "$0")/.."

out=claude-project/source
rm -rf "$out"
mkdir -p "$out"

rev="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
branch="$(git branch --show-current 2>/dev/null || echo unknown)"
dirty="$(git status --porcelain 2>/dev/null | grep -vc '^?? claude-project/source' || true)"
stamp="$(date -Iseconds)"

header() {
  echo "# Source snapshot: $1"
  echo
  echo "Generated $stamp from commit \`$rev\` on branch \`$branch\`$( [ "$dirty" -gt 0 ] && echo " **plus $dirty uncommitted changes**" )."
  echo "This is a copy; the repository is the truth. If this snapshot may be stale, ask the user for the current file."
  echo
}

lang() {
  case "$1" in
    *.rs) echo rust ;; *.toml) echo toml ;; *.json) echo json ;; *.sh) echo bash ;;
    *.py) echo python ;; *.yml|*.yaml) echo yaml ;; *.md) echo markdown ;;
    *.policy|*.conf) echo xml ;; *.service) echo ini ;; *) echo text ;;
  esac
}

# Four-backtick fences so files that contain ``` blocks (READMEs) stay intact.
emit() {
  printf '\n## `%s`\n\n````%s\n' "$1" "$(lang "$1")"
  cat "$1"
  printf '\n````\n'
}

for crate in crates/*/; do
  name="$(basename "$crate")"
  {
    header "$name"
    find "$crate" -type f \( -name '*.rs' -o -name 'Cargo.toml' \) -not -path '*/fixtures/*' | sort |
      while read -r f; do emit "$f"; done
  } > "$out/$name.md"
done

{
  header "workspace (configs, data, scripts, packaging, CI, docs)"
  for f in Cargo.toml rust-toolchain.toml rustfmt.toml clippy.toml deny.toml .gitignore \
           README.md CLAUDE.md docs/*.md data/games.json scripts/* packaging/*/* .github/workflows/*.yml; do
    [ -f "$f" ] && emit "$f"
  done
} > "$out/workspace.md"

{
  echo "# Source snapshot index"
  echo
  echo "Commit \`$rev\` on \`$branch\`, generated $stamp."
  echo
  echo "| File | Size | Covers |"
  echo "|---|---|---|"
  for f in "$out"/*.md; do
    [ "$(basename "$f")" = INDEX.md ] && continue
    n="$(grep -c '^## `' "$f")"
    printf '| %s | %s KB | %s files |\n' "$(basename "$f")" "$(( $(wc -c < "$f") / 1024 ))" "$n"
  done
} > "$out/INDEX.md"

echo "wrote $(ls "$out" | wc -l) files to $out ($(du -sh "$out" | cut -f1))"
