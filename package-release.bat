@echo off
REM Build and Package HWiNFO-SteelSeries Release
REM This script calls the PowerShell packaging script

echo HWiNFO-SteelSeries Release Packager
echo ===================================
echo.

powershell -ExecutionPolicy Bypass -File package-release.ps1 %*
