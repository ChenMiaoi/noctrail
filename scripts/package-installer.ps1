$ErrorActionPreference = "Stop"

$RootDir = Split-Path -Parent $PSScriptRoot
$AppManifest = Join-Path $RootDir "crates/noctrail-app/Cargo.toml"

Set-Location $RootDir
$env:CI = "true"
cargo packager --manifest-path $AppManifest --release --formats wix

$msi = Get-ChildItem -Path (Join-Path $RootDir "target/packager") -Filter *.msi -Recurse |
  Sort-Object FullName |
  Select-Object -Last 1

if (-not $msi) {
  throw "missing .msi output under target/packager"
}

$env:NOCTRAIL_INSTALLER_MSI = $msi.FullName
cargo run -p noctrail-cli -- installer-smoke
