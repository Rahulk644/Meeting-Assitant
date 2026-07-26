#!/usr/bin/env bash
# Read-only bundle and process-footprint snapshot for installed macOS applications.
set -euo pipefail

usage() {
  echo "usage: $0 /path/to/App.app [/path/to/Another.app ...]" >&2
}

if [[ "$#" -eq 0 ]]; then
  usage
  exit 2
fi

for required_command in du file lipo plutil ps; do
  command -v "$required_command" >/dev/null || {
    echo "missing required command: $required_command" >&2
    exit 1
  }
done

plist_value() {
  local plist="$1"
  local key="$2"
  plutil -extract "$key" raw -o - "$plist" 2>/dev/null || printf 'unknown'
}

directory_kib() {
  local path="$1"
  if [[ -d "$path" ]]; then
    du -sk "$path" | awk '{print $1}'
  else
    printf '0'
  fi
}

mach_o_count() {
  local contents="$1"
  find "$contents" -type f -perm -111 -exec file {} + 2>/dev/null |
    awk -F: '/Mach-O/ { count += 1 } END { print count + 0 }'
}

process_snapshot() {
  local app_path="$1"
  ps -axo pid=,rss=,%cpu=,command= |
    awk -v app_path="$app_path" '
      {
        rss = $2
        cpu = $3
        $1 = $2 = $3 = ""
        sub(/^[[:space:]]+/, "")
        if (index($0, app_path) == 1) {
          count += 1
          rss_total += rss
          cpu_total += cpu
        }
      }
      END {
        printf "%d\t%d\t%.1f", count + 0, rss_total + 0, cpu_total + 0
      }
    '
}

printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
  "name" \
  "version" \
  "bundle_id" \
  "architectures" \
  "bundle_kib" \
  "frameworks_kib" \
  "resources_kib" \
  "mach_o_count" \
  "process_count" \
  "rss_kib_snapshot" \
  "cpu_percent_snapshot" \
  "path"

for app_path in "$@"; do
  [[ -d "$app_path" ]] || {
    echo "app bundle not found: $app_path" >&2
    exit 1
  }

  info_plist="$app_path/Contents/Info.plist"
  [[ -f "$info_plist" ]] || {
    echo "Info.plist not found: $info_plist" >&2
    exit 1
  }

  name="$(plist_value "$info_plist" CFBundleDisplayName)"
  if [[ "$name" == "unknown" ]]; then
    name="$(plist_value "$info_plist" CFBundleName)"
  fi
  version="$(plist_value "$info_plist" CFBundleShortVersionString)"
  bundle_id="$(plist_value "$info_plist" CFBundleIdentifier)"
  executable_name="$(plist_value "$info_plist" CFBundleExecutable)"
  executable_path="$app_path/Contents/MacOS/$executable_name"
  architectures="unknown"
  if [[ -f "$executable_path" ]]; then
    architectures="$(lipo -archs "$executable_path" 2>/dev/null || printf 'unknown')"
  fi

  bundle_kib="$(directory_kib "$app_path")"
  frameworks_kib="$(directory_kib "$app_path/Contents/Frameworks")"
  resources_kib="$(directory_kib "$app_path/Contents/Resources")"
  executable_count="$(mach_o_count "$app_path/Contents")"
  snapshot="$(process_snapshot "$app_path")"
  IFS=$'\t' read -r process_count rss_kib cpu_percent <<<"$snapshot"

  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$name" \
    "$version" \
    "$bundle_id" \
    "$architectures" \
    "$bundle_kib" \
    "$frameworks_kib" \
    "$resources_kib" \
    "$executable_count" \
    "$process_count" \
    "$rss_kib" \
    "$cpu_percent" \
    "$app_path"
done
