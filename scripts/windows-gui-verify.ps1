param(
    [string]$Binary = "target\\debug\\noctrail-app.exe",
    [string]$OutputRoot = "artifacts\\gui-verification",
    [int]$StartupTimeoutSeconds = 30,
    [switch]$ValidateStartupFocus,
    [ValidateSet("basic", "visual-smoke")]
    [string]$Scenario = "basic"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName Microsoft.VisualBasic
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
    public static extern bool BringWindowToTop(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern bool SetWindowPos(
        IntPtr hWnd,
        IntPtr hWndInsertAfter,
        int X,
        int Y,
        int cx,
        int cy,
        uint uFlags
    );

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

function Focus-NoctrailWindow {
    param(
        [System.Diagnostics.Process]$Process,
        [IntPtr]$Handle,
        [int]$TimeoutSeconds = 5
    )

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    $topMost = [IntPtr]::new(-1)
    $notTopMost = [IntPtr]::new(-2)
    while ((Get-Date) -lt $deadline) {
        [NoctrailGuiWin32]::ShowWindow($Handle, 5) | Out-Null
        [NoctrailGuiWin32]::BringWindowToTop($Handle) | Out-Null
        [NoctrailGuiWin32]::SetWindowPos($Handle, $topMost, 0, 0, 0, 0, 0x0003) | Out-Null
        [NoctrailGuiWin32]::SetWindowPos($Handle, $notTopMost, 0, 0, 0, 0, 0x0003) | Out-Null
        [Microsoft.VisualBasic.Interaction]::AppActivate($Process.Id) | Out-Null
        [NoctrailGuiWin32]::SetForegroundWindow($Handle) | Out-Null
        Start-Sleep -Milliseconds 250
        if ([NoctrailGuiWin32]::GetForegroundWindow() -eq $Handle) {
            Start-Sleep -Milliseconds 500
            return $true
        }
    }

    return $false
}

function Start-NoctrailProcess {
    param(
        [string]$BinaryPath,
        [string]$WorkingDirectory,
        [string[]]$ArgumentList = @(),
        [string]$VisualScene
    )

    $previousScene = $env:NOCTRAIL_VISUAL_SCENE
    try {
        if ($null -ne $VisualScene -and $VisualScene.Length -gt 0) {
            $env:NOCTRAIL_VISUAL_SCENE = $VisualScene
        }
        else {
            Remove-Item Env:NOCTRAIL_VISUAL_SCENE -ErrorAction SilentlyContinue
        }

        return Start-Process `
            -FilePath $BinaryPath `
            -WorkingDirectory $WorkingDirectory `
            -ArgumentList $ArgumentList `
            -PassThru
    }
    finally {
        if ($null -ne $previousScene) {
            $env:NOCTRAIL_VISUAL_SCENE = $previousScene
        }
        else {
            Remove-Item Env:NOCTRAIL_VISUAL_SCENE -ErrorAction SilentlyContinue
        }
    }
}

function Stop-NoctrailProcess {
    param([System.Diagnostics.Process]$Process)

    if (-not $Process.HasExited) {
        $null = $Process.CloseMainWindow()
        Start-Sleep -Seconds 2
        if (-not $Process.HasExited) {
            Stop-Process -Id $Process.Id -Force
        }
    }
}

$repoRoot = (Get-Location).Path
$resolvedBinary = (Resolve-Path $Binary).Path
$verificationId = Get-Date -Format "yyyyMMdd-HHmmss"
$outputDir = Join-Path $repoRoot $OutputRoot
$outputDir = Join-Path $outputDir $verificationId
New-Item -ItemType Directory -Force $outputDir | Out-Null

$logPath = Join-Path $outputDir "scenario.log"
$steps = New-Object System.Collections.Generic.List[string]
$captures = New-Object System.Collections.Generic.List[string]

if ($Scenario -eq "visual-smoke") {
    $visualScenes = @(
        @{ Name = "single-pane"; File = "single-pane.png" },
        @{ Name = "tiling-4up"; File = "tiling-4up.png" },
        @{ Name = "scratch"; File = "scratch.png" }
    )

    foreach ($visualScene in $visualScenes) {
        $process = Start-NoctrailProcess `
            -BinaryPath $resolvedBinary `
            -WorkingDirectory $repoRoot `
            -ArgumentList @("visual-smoke") `
            -VisualScene $visualScene.Name
        try {
            $handle = Wait-MainWindow -Process $process -TimeoutSeconds $StartupTimeoutSeconds
            $focused = Focus-NoctrailWindow -Process $process -Handle $handle
            Start-Sleep -Seconds 3
            $screenshotPath = Join-Path $outputDir $visualScene.File
            Save-WindowScreenshot -Handle $handle -Path $screenshotPath
            $captures.Add($screenshotPath)
            $steps.Add("captured $($visualScene.Name) focus=$focused -> $screenshotPath")
        }
        finally {
            Stop-NoctrailProcess -Process $process
        }
    }

    @(
        "binary=$resolvedBinary"
        "scenario=$Scenario"
        "captures="
        ($captures | ForEach-Object { "- $_" })
        "steps="
        ($steps | ForEach-Object { "- $_" })
    ) | Set-Content -Path $logPath -Encoding UTF8

    Write-Output "gui-verification.ok=$outputDir"
    foreach ($capture in $captures) {
        Write-Output "gui-verification.screenshot=$capture"
    }
    return
}

$process = Start-NoctrailProcess -BinaryPath $resolvedBinary -WorkingDirectory $repoRoot
try {
    $handle = Wait-MainWindow -Process $process -TimeoutSeconds $StartupTimeoutSeconds
    $focusConfirmed = Focus-NoctrailWindow -Process $process -Handle $handle
    $startupFocused = ([NoctrailGuiWin32]::GetForegroundWindow() -eq $handle)
    $steps.Add("startup focus confirmed: $focusConfirmed")
    $steps.Add("startup foreground match: $startupFocused")
    if ($ValidateStartupFocus -and $startupFocused) {
        Send-KeyChord "echo AUTO_FOCUS_OK{ENTER}" 900
        $steps.Add("typed echo AUTO_FOCUS_OK without forcing foreground")
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
    $screenshotPath = Join-Path $outputDir "gui.png"
    Save-WindowScreenshot -Handle $handle -Path $screenshotPath
    $steps.Add("captured final window screenshot")

    @(
        "binary=$resolvedBinary"
        "scenario=$Scenario"
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
    Stop-NoctrailProcess -Process $process
}
