#!/usr/bin/env bash
# Creates a new server from lspkit's template, and checks that the template still builds.
#
#   template.sh new <name> <dir>   create server <name> in <dir>, pinned to lspkit's pushed HEAD
#   template.sh check              generate the template into a scratch directory against this
#                                  checkout and run its lints, tests and arch checks
#   template.sh scratch <dir>      generate the template into <dir> against this checkout and keep
#                                  it there, for working on the template: edit, build, copy back
#
# `template/` uses `kitserver` as the placeholder for the name and `LSPKIT_REV` for the revision.
# It is never built in place. `check` generates it and builds the result, and CI runs `check` on
# every change to lspkit.
set -euo pipefail

here="$(cd "$(dirname "$0")/.." && pwd)"

die() {
  echo "template: $*" >&2
  exit 1
}

# Copies the template to `$2` as a server named `$1`. With `$3` = "path", lspkit is taken from this
# checkout by path rather than from git by revision.
generate() {
  local name="$1" dir="$2" source="$3"
  [[ "$name" =~ ^[a-z][a-z0-9]*$ ]] || die "a name is lowercase letters and digits: $name"
  [[ ! -e "$dir" ]] || die "$dir already exists"
  mkdir -p "$dir"
  cp -R "$here/template/." "$dir/"
  # The toolchain, formatting and license files are copied from lspkit, so there is one copy each.
  cp "$here/rust-toolchain.toml" "$here/rustfmt.toml" "$here/deny.toml" "$dir/"

  local file
  while IFS= read -r -d '' file; do
    sed -i.bak "s/kitserver/$name/g" "$file" && rm -f "$file.bak"
  done < <(find "$dir" -type f -print0)

  if [[ "$source" == path ]]; then
    # A dependency's features (`lspkit-heap`'s `override`) are carried over to the path form.
    sed -i.bak -E "s#^(lspkit-([a-z]+)) = \\{ git = \"[^\"]+\", rev = \"LSPKIT_REV\"(, features = \\[[^]]*\\])? \\}#\\1 = { path = \"$here/crates/\\2\"\\3 }#" \
      "$dir/Cargo.toml" && rm -f "$dir/Cargo.toml.bak"
  else
    local rev
    rev="$(git -C "$here" rev-parse HEAD)"
    git -C "$here" fetch --quiet origin
    git -C "$here" merge-base --is-ancestor "$rev" origin/main ||
      die "lspkit's HEAD is not on origin/main: a server fetches lspkit by git, so push first"
    sed -i.bak "s/LSPKIT_REV/$rev/g" "$dir/Cargo.toml" && rm -f "$dir/Cargo.toml.bak"
  fi
  ! grep -rq LSPKIT_REV "$dir" || die "a revision placeholder was left in $dir"
}

new() {
  local name="${1:?usage: template.sh new <name> <dir>}" dir="${2:?usage: template.sh new <name> <dir>}"
  # Checked before the trap is set, so an existing directory is never removed.
  [[ ! -e "$dir" ]] || die "$dir already exists"
  # Removes a half-built directory on failure, so the next run does not stop at "already exists".
  # The trap is cleared once the server is complete.
  trap 'rm -rf "$dir"' EXIT
  generate "$name" "$dir" git
  (cd "$dir" && git init --quiet && cargo generate-lockfile)
  trap - EXIT
  cat <<EOF
$name is in $dir, on lspkit $(git -C "$here" rev-parse --short HEAD).

  cd $dir && just pr-ready      the gate: lints, tests, arch checks, licenses
  just perf <project>           check the budgets on real files

Its CLAUDE.md imports ../lspkit/docs/working.md, so keep the checkout next to lspkit's. Then
replace the example check in crates/check with the language's own, add the server to lspkit's
consumers (tools/consumers.sh) so \`just bump\` moves it too, and commit.
EOF
}

check() {
  local scratch
  scratch="$(mktemp -d)"
  # shellcheck disable=SC2064 # the path is fixed now, on purpose
  trap "rm -rf '$scratch'" EXIT
  generate kitserver "$scratch/kitserver" path
  # Keep the generated server's build output between runs, under lspkit's target directory.
  export CARGO_TARGET_DIR="$here/target/template"
  cd "$scratch/kitserver"
  echo "== the template, generated in $scratch/kitserver"
  cargo fmt --all --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
}

case "${1:-}" in
  new) shift; new "$@" ;;
  check) check ;;
  scratch) generate kitserver "${2:?usage: template.sh scratch <dir>}" path ;;
  *) die "usage: template.sh new <name> <dir> | check | scratch <dir>" ;;
esac
