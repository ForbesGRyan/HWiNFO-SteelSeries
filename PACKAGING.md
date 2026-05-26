# Packaging Instructions

## Building a Release Package

The icon is now embedded in the executable at compile time, so you don't need to distribute the assets folder.

### Quick Start

Run one of these commands from the project root:

**Windows Batch:**
```cmd
package-release.bat
```

**PowerShell:**
```powershell
.\package-release.ps1
```

### What It Does

1. Builds the release version with optimizations
2. Creates a package directory with the executable
3. Copies README.md and LICENSE if they exist
4. Creates an example config file
5. Zips everything into `target\release\package\hwinfo-steelseries-oled-v{version}-windows.zip`

### Options

**Skip the build step** (if you already built):
```powershell
.\package-release.ps1 -SkipBuild
```

### Output

The final zip file will be created at:
```
target\release\package\hwinfo-steelseries-oled-v{version}-windows.zip
```

## Manual Build

If you just want to build without packaging:

```cmd
cargo build --release
```

The executable will be at: `target\release\hwinfo-steelseries-oled.exe`

**Note:** The icon is embedded at compile time, so the executable is completely standalone.
