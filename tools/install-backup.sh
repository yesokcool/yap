#!/bin/zsh
# Daily Yap backup on a Mac (04:00, or at next wake if asleep), into backups/
# here (or YAP_BACKUP_DIR). Put that folder somewhere synced for an off-site copy.
# Which server and admin key: tools/instance.mjs (the job reads YAP_URL if you
# set it when installing, else PUBLIC_URL from wrangler.jsonc, and the admin
# key from the Keychain).
#   tools/install-backup.sh            install / update
#   tools/install-backup.sh --uninstall
#   tools/install-backup.sh --run      run it right now (via launchd)
#   YAP_BACKUP_LABEL=… YAP_BACKUP_DIR=… YAP_URL=… tools/install-backup.sh   (your own job name / folder / server)
set -euo pipefail

LABEL="${YAP_BACKUP_LABEL:-local.yap.backup}"
KEYCHAIN_ITEM="${YAP_KEYCHAIN_ADMIN:-yap-admin-key}"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
LOG="$HOME/Library/Logs/yap-backup.log"
ROOT="${0:A:h:h}"
NODE="$(command -v node)"

if [[ "${1:-}" == "--uninstall" ]]; then
  launchctl bootout "gui/$UID/$LABEL" 2>/dev/null || true
  rm -f "$PLIST"
  echo "Removed $LABEL."
  exit 0
fi

if [[ "${1:-}" == "--run" ]]; then
  launchctl kickstart -k "gui/$UID/$LABEL"
  sleep 3
  tail -3 "$LOG"
  exit 0
fi

security find-generic-password -s "$KEYCHAIN_ITEM" -w >/dev/null 2>&1 || {
  echo "Missing Keychain item '$KEYCHAIN_ITEM' (the Worker's ADMIN_KEY):" >&2
  echo "  security add-generic-password -s $KEYCHAIN_ITEM -a yap -w" >&2
  exit 1
}

# The job runs at 04:00 on its own: make sure now that it will find the server and key.
"$NODE" "$ROOT/tools/backup.mjs" --check || {
  echo "Not installed: tell it which server (YAP_URL=https://… $0, or PUBLIC_URL in wrangler.jsonc)." >&2
  exit 1
}

mkdir -p "$HOME/Library/LaunchAgents" "$HOME/Library/Logs"
cat > "$PLIST" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$LABEL</string>
  <key>ProgramArguments</key>
  <array>
    <string>$NODE</string>
    <string>$ROOT/tools/backup.mjs</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>YAP_KEYCHAIN_ADMIN</key><string>$KEYCHAIN_ITEM</string>${YAP_BACKUP_DIR:+
    <key>YAP_BACKUP_DIR</key><string>$YAP_BACKUP_DIR</string>}${YAP_URL:+
    <key>YAP_URL</key><string>$YAP_URL</string>}
  </dict>
  <key>StartCalendarInterval</key>
  <dict><key>Hour</key><integer>4</integer><key>Minute</key><integer>0</integer></dict>
  <key>StandardOutPath</key><string>$LOG</string>
  <key>StandardErrorPath</key><string>$LOG</string>
  <key>ProcessType</key><string>Background</string>
</dict>
</plist>
PLIST

launchctl bootout "gui/$UID/$LABEL" 2>/dev/null || true
launchctl bootstrap "gui/$UID" "$PLIST"
echo "Installed $LABEL (daily 04:00). Log: $LOG"
