# Release Scripts

This directory contains automated release scripts for the Chatspeed project.

## Scripts

### `release.sh` (macOS/Linux)

Bash script for Unix-like systems.

**Usage:**
```bash
# Interactive mode
./scripts/release.sh

# Direct version specification
./scripts/release.sh 1.0.0
./scripts/release.sh v1.0.1-beta.1
```

### `release.ps1` (Windows)

PowerShell script for Windows systems.

**Usage:**
```powershell
# Interactive mode
.\scripts\release.ps1

# Direct version specification
.\scripts\release.ps1 1.0.0
.\scripts\release.ps1 v1.0.1-beta.1
```

## What the Scripts Do

1. **Version Validation**: Validates the provided version follows semantic versioning (e.g., 1.0.0, 1.0.0-beta.1)
2. **File Updates**: Updates version numbers in:
   - `src-tauri/tauri.conf.json`
   - `src-tauri/Cargo.toml`
3. **Verification**: Ensures both files contain the correct version after update
4. **Git Operations**:
   - Creates a commit with message "chore: release version {version}"
   - Creates a Git tag "v{version}"
   - Pushes both commit and tag to remote repository
5. **CI/CD Trigger**: The pushed tag automatically triggers the GitHub Actions release workflow

## Safety Features

- **Git Status Check**: Warns if there are uncommitted changes
- **Version Verification**: Validates version format and file updates
- **Interactive Confirmation**: Asks for confirmation before proceeding
- **Error Handling**: Stops execution on any error and provides clear messages

## Examples

### Release a stable version:
```bash
./scripts/release.sh 1.0.0
```

### Release a pre-release version:
```bash
./scripts/release.sh 1.0.1-beta.1
```

### Interactive mode:
```bash
./scripts/release.sh
# Will prompt: Enter new version (e.g., 1.0.0 or 1.0.0-beta.1):
```

## Prerequisites

- Git repository with remote configured
- Write access to the repository
- Clean working directory (or willingness to proceed with uncommitted changes)

## Troubleshooting

### "Please run this script from the project root directory"
Make sure you're running the script from the root of the Chatspeed project where `src-tauri/tauri.conf.json` exists.

### "Not a Git repository"
Ensure you're in a Git repository and have initialized it properly.

### Git push fails
Check your Git credentials and network connection. You may need to authenticate with GitHub.

### Version format error
Use semantic versioning format: `x.y.z` or `x.y.z-prerelease` (e.g., `1.0.0`, `1.0.0-beta.1`)