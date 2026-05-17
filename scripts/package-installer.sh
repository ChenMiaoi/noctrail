#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
app_manifest="$root_dir/crates/noctrail-app/Cargo.toml"

find_latest_file() {
  local start="$1"
  local name="$2"
  find "$start" -type f -name "$name" -print | sort | tail -n 1
}

find_latest_dir() {
  local start="$1"
  local name="$2"
  find "$start" -type d -name "$name" -print | sort | tail -n 1
}

case "$(uname -s)" in
  Darwin)
    CI=true cargo packager --manifest-path "$app_manifest" --release --formats app --formats dmg
    export NOCTRAIL_INSTALLER_APP
    export NOCTRAIL_INSTALLER_DMG
    NOCTRAIL_INSTALLER_APP="$(find_latest_dir "$root_dir/target/packager" 'Noctrail.app')"
    NOCTRAIL_INSTALLER_DMG="$(find_latest_file "$root_dir/target/packager" '*.dmg')"
    ;;
  Linux)
    CI=true cargo packager --manifest-path "$app_manifest" --release --formats appimage --formats deb
    cargo generate-rpm -p noctrail-app --target-dir "$root_dir/target"
    export NOCTRAIL_INSTALLER_APPIMAGE
    export NOCTRAIL_INSTALLER_DEB
    export NOCTRAIL_INSTALLER_RPM
    NOCTRAIL_INSTALLER_APPIMAGE="$(find_latest_file "$root_dir/target/packager" '*.AppImage')"
    NOCTRAIL_INSTALLER_DEB="$(find_latest_file "$root_dir/target/packager" '*.deb')"
    NOCTRAIL_INSTALLER_RPM="$(find_latest_file "$root_dir/target/generate-rpm" '*.rpm')"
    ;;
  *)
    echo "unsupported platform for package-installer.sh: $(uname -s)" >&2
    exit 2
    ;;
esac

cargo run -p noctrail-cli -- installer-smoke
