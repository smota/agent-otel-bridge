#!/usr/bin/env bash
# Build only the production hook with the reviewed Linux native link contract.
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
native_host=$(rustc -vV | sed -n 's/^host: //p')
if [[ $(uname -s) != Linux || $native_host != x86_64-unknown-linux-gnu || ${CARGO_BUILD_TARGET:-$native_host} != "$native_host" ]]; then
  echo 'The native hook packager requires a native x86_64-unknown-linux-gnu host and target.' >&2
  exit 2
fi
for command in cargo readelf stat sha256sum install cp mv; do
  command -v "$command" >/dev/null || { echo "Required build tool unavailable: $command" >&2; exit 2; }
done

target_dir=${CARGO_TARGET_DIR:-$repo_root/target}
[[ $target_dir == /* ]] || target_dir=$repo_root/$target_dir
linker_script=$repo_root/crates/agent-otel-client/link/linux-hook.ld
# Cargo's dep-info does not track linker-script contents. An immutable,
# content-addressed copy makes a script-only edit change the final link argument.
linker_digest=$(sha256sum -- "$linker_script")
linker_digest=${linker_digest%% *}
linker_cache=$target_dir/native-hook-link/$linker_digest
mkdir -p -- "$linker_cache"
cached_script=$linker_cache/linux-hook.ld
if [[ ! -e $cached_script && ! -L $cached_script ]]; then
  temporary_script=$(mktemp "$linker_cache/.stage.XXXXXX")
  cp -- "$linker_script" "$temporary_script"
  chmod 0444 -- "$temporary_script"
  mv -n -- "$temporary_script" "$cached_script"
  rm -f -- "$temporary_script"
fi
cached_digest=$(sha256sum -- "$cached_script")
cached_digest=${cached_digest%% *}
if [[ -L $cached_script || $cached_digest != "$linker_digest" ]]; then
  echo 'Rejected native hook: immutable linker-script cache differs from its content address.' >&2
  exit 1
fi
cd -- "$repo_root"
# An explicit native target prevents inherited cross-target configuration.
# Extra rustc arguments affect only agent-hook, never dependencies or tests.
CARGO_TARGET_DIR="$target_dir" cargo rustc --release --locked \
  --target "$native_host" -p agent-otel-client --bin agent-hook -- \
  -C panic=abort \
  -C link-arg=-Wl,--no-eh-frame-hdr \
  -C "link-arg=-Wl,-T,$cached_script"

built=$target_dir/$native_host/release/agent-hook
headers=$(mktemp)
sections=$(mktemp)
dynamic=$(mktemp)
trap 'rm -f -- "$headers" "$sections" "$dynamic"' EXIT
readelf -lW "$built" > "$headers"
readelf -SW "$built" > "$sections"
readelf -dW "$built" > "$dynamic"
if grep -q 'GNU_EH_FRAME' "$headers" || grep -Eq '\.eh_frame([ ._]|$)' "$sections"; then
  echo 'Rejected native hook: unwind sections or a stale unwind program header remain.' >&2
  exit 1
fi
grep -q 'GNU_RELRO' "$headers" || { echo 'Rejected native hook: RELRO is missing.' >&2; exit 1; }
grep -q 'GNU_STACK' "$headers" || { echo 'Rejected native hook: stack protection is unspecified.' >&2; exit 1; }
grep -Eq 'BIND_NOW|FLAGS_1.*NOW' "$dynamic" || { echo 'Rejected native hook: immediate binding is missing.' >&2; exit 1; }
if grep -q '(TEXTREL)' "$dynamic"; then
  echo 'Rejected native hook: dynamic text relocations.' >&2
  exit 1
fi
grep -q '\.gcc_except_table' "$sections" || { echo 'Rejected native hook: exception table was not retained.' >&2; exit 1; }
awk '$1 == "LOAD" && /W/ && /E/ { bad=1 } $1 == "GNU_STACK" && /E/ { bad=1 } END { exit bad }' "$headers" || {
  echo 'Rejected native hook: writable executable segment or executable stack.' >&2
  exit 1
}
size=$(stat -c '%s' "$built")
if (( size >= 300000 )); then
  echo "Rejected native hook: $size bytes reaches the strict 300000-byte limit." >&2
  exit 1
fi
# Ordinary Cargo builds may overwrite this conventional staging path. Publish
# the checked native result last, after every other build, before installation.
mkdir -p -- "$target_dir/release"
install -m 0755 -- "$built" "$target_dir/release/agent-hook"
printf 'Native Linux hook: %s bytes; ' "$size"
sha256sum -- "$target_dir/release/agent-hook"
