@echo off
rem One-click Windows build of Solara: double-click this file.
rem Builds the release EXE, the MSI installer and the portable zip with
rem packaging\windows\package.ps1, then prints the version and full path of solara.exe.
rem Needs Rust (MSVC toolchain), the Windows SDK, WiX 5.0.2 and PowerShell 7 (pwsh);
rem see packaging\windows\INSTALLATION.md.
setlocal
cd /d "%~dp0"
where pwsh >nul 2>nul
if errorlevel 1 (
  echo PowerShell 7 ^(pwsh^) is needed: winget install Microsoft.PowerShell
  pause
  exit /b 1
)
pwsh -NoProfile -ExecutionPolicy Bypass -File "packaging\windows\package.ps1" -Arch x64
if errorlevel 1 (
  echo.
  echo The build failed. The messages above say why.
  pause
  exit /b 1
)
pwsh -NoProfile -Command "$t = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'target' }; $exe = Get-Item (Join-Path $t 'windows-package\x64\solara.exe'); Write-Output ''; Write-Output ('Solara ' + $exe.VersionInfo.ProductVersion); Write-Output ('EXE: ' + $exe.FullName); Write-Output ('Installer and portable zip: ' + (Resolve-Path 'dist\release'))"
echo.
pause
