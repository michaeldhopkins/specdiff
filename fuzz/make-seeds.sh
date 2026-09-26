#!/usr/bin/env bash
# Regenerate the committed seed-* inputs for the fuzz targets from the E2E fixtures and
# fuzz/seed-src/, and the dictionaries from frameworks/*.toml. Run from the repo root:
#
#   fuzz/make-seeds.sh [fixtures-dir]     (default ~/projects/specdiff-tests/fixtures)
#
# outline seeds are `<path>\n<source>`, one per fixture file and per fuzz/seed-src file.
# outline_diff seeds are `<path>\0<base>\0<head>`, one per fixture file present on both sides,
# plus one per fuzz/seed-src file against a copy with its first `test` replaced by `check`.
# Writes fuzz/corpus/{outline,outline_diff}/seed-* and fuzz/dict/*.dict, overwriting both.
set -euo pipefail

FIXTURES="${1:-$HOME/projects/specdiff-tests/fixtures}"
OUT_O=fuzz/corpus/outline
OUT_D=fuzz/corpus/outline_diff
mkdir -p "$OUT_O" "$OUT_D"

seed_name() { printf '%s' "$1" | tr '/.' '__'; }

if [ -d "$FIXTURES" ]; then
  for fw in "$FIXTURES"/*/; do
    fw_name="$(basename "$fw")"
    (cd "$fw/base" 2>/dev/null && find . -type f | sed 's#^\./##') | while read -r rel; do
      { printf '%s\n' "$rel"; cat "$fw/base/$rel"; } > "$OUT_O/seed-$fw_name-base-$(seed_name "$rel")"
      if [ -f "$fw/head/$rel" ]; then
        { printf '%s\0' "$rel"; cat "$fw/base/$rel"; printf '\0'; cat "$fw/head/$rel"; } \
          > "$OUT_D/seed-$fw_name-$(seed_name "$rel")"
      fi
    done
    (cd "$fw/head" 2>/dev/null && find . -type f | sed 's#^\./##') | while read -r rel; do
      { printf '%s\n' "$rel"; cat "$fw/head/$rel"; } > "$OUT_O/seed-$fw_name-head-$(seed_name "$rel")"
    done
  done
else
  echo "no fixtures at $FIXTURES; only fuzz/seed-src seeds written" >&2
fi

# fuzz/seed-src/<framework>/<path>: snippets for frameworks the fixtures do not cover.
find fuzz/seed-src -type f | sort | while read -r f; do
  rest="${f#fuzz/seed-src/}"
  fw_name="${rest%%/*}"
  rel="${rest#*/}"
  { printf '%s\n' "$rel"; cat "$f"; } > "$OUT_O/seed-$fw_name-$(seed_name "$rel")"
  { printf '%s\0' "$rel"; cat "$f"; printf '\0'; awk '!done && sub(/test/, "check") { done = 1 } { print }' "$f"; } \
    > "$OUT_D/seed-$fw_name-$(seed_name "$rel")"
done

# The dictionary: every one-word string the framework TOMLs name (method names, attribute and
# decorator names, file suffixes; the AST node kinds come along and cost little), plus the
# punctuation the grammars hinge on and the outline_diff separator.
mkdir -p fuzz/dict
{
  grep -ho '"[^"\\]*"' frameworks/*.toml | grep -v ' ' | grep -v '\*' | sort -u
  printf '%s\n' '"\x00"' '"\x0a"' '"("' '")"' '"{"' '"}"' '"do"' '"end"' '"\x22"' '"'"'"'"' \
    '"#["' '"@"' '"->"' '"=>"' '"<?php\x0a"' '"fn "' '"def "' '"class "' '"mod "' '"$"' '"#{"'
} > fuzz/dict/outline.dict
cp fuzz/dict/outline.dict fuzz/dict/outline_diff.dict

echo "outline: $(find "$OUT_O" -name 'seed-*' | wc -l | tr -d ' ') seeds; outline_diff: $(find "$OUT_D" -name 'seed-*' | wc -l | tr -d ' ') seeds"
