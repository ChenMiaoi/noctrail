param(
    [int]$Samples = 12
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = (Get-Location).Path
$measureScript = Join-Path $repoRoot "scripts\\windows-measure-input-latency.ps1"
$noctrailConfig = Join-Path $env:TEMP "noctrail-latency-config.toml"

@"
[theme.cursor]
blink-interval-ms = 10000
"@ | Set-Content -Path $noctrailConfig -Encoding UTF8

$targets = @(
    @{
        Label = "noctrail"
        Executable = Join-Path $repoRoot "target\\debug\\noctrail-app.exe"
        Arguments = @("--config", $noctrailConfig)
        WorkingDirectory = $repoRoot
    },
    @{
        Label = "warp"
        Executable = "D:\\Application\\Warp\\warp.exe"
        Arguments = @()
        WorkingDirectory = $repoRoot
    }
)

$results = New-Object System.Collections.Generic.List[object]

foreach ($target in $targets) {
    if (-not (Test-Path $target.Executable)) {
        $results.Add([pscustomobject]@{
            label = $target.Label
            status = "missing"
        }) | Out-Null
        continue
    }

    $params = @{
        Executable = $target.Executable
        Arguments = $target.Arguments
        WorkingDirectory = $target.WorkingDirectory
        Label = $target.Label
        Samples = $Samples
    }
    $output = & $measureScript @params

    $values = @{}
    foreach ($line in $output) {
        if ($line -match '^(?<key>[^=]+)=(?<value>.*)$') {
            $values[$matches['key']] = $matches['value']
        }
    }

    $results.Add([pscustomobject]@{
        label = $target.Label
        status = "ok"
        p50_ms = $values["p50_ms"]
        p95_ms = $values["p95_ms"]
        avg_ms = $values["avg_ms"]
        timed_out = $values["timed_out"]
        summary = $values["summary"]
        screenshot = $values["screenshot"]
    }) | Out-Null
}

$results | Format-Table -AutoSize
