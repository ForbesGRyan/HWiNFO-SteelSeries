# Build and Package HWiNFO-SteelSeries Release
# This script builds the release version and creates a zip package

param(
    [switch]$SkipBuild = $false
)

$ErrorActionPreference = "Stop"

Write-Host "HWiNFO-SteelSeries Release Packager" -ForegroundColor Cyan
Write-Host "===================================" -ForegroundColor Cyan
Write-Host ""

# Get version from Cargo.toml
$cargoToml = Get-Content "Cargo.toml" -Raw
if ($cargoToml -match 'version\s*=\s*"([^"]+)"') {
    $version = $matches[1]
} else {
    $version = "unknown"
}

Write-Host "Version: $version" -ForegroundColor Green

# Build release version
if (-not $SkipBuild) {
    Write-Host ""
    Write-Host "Building release version..." -ForegroundColor Yellow
    cargo build --release
    if ($LASTEXITCODE -ne 0) {
        Write-Host "Build failed!" -ForegroundColor Red
        exit 1
    }
    Write-Host "Build completed successfully!" -ForegroundColor Green
}

# Prepare release directory
$releaseName = "hwinfo-steelseries-oled-v$version-windows"
$releaseDir = "target\release\package"
$packageDir = "$releaseDir\$releaseName"

Write-Host ""
Write-Host "Preparing release package..." -ForegroundColor Yellow

# Clean up old package directory
if (Test-Path $releaseDir) {
    Remove-Item -Path $releaseDir -Recurse -Force
}
New-Item -ItemType Directory -Path $packageDir -Force | Out-Null

# Copy executable
Write-Host "  Copying executable..." -ForegroundColor Gray
Copy-Item "target\release\hwinfo-steelseries-oled.exe" -Destination $packageDir

# Note: Assets (icon) are now embedded in the executable, no need to copy

# Copy README and other docs if they exist
if (Test-Path "README.md") {
    Write-Host "  Copying README.md..." -ForegroundColor Gray
    Copy-Item "README.md" -Destination $packageDir
}

if (Test-Path "LICENSE") {
    Write-Host "  Copying LICENSE..." -ForegroundColor Gray
    Copy-Item "LICENSE" -Destination $packageDir
}

# Create example config
Write-Host "  Creating example config..." -ForegroundColor Gray
$exampleConfig = @"
# HWiNFO-SteelSeries OLED Configuration Example
# Delete or rename this file and run the application to create your own config

[Main]
style=Vertical
decimal=true
pages=1
page_time=5

# The application will prompt you to configure sensors on first run
"@
Set-Content -Path "$packageDir\conf.ini.example" -Value $exampleConfig

# Create zip file
$zipPath = "$releaseDir\$releaseName.zip"
Write-Host ""
Write-Host "Creating zip archive..." -ForegroundColor Yellow

# Remove old zip if it exists
if (Test-Path $zipPath) {
    Remove-Item -Path $zipPath -Force
}

# Create zip
Compress-Archive -Path $packageDir -DestinationPath $zipPath -CompressionLevel Optimal

Write-Host ""
Write-Host "Package created successfully!" -ForegroundColor Green
Write-Host "Location: $zipPath" -ForegroundColor Cyan
Write-Host ""
Write-Host "Contents:" -ForegroundColor Yellow
Get-ChildItem -Path $packageDir -Recurse | ForEach-Object {
    $relativePath = $_.FullName.Substring($packageDir.Length + 1)
    Write-Host "  - $relativePath" -ForegroundColor Gray
}

# Show zip file size
$zipSize = (Get-Item $zipPath).Length / 1MB
Write-Host ""
Write-Host "Zip file size: $([math]::Round($zipSize, 2)) MB" -ForegroundColor Cyan
