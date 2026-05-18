param(
    [string]$Binary = "target\\release\\noctrail-app.exe",
    [int]$Samples = 16,
    [int]$BurstCount = 1,
    [double]$P95ThresholdMs = 60,
    [switch]$BuildRelease
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = (Get-Location).Path
$measureScript = Join-Path $repoRoot "scripts\\windows-measure-present-latency.ps1"

if ($BuildRelease) {
    cargo build -p noctrail-app --release
}

$output = & $measureScript -Binary $Binary -Samples $Samples -BurstCount $BurstCount

$values = @{}
foreach ($line in $output) {
    if ($line -match '^(?<key>[^=]+)=(?<value>.*)$') {
        $values[$matches['key']] = $matches['value']
    }
}

$p95 = if ($values.ContainsKey("p95_ms")) { [double]$values["p95_ms"] } else { $null }
$summary = $values["summary"]
$probe = $values["probe"]
$screenshot = $values["screenshot"]

if ($null -eq $p95) {
    throw "missing p95_ms in measurement output"
}

Write-Output "latency-gate.threshold_ms=$P95ThresholdMs"
Write-Output "latency-gate.p95_ms=$p95"
Write-Output "latency-gate.summary=$summary"
Write-Output "latency-gate.probe=$probe"
Write-Output "latency-gate.screenshot=$screenshot"

if ($p95 -gt $P95ThresholdMs) {
    throw "present latency gate failed: p95 ${p95}ms exceeded threshold ${P95ThresholdMs}ms"
}

Write-Output "latency-gate.status=pass"
