#!/usr/bin/env bash
# The 60-second demo (docs/plan/phase-2.md, "The 60-second demo script"),
# run on the fictional dataset in fixtures/demo/. Every person and company
# in it is invented.
#
#   scripts/demo.sh             # paced for recording: typed commands, pauses
#   scripts/demo.sh --fast      # no typing effect, no pauses (rehearsal, CI)
#   scripts/demo.sh --preflight # warm the model cache, then exit
#
# Recording (P2-DOC-02, done by a human):
#   scripts/demo.sh --preflight          # with network: downloads the model once
#   # turn Wi-Fi off, then:
#   asciinema rec --idle-time-limit 1 -c scripts/demo.sh demo.cast
#   # the Claude Code segment (0:45) is recorded separately in Claude Code.
#
# Environment:
#   OST         the binary to run (default: `ost` on PATH, else target/release/ost)
#   DEMO_HOME   where the demo graph lives (default: a fresh temp dir; your
#               ~/.openstance is never touched)
#   DEMO_NOW    the clock for scoring (default 2026-10-15, the dataset's
#               "today"; without it recency drifts as real time passes)
#   TYPE_DELAY  seconds per typed character (default 0.035)
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fast=0
preflight=0
for arg in "$@"; do
  case "$arg" in
    --fast) fast=1 ;;
    --preflight) preflight=1 ;;
    -h | --help)
      sed -n '2,24p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "usage: scripts/demo.sh [--fast] [--preflight]" >&2
      exit 2
      ;;
  esac
done

if [ -n "${OST:-}" ]; then
  bin="$OST"
elif command -v ost > /dev/null 2>&1; then
  bin="$(command -v ost)"
elif [ -x "$repo/target/release/ost" ]; then
  bin="$repo/target/release/ost"
else
  echo "error: no ost binary; run \`cargo install --path crates/openstance\` or set OST" >&2
  exit 1
fi
# The demo cd's into its temp dir, so a relative OST must be made absolute.
case "$bin" in
  */*) bin="$(cd "$(dirname "$bin")" && pwd)/$(basename "$bin")" ;;
esac

now="${DEMO_NOW:-2026-10-15}"
home="${DEMO_HOME:-$(mktemp -d "${TMPDIR:-/tmp}/openstance-demo.XXXXXX")}"
exports="$home/exports"
export OPENSTANCE_HOME="$home/.openstance"
# A plain config: the demo must work with nothing but `init --me`.
unset OPENSTANCE_CONFIG

# `ost` as the viewer sees it; the pinned clock stays off screen.
ost() { "$bin" --now "$now" "$@"; }

fresh_graph() {
  rm -rf "$OPENSTANCE_HOME"
  ost init --me you@example.com > /dev/null
}

stage_exports() {
  rm -rf "$exports"
  mkdir -p "$exports"
  cp "$repo/fixtures/demo/takeout.mbox" "$repo/fixtures/demo/Connections.csv" \
    "$repo/fixtures/demo/contacts.vcf" "$exports/"
  # Sources without an export date in their name (Connections.csv) are
  # labelled with the file's mtime; date the copies on the demo's "today"
  # so evidence lines do not show the day the demo was run.
  touch -t "$(printf '%s' "$now" | tr -d -)1200" "$exports"/*
}

has_command() { "$bin" "$1" --help > /dev/null 2>&1; }

if [ "$preflight" -eq 1 ]; then
  # One full import downloads and caches the embedding model (about 33 MB)
  # when an embedder is configured; later runs need no network.
  stage_exports
  fresh_graph
  ost import "$exports"/* > /dev/null
  echo "preflight done: model cached, demo graph builds offline"
  exit 0
fi

pause() { [ "$fast" -eq 1 ] || sleep "$1"; }

# Print a prompt and "type" the command, then run it.
run() {
  local shown="$1"
  shift
  printf '\033[1;32m$\033[0m '
  if [ "$fast" -eq 1 ]; then
    printf '%s\n' "$shown"
  else
    local i
    for ((i = 0; i < ${#shown}; i++)); do
      printf '%s' "${shown:i:1}"
      sleep "${TYPE_DELAY:-0.035}"
    done
    sleep 0.3
    printf '\n'
  fi
  "$@"
}

stage_exports
fresh_graph
cd "$home"
clear 2> /dev/null || true

# 0:00-0:05  These are files you can download yourself today.
run "ls exports" ls exports
pause 2

# 0:05-0:15  One import: resolution, scoring and a short review queue.
run "ost import exports/*" ost import exports/takeout.mbox exports/Connections.csv exports/contacts.vcf
pause 3

# 0:15-0:27  Parsed filters, then ranked people with evidence lines.
#   The plan's line is "fintech founders I've actually met"; "met" means a
#   calendar meeting, and the .ics importer arrives in Phase 3, so this
#   dataset (mail, LinkedIn, contacts) asks who replied instead.
run 'ost search "top 5 fintech founders who replied"' \
  ost search "top 5 fintech founders who replied"
pause 4

# 0:27-0:35  A number you can argue with.
run 'ost who "Maya Chen"' ost who "Maya Chen"
pause 4

# 0:35-0:45  Three warm paths to someone you barely know.
if has_command how-connected; then
  run 'ost how-connected "Dana Whitfield"' ost how-connected "Dana Whitfield"
else
  echo "(how-connected is not in this build yet; showing Dana's mutuals via who)"
  run 'ost who "Dana Whitfield"' ost who "Dana Whitfield"
fi
pause 4

# 0:45-0:57  Claude Code (recorded separately, in Claude Code itself):
#   claude mcp add openstance -- ost mcp
#   prompt: "Who do I know at Lumen Pay, and who's my warmest way in to their CTO?"
#   expected: search_people (company = Lumen Pay), then how_connected
#   (Dana Whitfield), answered with the evidence lines from the tools.
# In the terminal recording, the same two questions as CLI calls:
run 'ost search "who do I know at Lumen Pay"' ost search "who do I know at Lumen Pay"
pause 3

# 0:57-1:00  Title card.
printf '\n\033[1mopenstance\033[0m: your network as a file. Free search, works in Claude Code.\n'
printf 'github.com/openstance/openstance\n\n'
printf '(all people and companies in this demo are fictional)\n'
