#!/usr/bin/env bash
set -euo pipefail

# Installs a provider-neutral loopback tunnel to a private transcription worker.

usage() {
  echo "usage: $0 --host HOST --identity /absolute/key [--user USER] [--local-port PORT]" >&2
  exit 2
}

host=""
identity=""
remote_user="myassistant-stt"
local_port="18091"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --host) host="${2:-}"; shift 2 ;;
    --identity) identity="${2:-}"; shift 2 ;;
    --user) remote_user="${2:-}"; shift 2 ;;
    --local-port) local_port="${2:-}"; shift 2 ;;
    *) usage ;;
  esac
done

[[ "$host" =~ ^[A-Za-z0-9.-]+$ ]] || usage
[[ "$remote_user" =~ ^[a-z_][a-z0-9_-]*$ ]] || usage
[[ "$local_port" =~ ^[0-9]+$ ]] || usage
(( local_port > 0 && local_port < 65536 )) || usage
[[ "$identity" = /* && -f "$identity" ]] || {
  echo "identity must be an existing absolute file" >&2
  exit 2
}

label="com.myassistant.stt-tunnel"
agents_dir="$HOME/Library/LaunchAgents"
logs_dir="$HOME/Library/Logs/My Assistant"
plist="$agents_dir/$label.plist"
tmp="$(mktemp -t my-assistant-stt-tunnel.XXXXXX)"
trap 'rm -f "$tmp"' EXIT
mkdir -p "$agents_dir" "$logs_dir"

plutil -create xml1 "$tmp"
plutil -insert Label -string "$label" "$tmp"
plutil -insert ProgramArguments -json "[\"/usr/bin/ssh\",\"-N\",\"-o\",\"BatchMode=yes\",\"-o\",\"ExitOnForwardFailure=yes\",\"-o\",\"ServerAliveInterval=30\",\"-o\",\"ServerAliveCountMax=3\",\"-o\",\"StrictHostKeyChecking=yes\",\"-i\",\"$identity\",\"-L\",\"127.0.0.1:$local_port:127.0.0.1:18091\",\"$remote_user@$host\"]" "$tmp"
plutil -insert RunAtLoad -bool true "$tmp"
plutil -insert KeepAlive -json '{"NetworkState":true,"SuccessfulExit":false}' "$tmp"
plutil -insert ProcessType -string Background "$tmp"
plutil -insert StandardOutPath -string "$logs_dir/stt-tunnel.log" "$tmp"
plutil -insert StandardErrorPath -string "$logs_dir/stt-tunnel.log" "$tmp"
plutil -lint "$tmp" >/dev/null
install -m 600 "$tmp" "$plist"

domain="gui/$UID"
launchctl bootout "$domain" "$plist" >/dev/null 2>&1 || true
launchctl bootstrap "$domain" "$plist"
launchctl kickstart -k "$domain/$label"

for _ in {1..20}; do
  if curl -fsS --connect-timeout 1 "http://127.0.0.1:$local_port/livez" >/dev/null; then
    echo "Private transcription tunnel ready on 127.0.0.1:$local_port"
    exit 0
  fi
  sleep 0.25
done

echo "tunnel was installed but the private transcription service is not reachable" >&2
echo "inspect: $logs_dir/stt-tunnel.log" >&2
exit 1
