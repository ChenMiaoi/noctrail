param(
    [Parameter(Mandatory = $true)]
    [string]$Executable,
    [string[]]$Arguments = @(),
    [string]$WorkingDirectory = (Get-Location).Path,
    [string]$Label = "app",
    [int]$Samples = 12,
    [int]$BottomRoiHeight = 1200,
    [int]$Inset = 12,
    [int]$PollIntervalMs = 8,
    [int]$TimeoutMs = 2000,
    [int]$SettlingMs = 2500,
    [int]$BetweenSamplesMs = 180,
    [int]$PixelChannelDeltaThreshold = 24,
    [int]$MinimumChangedPixels = 18,
    [int]$BurstCount = 1,
    [string]$BurstCharacter = "x",
    [int]$BurstInterKeyDelayMs = 8,
    [int]$BurstSettleMs = 70,
    [string]$OutputRoot = "artifacts\\input-latency"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type -ReferencedAssemblies @("System.Drawing.dll") @"
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;

public static class NoctrailLatencyWin32 {
    const uint KEYEVENTF_KEYUP = 0x0002;

    [StructLayout(LayoutKind.Sequential)]
    public struct RECT {
        public int Left;
        public int Top;
        public int Right;
        public int Bottom;
    }

    public class CaptureFrame {
        public int Width;
        public int Height;
        public byte[] Pixels;
    }

    [DllImport("user32.dll")]
    public static extern bool SetForegroundWindow(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);

    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);

    [DllImport("user32.dll", SetLastError = true)]
    static extern void keybd_event(byte bVk, byte bScan, uint dwFlags, UIntPtr dwExtraInfo);

    public static CaptureFrame CaptureBottomRoi(IntPtr hWnd, int inset, int bottomHeight) {
        RECT rect;
        if (!GetWindowRect(hWnd, out rect)) {
            throw new InvalidOperationException("GetWindowRect failed");
        }

        int width = Math.Max(1, rect.Right - rect.Left - (inset * 2));
        int height = Math.Max(1, Math.Min(bottomHeight, rect.Bottom - rect.Top));
        int x = rect.Left + inset;
        int y = Math.Max(rect.Top, rect.Bottom - bottomHeight - inset);
        height = Math.Max(1, rect.Bottom - y - inset);

        using (var bitmap = new Bitmap(width, height, PixelFormat.Format32bppArgb))
        using (var graphics = Graphics.FromImage(bitmap)) {
            graphics.CopyFromScreen(x, y, 0, 0, new Size(width, height));
            var frame = new CaptureFrame();
            frame.Width = width;
            frame.Height = height;
            frame.Pixels = ExtractPixels(bitmap);
            return frame;
        }
    }

    public static void SaveWindowPng(IntPtr hWnd, string path) {
        RECT rect;
        if (!GetWindowRect(hWnd, out rect)) {
            throw new InvalidOperationException("GetWindowRect failed");
        }

        int width = Math.Max(1, rect.Right - rect.Left);
        int height = Math.Max(1, rect.Bottom - rect.Top);
        using (var bitmap = new Bitmap(width, height, PixelFormat.Format32bppArgb))
        using (var graphics = Graphics.FromImage(bitmap)) {
            graphics.CopyFromScreen(rect.Left, rect.Top, 0, 0, new Size(width, height));
            bitmap.Save(path, ImageFormat.Png);
        }
    }

    public static int CountChangedPixels(CaptureFrame baseline, CaptureFrame current, int channelThreshold) {
        if (baseline.Width != current.Width || baseline.Height != current.Height) {
            throw new InvalidOperationException("capture frame sizes do not match");
        }

        int changed = 0;
        for (int i = 0; i < baseline.Pixels.Length; i += 4) {
            int delta =
                Math.Abs(baseline.Pixels[i] - current.Pixels[i]) +
                Math.Abs(baseline.Pixels[i + 1] - current.Pixels[i + 1]) +
                Math.Abs(baseline.Pixels[i + 2] - current.Pixels[i + 2]);
            if (delta >= channelThreshold) {
                changed++;
            }
        }
        return changed;
    }

    public static void TapVirtualKey(ushort virtualKey) {
        keybd_event((byte)virtualKey, 0, 0, UIntPtr.Zero);
        keybd_event((byte)virtualKey, 0, KEYEVENTF_KEYUP, UIntPtr.Zero);
    }

    static byte[] ExtractPixels(Bitmap bitmap) {
        var rect = new Rectangle(0, 0, bitmap.Width, bitmap.Height);
        var data = bitmap.LockBits(rect, ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
        try {
            int bytes = Math.Abs(data.Stride) * bitmap.Height;
            var pixels = new byte[bytes];
            Marshal.Copy(data.Scan0, pixels, 0, bytes);
            return pixels;
        }
        finally {
            bitmap.UnlockBits(data);
        }
    }
}
"@

function Send-KeyTap {
    param([string]$Keys)
    switch ($Keys) {
        "x" {
            [NoctrailLatencyWin32]::TapVirtualKey(0x58)
        }
        "{BACKSPACE}" {
            [NoctrailLatencyWin32]::TapVirtualKey(0x08)
        }
        default {
            [System.Windows.Forms.SendKeys]::SendWait($Keys)
        }
    }
}

function Send-KeyBurst {
    param(
        [string]$Character,
        [int]$Count,
        [int]$InterKeyDelayMs
    )

    for ($index = 0; $index -lt $Count; $index++) {
        Send-KeyTap $Character
        if ($InterKeyDelayMs -gt 0 -and $index -lt ($Count - 1)) {
            Start-Sleep -Milliseconds $InterKeyDelayMs
        }
    }
}

function Wait-MainWindow {
    param(
        [System.Diagnostics.Process]$Process,
        [int]$TimeoutMs
    )

    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    while ([DateTime]::UtcNow -lt $deadline) {
        $Process.Refresh()
        if ($null -ne $Process.MainWindowHandle -and $Process.MainWindowHandle -ne 0) {
            $rect = New-Object NoctrailLatencyWin32+RECT
            if ([NoctrailLatencyWin32]::GetWindowRect($Process.MainWindowHandle, [ref]$rect)) {
                $width = $rect.Right - $rect.Left
                $height = $rect.Bottom - $rect.Top
                if ($width -ge 640 -and $height -ge 400) {
                    return $Process.MainWindowHandle
                }
            }
        }
        Start-Sleep -Milliseconds 100
    }

    throw "timed out waiting for the main window"
}

function Measure-KeyEchoLatency {
    param(
        $Handle
    )

    $sampleResults = New-Object System.Collections.Generic.List[object]
    $windowHandle = [System.IntPtr]$Handle
    $sampleCount = [int]$Samples
    $settlingDelayMs = [int]$SettlingMs
    $betweenDelayMs = [int]$BetweenSamplesMs
    $roiInset = [int]$Inset
    $roiHeight = [int]$BottomRoiHeight
    $pollDelayMs = [int]$PollIntervalMs
    $timeoutBudgetMs = [int]$TimeoutMs
    $pixelThreshold = [int]$PixelChannelDeltaThreshold
    $minimumChangedPixels = [int]$MinimumChangedPixels

    Start-Sleep -Milliseconds $settlingDelayMs

    for ($index = 0; $index -lt $sampleCount; $index++) {
        Start-Sleep -Milliseconds $betweenDelayMs

        $baseline = [NoctrailLatencyWin32]::CaptureBottomRoi($windowHandle, $roiInset, $roiHeight)
        Start-Sleep -Milliseconds 30
        $idle = [NoctrailLatencyWin32]::CaptureBottomRoi($windowHandle, $roiInset, $roiHeight)
        $idleDiff = [NoctrailLatencyWin32]::CountChangedPixels(
            $baseline,
            $idle,
            $pixelThreshold
        )
        $threshold = [Math]::Max($minimumChangedPixels, $idleDiff + 12)

        $stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
        Send-KeyTap "x"

        $latencyMs = $null
        $changedPixels = 0
        $maxChangedPixels = 0
        while ($stopwatch.ElapsedMilliseconds -lt $timeoutBudgetMs) {
            Start-Sleep -Milliseconds $pollDelayMs
            $current = [NoctrailLatencyWin32]::CaptureBottomRoi($windowHandle, $roiInset, $roiHeight)
            $changedPixels = [NoctrailLatencyWin32]::CountChangedPixels(
                $idle,
                $current,
                $pixelThreshold
            )
            $maxChangedPixels = [Math]::Max($maxChangedPixels, $changedPixels)
            if ($changedPixels -ge $threshold) {
                $latencyMs = [Math]::Round($stopwatch.Elapsed.TotalMilliseconds, 3)
                break
            }
        }

        Send-KeyTap "{BACKSPACE}"

        $sampleResults.Add([pscustomobject]@{
            sample = $index + 1
            idle_diff_pixels = $idleDiff
            threshold_pixels = $threshold
            changed_pixels = $changedPixels
            max_changed_pixels = $maxChangedPixels
            latency_ms = $latencyMs
            timeout = ($null -eq $latencyMs)
        }) | Out-Null
    }

    return $sampleResults
}

function Measure-BurstEchoLatency {
    param(
        $Handle
    )

    $sampleResults = New-Object System.Collections.Generic.List[object]
    $windowHandle = [System.IntPtr]$Handle
    $sampleCount = [int]$Samples
    $settlingDelayMs = [int]$SettlingMs
    $betweenDelayMs = [int]$BetweenSamplesMs
    $roiInset = [int]$Inset
    $roiHeight = [int]$BottomRoiHeight
    $pollDelayMs = [int]$PollIntervalMs
    $timeoutBudgetMs = [int]$TimeoutMs
    $pixelThreshold = [int]$PixelChannelDeltaThreshold
    $minimumChangedPixels = [int]$MinimumChangedPixels
    $burstCount = [int]$BurstCount
    $burstCharacter = [string]$BurstCharacter
    $burstInterKeyDelayMs = [int]$BurstInterKeyDelayMs
    $burstSettleMs = [double]$BurstSettleMs

    Start-Sleep -Milliseconds $settlingDelayMs

    for ($index = 0; $index -lt $sampleCount; $index++) {
        Start-Sleep -Milliseconds $betweenDelayMs

        $baseline = [NoctrailLatencyWin32]::CaptureBottomRoi($windowHandle, $roiInset, $roiHeight)
        Start-Sleep -Milliseconds 30
        $idle = [NoctrailLatencyWin32]::CaptureBottomRoi($windowHandle, $roiInset, $roiHeight)
        $idleDiff = [NoctrailLatencyWin32]::CountChangedPixels(
            $baseline,
            $idle,
            $pixelThreshold
        )
        $threshold = [Math]::Max($minimumChangedPixels, $idleDiff + 12)

        $stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
        Send-KeyBurst -Character $burstCharacter -Count $burstCount -InterKeyDelayMs $burstInterKeyDelayMs

        $latencyMs = $null
        $changedPixels = 0
        $maxChangedPixels = 0
        $lastChangedPixels = -1
        $lastVisualChangeMs = $null
        while ($stopwatch.ElapsedMilliseconds -lt $timeoutBudgetMs) {
            Start-Sleep -Milliseconds $pollDelayMs
            $current = [NoctrailLatencyWin32]::CaptureBottomRoi($windowHandle, $roiInset, $roiHeight)
            $changedPixels = [NoctrailLatencyWin32]::CountChangedPixels(
                $idle,
                $current,
                $pixelThreshold
            )
            $maxChangedPixels = [Math]::Max($maxChangedPixels, $changedPixels)
            if ($changedPixels -ne $lastChangedPixels) {
                $lastChangedPixels = $changedPixels
                if ($changedPixels -ge $threshold) {
                    $lastVisualChangeMs = [Math]::Round($stopwatch.Elapsed.TotalMilliseconds, 3)
                }
            }
            if ($null -ne $lastVisualChangeMs -and
                ($stopwatch.Elapsed.TotalMilliseconds - $lastVisualChangeMs) -ge $burstSettleMs) {
                $latencyMs = $lastVisualChangeMs
                break
            }
        }

        Send-KeyBurst -Character "{BACKSPACE}" -Count $burstCount -InterKeyDelayMs 4

        $sampleResults.Add([pscustomobject]@{
            sample = $index + 1
            burst_count = $burstCount
            idle_diff_pixels = $idleDiff
            threshold_pixels = $threshold
            changed_pixels = $changedPixels
            max_changed_pixels = $maxChangedPixels
            latency_ms = $latencyMs
            chars_per_second = if ($null -ne $latencyMs -and $latencyMs -gt 0) {
                [Math]::Round(($burstCount * 1000.0) / $latencyMs, 3)
            } else {
                $null
            }
            timeout = ($null -eq $latencyMs)
        }) | Out-Null
    }

    return $sampleResults
}

$resolvedExecutable = (Resolve-Path $Executable).Path
$verificationId = Get-Date -Format "yyyyMMdd-HHmmss"
$labelSlug = ($Label -replace '[^A-Za-z0-9._-]', '-').ToLowerInvariant()
$outputDir = Join-Path (Join-Path (Get-Location) $OutputRoot) "$verificationId-$labelSlug"
New-Item -ItemType Directory -Force $outputDir | Out-Null

$startProcessParams = @{
    FilePath = $resolvedExecutable
    WorkingDirectory = $WorkingDirectory
    PassThru = $true
}
if ($Arguments.Count -gt 0) {
    $startProcessParams.ArgumentList = $Arguments
}
$process = Start-Process @startProcessParams

try {
    $handle = @(Wait-MainWindow -Process $process -TimeoutMs 45000)[0]
    [NoctrailLatencyWin32]::ShowWindow($handle, 5) | Out-Null
    [NoctrailLatencyWin32]::SetForegroundWindow($handle) | Out-Null
    Start-Sleep -Milliseconds 400

    $sampleMeasurements = if ($BurstCount -gt 1) {
        Measure-BurstEchoLatency -Handle $handle
    } else {
        Measure-KeyEchoLatency -Handle $handle
    }
    $successful = $sampleMeasurements | Where-Object { -not $_.timeout }
    $latencies = @($successful | ForEach-Object { [double]$_.latency_ms } | Sort-Object)

    function Percentile([double[]]$values, [double]$ratio) {
        if ($values.Length -eq 0) {
            return $null
        }
        $index = [Math]::Ceiling($values.Length * $ratio) - 1
        $index = [Math]::Min([Math]::Max($index, 0), $values.Length - 1)
        return [Math]::Round($values[$index], 3)
    }

    $screenshotPath = Join-Path $outputDir "window.png"
    [NoctrailLatencyWin32]::SaveWindowPng($handle, $screenshotPath)

    $summary = [pscustomobject]@{
        label = $Label
        executable = $resolvedExecutable
        arguments = $Arguments
        samples = $Samples
        burst_count = $BurstCount
        burst_character = $BurstCharacter
        successful_samples = $latencies.Count
        timed_out_samples = @($sampleMeasurements | Where-Object timeout).Count
        min_ms = if ($latencies.Count -gt 0) { [Math]::Round(($latencies | Measure-Object -Minimum).Minimum, 3) } else { $null }
        p50_ms = Percentile $latencies 0.5
        p95_ms = Percentile $latencies 0.95
        max_ms = if ($latencies.Count -gt 0) { [Math]::Round(($latencies | Measure-Object -Maximum).Maximum, 3) } else { $null }
        avg_ms = if ($latencies.Count -gt 0) { [Math]::Round(($latencies | Measure-Object -Average).Average, 3) } else { $null }
        screenshot = $screenshotPath
    }

    $summaryPath = Join-Path $outputDir "summary.json"
    $samplesPath = Join-Path $outputDir "samples.json"
    $summary | ConvertTo-Json -Depth 4 | Set-Content -Path $summaryPath -Encoding UTF8
    $sampleMeasurements | ConvertTo-Json -Depth 4 | Set-Content -Path $samplesPath -Encoding UTF8

    Write-Output "label=$($summary.label)"
    Write-Output "p50_ms=$($summary.p50_ms)"
    Write-Output "p95_ms=$($summary.p95_ms)"
    Write-Output "avg_ms=$($summary.avg_ms)"
    Write-Output "timed_out=$($summary.timed_out_samples)"
    Write-Output "successful=$($summary.successful_samples)"
    Write-Output "summary=$summaryPath"
    Write-Output "samples=$samplesPath"
    Write-Output "screenshot=$screenshotPath"
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
