param(
    [string]$Binary = "target\\debug\\noctrail-app.exe",
    [string]$OutputRoot = "artifacts\\present-latency",
    [int]$Samples = 16,
    [int]$BurstCount = 1,
    [int]$StartupTimeoutSeconds = 30
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;

public static class NoctrailProbeWin32 {
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT {
        public int Left;
        public int Top;
        public int Right;
        public int Bottom;
    }

    [DllImport("user32.dll")]
    public static extern bool SetForegroundWindow(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);

    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
}
"@

function Wait-MainWindow {
    param(
        [System.Diagnostics.Process]$Process,
        [int]$TimeoutSeconds
    )

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        $Process.Refresh()
        if ($null -ne $Process.MainWindowHandle -and $Process.MainWindowHandle -ne 0) {
            $rect = New-Object NoctrailProbeWin32+RECT
            if ([NoctrailProbeWin32]::GetWindowRect($Process.MainWindowHandle, [ref]$rect)) {
                $width = $rect.Right - $rect.Left
                $height = $rect.Bottom - $rect.Top
                if ($width -ge 640 -and $height -ge 400) {
                    return $Process.MainWindowHandle
                }
            }
        }
        Start-Sleep -Milliseconds 100
    }

    throw "timed out waiting for main window"
}

function Save-WindowScreenshot {
    param(
        [IntPtr]$Handle,
        [string]$Path
    )

    $rect = New-Object NoctrailProbeWin32+RECT
    if (-not [NoctrailProbeWin32]::GetWindowRect($Handle, [ref]$rect)) {
        throw "failed to resolve window bounds"
    }

    $bitmap = New-Object System.Drawing.Bitmap ([Math]::Max(1, $rect.Right - $rect.Left)), ([Math]::Max(1, $rect.Bottom - $rect.Top))
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size)
        $bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    }
    finally {
        $graphics.Dispose()
        $bitmap.Dispose()
    }
}

function Send-KeyTap {
    param([string]$Keys)
    [System.Windows.Forms.SendKeys]::SendWait($Keys)
}

$repoRoot = (Get-Location).Path
$resolvedBinary = (Resolve-Path $Binary).Path
$verificationId = Get-Date -Format "yyyyMMdd-HHmmss"
$outputDir = Join-Path (Join-Path $repoRoot $OutputRoot) $verificationId
New-Item -ItemType Directory -Force $outputDir | Out-Null
$probePath = Join-Path $outputDir "probe.json"
$screenshotPath = Join-Path $outputDir "window.png"

$previousProbeEnv = [System.Environment]::GetEnvironmentVariable("NOCTRAIL_INPUT_LATENCY_LOG", "Process")
[System.Environment]::SetEnvironmentVariable("NOCTRAIL_INPUT_LATENCY_LOG", $probePath, "Process")
$process = Start-Process -FilePath $resolvedBinary -WorkingDirectory $repoRoot -PassThru
[System.Environment]::SetEnvironmentVariable("NOCTRAIL_INPUT_LATENCY_LOG", $previousProbeEnv, "Process")

try {
    $handle = Wait-MainWindow -Process $process -TimeoutSeconds $StartupTimeoutSeconds
    [NoctrailProbeWin32]::ShowWindow($handle, 5) | Out-Null
    [NoctrailProbeWin32]::SetForegroundWindow($handle) | Out-Null
    Start-Sleep -Milliseconds 1200

    for ($sample = 0; $sample -lt $Samples; $sample++) {
        for ($index = 0; $index -lt $BurstCount; $index++) {
            Send-KeyTap "x"
            Start-Sleep -Milliseconds 25
        }
        Start-Sleep -Milliseconds 90
        for ($index = 0; $index -lt $BurstCount; $index++) {
            Send-KeyTap "{BACKSPACE}"
            Start-Sleep -Milliseconds 20
        }
        Start-Sleep -Milliseconds 120
    }

    Save-WindowScreenshot -Handle $handle -Path $screenshotPath
}
finally {
    if (-not $process.HasExited) {
        $null = $process.CloseMainWindow()
        Start-Sleep -Seconds 2
        if (-not $process.HasExited) {
            Stop-Process -Id $process.Id -Force
        }
    }
}

$deadline = (Get-Date).AddSeconds(10)
while ((-not (Test-Path $probePath)) -and ((Get-Date) -lt $deadline)) {
    Start-Sleep -Milliseconds 100
}
if (-not (Test-Path $probePath)) {
    throw "probe output missing: $probePath"
}

$probe = Get-Content $probePath -Raw | ConvertFrom-Json
$insertSamples = @($probe.samples | Where-Object { $_.kind -eq "insert" -or $_.kind -eq "ime-commit" -or $_.kind -eq "paste" } | ForEach-Object { [double]$_.latency_ms } | Sort-Object)
if ($insertSamples.Count -eq 0) {
    throw "no insert samples were recorded"
}

function Percentile([double[]]$Values, [double]$Ratio) {
    if ($Values.Length -eq 0) {
        return $null
    }
    $index = [Math]::Ceiling(($Values.Length - 1) * $Ratio)
    $index = [Math]::Min([Math]::Max($index, 0), $Values.Length - 1)
    return [Math]::Round($Values[$index], 3)
}

$summary = [pscustomobject]@{
    samples = $insertSamples.Count
    burst_count = $BurstCount
    min_ms = [Math]::Round(($insertSamples | Measure-Object -Minimum).Minimum, 3)
    p50_ms = Percentile $insertSamples 0.5
    p95_ms = Percentile $insertSamples 0.95
    max_ms = [Math]::Round(($insertSamples | Measure-Object -Maximum).Maximum, 3)
    avg_ms = [Math]::Round(($insertSamples | Measure-Object -Average).Average, 3)
    probe = $probePath
    screenshot = $screenshotPath
}

$summaryPath = Join-Path $outputDir "summary.json"
$summary | ConvertTo-Json -Depth 4 | Set-Content -Path $summaryPath -Encoding UTF8

Write-Output "samples=$($summary.samples)"
Write-Output "p50_ms=$($summary.p50_ms)"
Write-Output "p95_ms=$($summary.p95_ms)"
Write-Output "avg_ms=$($summary.avg_ms)"
Write-Output "summary=$summaryPath"
Write-Output "probe=$probePath"
Write-Output "screenshot=$screenshotPath"
