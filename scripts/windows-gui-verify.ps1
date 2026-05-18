param(
    [string]$Binary = "target\\debug\\noctrail-app.exe",
    [string]$OutputRoot = "artifacts\\gui-verification",
    [int]$StartupTimeoutSeconds = 30,
    [switch]$ValidateStartupFocus
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;

public static class NoctrailGuiWin32 {
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
    public static extern IntPtr GetForegroundWindow();

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
            $rect = New-Object NoctrailGuiWin32+RECT
            if ([NoctrailGuiWin32]::GetWindowRect($Process.MainWindowHandle, [ref]$rect)) {
                $width = $rect.Right - $rect.Left
                $height = $rect.Bottom - $rect.Top
                if ($width -ge 640 -and $height -ge 400) {
                    return $Process.MainWindowHandle
                }
            }
        }
        Start-Sleep -Milliseconds 250
    }

    throw "timed out waiting for a Noctrail window"
}

function Send-KeyChord {
    param(
        [string]$Keys,
        [int]$PauseMilliseconds = 500
    )

    [System.Windows.Forms.SendKeys]::SendWait($Keys)
    Start-Sleep -Milliseconds $PauseMilliseconds
}

function Save-WindowScreenshot {
    param(
        [IntPtr]$Handle,
        [string]$Path
    )

    $rect = New-Object NoctrailGuiWin32+RECT
    $resolved = $false
    for ($attempt = 0; $attempt -lt 10; $attempt++) {
        if ([NoctrailGuiWin32]::GetWindowRect($Handle, [ref]$rect)) {
            $resolved = $true
            break
        }
        Start-Sleep -Milliseconds 200
    }
    if (-not $resolved) {
        throw "failed to resolve window bounds"
    }

    $width = [Math]::Max(1, $rect.Right - $rect.Left)
    $height = [Math]::Max(1, $rect.Bottom - $rect.Top)
    $bitmap = New-Object System.Drawing.Bitmap $width, $height
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

$repoRoot = (Get-Location).Path
$resolvedBinary = (Resolve-Path $Binary).Path
$verificationId = Get-Date -Format "yyyyMMdd-HHmmss"
$outputDir = Join-Path $repoRoot $OutputRoot
$outputDir = Join-Path $outputDir $verificationId
New-Item -ItemType Directory -Force $outputDir | Out-Null

$process = Start-Process -FilePath $resolvedBinary -WorkingDirectory $repoRoot -PassThru
$logPath = Join-Path $outputDir "scenario.log"
$screenshotPath = Join-Path $outputDir "gui.png"
$steps = New-Object System.Collections.Generic.List[string]

try {
    $handle = Wait-MainWindow -Process $process -TimeoutSeconds $StartupTimeoutSeconds
    [NoctrailGuiWin32]::ShowWindow($handle, 5) | Out-Null
    $startupFocused = ([NoctrailGuiWin32]::GetForegroundWindow() -eq $handle)
    $steps.Add("startup foreground match: $startupFocused")
    if ($ValidateStartupFocus -and $startupFocused) {
        Send-KeyChord "echo AUTO_FOCUS_OK{ENTER}" 900
        $steps.Add("typed echo AUTO_FOCUS_OK without forcing foreground")
    }
    if (-not $startupFocused) {
        [NoctrailGuiWin32]::SetForegroundWindow($handle) | Out-Null
        $steps.Add("forced foreground after recording startup focus result")
    }
    Start-Sleep -Seconds 2

    Send-KeyChord "echo GUI_BASIC_OK{ENTER}" 900
    $steps.Add("typed echo GUI_BASIC_OK")

    [System.Windows.Forms.Clipboard]::SetText("echo PASTE_ONE && echo PASTE_TWO")
    Send-KeyChord "^+v" 250
    Send-KeyChord "{ENTER}" 1000
    $steps.Add("pasted command via Ctrl+Shift+V and executed it")

    Send-KeyChord "ping 127.0.0.1 -n 5{ENTER}" 1200
    Send-KeyChord "^c" 700
    Send-KeyChord "echo CTRL_C_OK{ENTER}" 900
    $steps.Add("started ping and interrupted it with Ctrl+C")

    Send-KeyChord "echo HISTORY_ONE{ENTER}" 700
    Send-KeyChord "echo HISTORY_TWO{ENTER}" 700
    Send-KeyChord "{UP}{ENTER}" 900
    $steps.Add("replayed the previous command through Up Arrow history")

    Start-Sleep -Seconds 1
    $process.Refresh()
    $handle = Wait-MainWindow -Process $process -TimeoutSeconds 5
    Save-WindowScreenshot -Handle $handle -Path $screenshotPath
    $steps.Add("captured final window screenshot")

    @(
        "binary=$resolvedBinary"
        "pid=$($process.Id)"
        "startup_focused=$startupFocused"
        "screenshot=$screenshotPath"
        "steps="
        ($steps | ForEach-Object { "- $_" })
    ) | Set-Content -Path $logPath -Encoding UTF8

    Write-Output "gui-verification.ok=$outputDir"
    Write-Output "gui-verification.startup-focused=$startupFocused"
    Write-Output "gui-verification.screenshot=$screenshotPath"
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
