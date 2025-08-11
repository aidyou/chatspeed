# Release Scripts

This directory contains automated release scripts for the Chatspeed project.

## Scripts

### `release.sh` (macOS/Linux)

Bash script for Unix-like systems that handles version updates and tag creation.

**Usage:**
```bash
# Interactive mode
./scripts/release.sh

# Direct version specification
./scripts/release.sh 1.0.0
./scripts/release.sh v1.0.1-beta.1
```

### `release.ps1` (Windows)

PowerShell script for Windows systems with the same functionality.

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
4. **Git Tag Operations**:
   - Creates a Git tag "v{version}"
   - Intelligently selects the correct remote repository (prioritizes GitHub)
   - Pushes the tag to the selected remote repository
5. **CI/CD Trigger**: The pushed tag automatically triggers the GitHub Actions release workflow

## Safety Features

- **Version Verification**: Validates version format and file updates
- **Interactive Confirmation**: Asks for confirmation before proceeding
- **Smart Remote Selection**: Automatically detects and uses GitHub remote repository
- **Tag Conflict Handling**: Checks for existing tags and offers to recreate them
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

- Git repository with GitHub remote configured
- Write access to the GitHub repository
- Clean working directory (or willingness to proceed with uncommitted changes)

## Remote Repository Setup

The script intelligently selects the target remote repository in this order:

1. **`github` remote** - If you have a dedicated GitHub remote
2. **`origin` pointing to GitHub** - If origin URL contains `github.com`
3. **Any GitHub remote** - Searches all remotes for GitHub URLs
4. **`origin` as fallback** - Uses origin if no GitHub remote is found

### Recommended Setup for Multiple Remotes:

```bash
# Add GitHub as a dedicated remote
git remote add github https://github.com/aidyou/chatspeed.git

# Keep your development remote as origin
git remote add origin https://your-dev-server.com/chatspeed.git
```

## Workflow Integration

- **build.yml**: Manual testing builds only (no automatic triggers)
- **release.yml**: Automatically triggered by version tags for official releases

## Troubleshooting

### "Please run this script from the project root directory"
Make sure you're running the script from the root of the Chatspeed project where `src-tauri/tauri.conf.json` exists.

### "Not a Git repository"
Ensure you're in a Git repository and have initialized it properly.

### "No GitHub remote found"
The script couldn't find a remote pointing to GitHub. Either:
- Add a GitHub remote: `git remote add github https://github.com/aidyou/chatspeed.git`
- Or ensure your `origin` points to GitHub

### "Tag already exists locally"
The script will ask if you want to delete and recreate the existing tag. Choose 'y' to proceed or 'N' to skip.

### Git push fails
Check your Git credentials and network connection. You may need to authenticate with GitHub.

### Version format error
Use semantic versioning format: `x.y.z` or `x.y.z-prerelease` (e.g., `1.0.0`, `1.0.0-beta.1`)

## Debug Tools

Use `./scripts/debug-git.sh` to check your Git configuration and remote setup if you encounter issues.