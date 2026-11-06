#!/usr/bin/env bash
# Moves the servers built on lspkit along with it. See `CLAUDE.md` §2.
#
#   consumers.sh try  <server-dir>             run a server's lints and tests against this checkout
#   consumers.sh bump <why> [<server-dir>...]  move every lspkit pin to HEAD, run the gate, commit
#
# `try` needs nothing pushed and leaves the server's files as they were. Cargo is pointed at this
# checkout with `--config patch...` on the command line. That rewrites the server's Cargo.lock, so
# the lockfile is restored afterwards. No `.cargo/config.toml` is written, because one above the
# servers would affect every build under it, including another agent's.
#
# `bump` is the last step of every change here. It sets the `rev` of every lspkit dependency of
# each server to the pushed HEAD and runs that server's `just pr-ready`. If the gate is green and
# only the manifest and lockfile changed, it commits "Take lspkit <rev>, <why>". Otherwise (a red
# gate, or a change that needs server code) it reports the server and leaves the bump uncommitted
# in its tree, so a breaking change lands with its fix in the server's own commit.
set -euo pipefail

here="$(cd "$(dirname "$0")/.." && pwd)"
# lspkit's git URL. `bump` moves every server's pins to it. `try` patches whatever URL a server's
# manifest names, so it also works on a server not bumped since lspkit's URL changed.
url="https://github.com/koroblabs/lspkit.git"
servers_default=("$here/../phrost" "$here/../orthra")

die() {
  echo "consumers: $*" >&2
  exit 1
}

# Prints the `--config patch...` arguments that point a server at every crate of this checkout.
# They are keyed by the lspkit URL in the server's manifest, since cargo matches a patch by URL.
patch_args() {
  local server="$1" dir name named
  named="$(grep -oE 'git = "[^"]*lspkit\.git"' "$server/Cargo.toml" | head -1 | sed -E 's/git = "(.*)"/\1/')"
  [[ -n "$named" ]] || die "$server/Cargo.toml names no lspkit git dependency"
  for dir in "$here"/crates/*/; do
    name="$(sed -n 's/^name = "\(.*\)"$/\1/p' "$dir/Cargo.toml" | head -1)"
    printf -- '--config\npatch."%s".%s.path="%s"\n' "$named" "$name" "${dir%/}"
  done
}

try() {
  local server="${1:?usage: consumers.sh try <server-dir>}"
  [[ -f "$server/Cargo.lock" ]] || die "$server has no Cargo.lock"
  local saved
  saved="$(mktemp)"
  cp "$server/Cargo.lock" "$saved"
  # shellcheck disable=SC2064 # the paths are fixed now, on purpose
  trap "cp '$saved' '$server/Cargo.lock'; rm -f '$saved'" EXIT

  local args=()
  while IFS= read -r line; do args+=("$line"); done < <(patch_args "$server")

  echo "== $(basename "$server") against $here"
  (cd "$server" && cargo clippy --workspace --all-targets "${args[@]}" -- -D warnings)
  (cd "$server" && cargo nextest run --workspace "${args[@]}")
}

bump() {
  local why="${1:?usage: consumers.sh bump <why> [<server-dir>...]}"
  shift
  local servers=("$@")
  [[ ${#servers[@]} -gt 0 ]] || servers=("${servers_default[@]}")

  [[ -z "$(git -C "$here" status --porcelain)" ]] || die "lspkit has uncommitted changes"
  git -C "$here" fetch --quiet origin
  local rev short
  rev="$(git -C "$here" rev-parse HEAD)"
  short="$(git -C "$here" rev-parse --short HEAD)"
  git -C "$here" merge-base --is-ancestor "$rev" origin/main ||
    die "$short is not on origin/main: a server fetches lspkit by git, so push first"

  local server failed=()
  for server in "${servers[@]}"; do
    server="$(cd "$server" && pwd)"
    local name
    name="$(basename "$server")"
    echo "== $name"
    if [[ -n "$(git -C "$server" status --porcelain)" ]]; then
      echo "$name has uncommitted changes; skipped, so no one else's work is committed"
      failed+=("$name")
      continue
    fi
    # Rewrites the URL as well as the rev, so a server pinned before lspkit's URL changed moves too.
    sed -i.bak -E "/lspkit\\.git/ { s#git = \"[^\"]*lspkit\\.git\"#git = \"$url\"#; s/rev = \"[0-9a-f]{40}\"/rev = \"$rev\"/; }" "$server/Cargo.toml"
    rm -f "$server/Cargo.toml.bak"
    # The sed only rewrites `rev = "<40 hex>"` on a line naming lspkit.git. A branch, a tag or a
    # multi-line table would be missed and "already on <rev>" would be wrong, so count the lines.
    local pinned total
    total="$(grep -c 'lspkit\.git' "$server/Cargo.toml" || true)"
    pinned="$(grep 'lspkit\.git' "$server/Cargo.toml" | grep -c "$rev" || true)"
    [[ "$total" -gt 0 ]] || die "$name: no lspkit git dependency in its Cargo.toml"
    [[ "$total" -eq "$pinned" ]] ||
      die "$name: $((total - pinned)) of $total lspkit.git dependency lines do not carry rev $short after the bump (written as a branch, a tag, or over several lines?)"
    if git -C "$server" diff --quiet -- Cargo.toml; then
      echo "$name is already on $short"
      continue
    fi
    if ! (cd "$server" && just pr-ready); then
      echo "$name: the gate is red on $short; the bump is left in its tree to be fixed there"
      failed+=("$name")
      continue
    fi
    if [[ -n "$(git -C "$server" status --porcelain -- . ':!Cargo.toml' ':!Cargo.lock')" ]]; then
      echo "$name: the gate changed more than the pins; left in its tree to be read and committed"
      failed+=("$name")
      continue
    fi
    git -C "$server" commit --quiet -m "Take lspkit $short, $why" -- Cargo.toml Cargo.lock
    echo "$name: committed \"Take lspkit $short, $why\""
  done

  [[ ${#failed[@]} -eq 0 ]] || die "not taken by: ${failed[*]}"
}

case "${1:-}" in
  try) shift; try "$@" ;;
  bump) shift; bump "$@" ;;
  *) die "usage: consumers.sh try <server-dir> | bump <why> [<server-dir>...]" ;;
esac
