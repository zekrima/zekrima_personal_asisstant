<#
.SYNOPSIS
    One-command Windows installer for Zekrima Personal Assistant.
.DESCRIPTION
    Downloads the latest release `zekrima.exe` from GitHub Releases (or uses a local binary
    next to this script), installs it to `%LOCALAPPDATA%\Programs\Zekrima`, creates a Start Menu
    shortcut, and optionally registers Windows Startup.
#>
param(
    [switch]$Startup,
    [switch]$Uninstall
)

$ErrorActionPreference = "Stop"
$InstallDir = Join-Path $env:LOCALAPPDATA "Programs\Zekrima"
$ExePath = Join-Path $InstallDir "zekrima.exe"
$StartMenuShortcut = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\Zekrima Personal Assistant.lnk"
$RunKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"

if ($Uninstall) {
    Write-Host "==> Stopping and removing Zekrima Personal Assistant..." -ForegroundColor Yellow
    Stop-Process -Name zekrima -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 300
    Remove-Item -Force $StartMenuShortcut -ErrorAction SilentlyContinue
    Remove-ItemProperty -Path $RunKey -Name "Zekrima" -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $InstallDir -ErrorAction SilentlyContinue
    Write-Host "==> Zekrima removed cleanly." -ForegroundColor Green
    exit 0
}

Write-Host "==> Installing Zekrima Personal Assistant to $InstallDir ..." -ForegroundColor Cyan
Stop-Process -Name zekrima -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 300
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null

$LocalExe = Join-Path $PSScriptRoot "target\release\zekrima.exe"
$LocalBin = Join-Path $PSScriptRoot "bin\zekrima.exe"

if (Test-Path $LocalExe) {
    Copy-Item -Force $LocalExe $ExePath
} elseif (Test-Path $LocalBin) {
    Copy-Item -Force $LocalBin $ExePath
} else {
    $DownloadUrl = "https://github.com/zekrima/zekrima_personal_asisstant/releases/latest/download/zekrima.exe"
    Write-Host "==> Downloading latest zekrima.exe from GitHub Releases..."
    Invoke-WebRequest -Uri $DownloadUrl -OutFile $ExePath
}

$WshShell = New-Object -ComObject WScript.Shell
$Shortcut = $WshShell.CreateShortcut($StartMenuShortcut)
$Shortcut.TargetPath = $ExePath
$Shortcut.WorkingDirectory = $InstallDir
$Shortcut.Description = "Zekrima Personal Assistant - Native Dynamic Island"
$Shortcut.Save()

if ($Startup) {
    Set-ItemProperty -Path $RunKey -Name "Zekrima" -Value "`"$ExePath`""
    Write-Host "==> Registered Run at Windows Startup." -ForegroundColor Green
}

Write-Host "==> Done! Launching Zekrima Personal Assistant..." -ForegroundColor Green
Start-Process -FilePath $ExePath
