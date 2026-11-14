#!/bin/bash

# Chatspeed Release Script - Simplified Version
# Automatically updates version numbers and creates release tags

set -e  # Exit immediately on error

# Color definitions
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[ NC' # No Color

# Print colored messages
print_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

print_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Validate version format
validate_version() {
    local version=$1
    # Remove possible 'v' prefix
    version=${version#v}

    # Validate semantic version format (x.y.z)
    if [[ ! $version =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.-]+)?$ ]]; then
        print_error "Invalid version format: $version"
        print_error "Expected format: x.y.z or x.y.z-prerelease (e.g., 1.0.0, 1.0.0-beta.1)"
        exit 1
    fi

    echo $version
}

# Get current version from configuration
get_current_version() {
    if [[ -f "src-tauri/tauri.conf.json" ]]; then
        grep '"version"' src-tauri/tauri.conf.json | head -1 | sed 's/.*"version": *"\([^"]*\)".*/\1/'
    else
        echo "unknown"
    fi
}

# Update version in tauri.conf.json
update_tauri_config() {
    local version=$1
    local file="src-tauri/tauri.conf.json"

    if [[ ! -f "$file" ]]; then
        print_error "File not found: $file"
        exit 1
    fi

    # Use sed to update version number
    if [[ "$OSTYPE" == "darwin"* ]]; then
        # macOS
        sed -i '' "s/\"version\": *\"[^\"]*\"/\"version\": \"$version\"/" "$file"
    else
        # Linux/Windows
        sed -i "s/\"version\": *\"[^\"]*\"/\"version\": \"$version\"/" "$file"
    fi

    print_success "Updated version in $file to $version"
}

# Only first-party packages sharing the desktop release version belong here.
RELEASE_MANIFESTS=(
    src-tauri/Cargo.toml
    src-tauri/crates/daemon/Cargo.toml
    src-tauri/crates/backend/Cargo.toml
    src-tauri/crates/cli/Cargo.toml
    src-tauri/crates/client/Cargo.toml
    src-tauri/crates/contracts/Cargo.toml
)

# Read package metadata without inspecting dependency version requirements.
get_package_field() {
    awk -v key="$2" '
        /^\[package\]$/ { in_package = 1; next }
        /^\[/ { in_package = 0 }
        in_package && $0 ~ "^[[:space:]]*" key "[[:space:]]*=" {
            sub(/^[^=]*=[[:space:]]*"/, "")
            sub(/".*/, "")
            print
            exit
        }
    ' "$1"
}

# Update the release packages together, leaving third-party dependencies alone.
update_cargo_toml() {
    local version=$1
    local file
    for file in "${RELEASE_MANIFESTS[@]}"; do
        if [[ ! -f "$file" ]]; then
            print_error "File not found: $file"
            exit 1
        fi
        if [[ "$OSTYPE" == "darwin"* ]]; then
            sed -i '' "/^\[package\]$/,/^\[/ s/^version[[:space:]]*=[[:space:]]*\"[^\"]*\"/version = \"$version\"/" "$file"
        else
            sed -i "/^\[package\]$/,/^\[/ s/^version[[:space:]]*=[[:space:]]*\"[^\"]*\"/version = \"$version\"/" "$file"
        fi
        print_success "Updated version in $file to $version"
    done
}

# Update version in vcpkg.json
update_vcpkg_json() {
    local version=$1
    local file="vcpkg.json"

    if [[ ! -f "$file" ]]; then
        print_warning "vcpkg.json not found, skipping version update for this file."
        return
    fi

    # Use sed to update version number
    if [[ "$OSTYPE" == "darwin"* ]]; then
        # macOS
        sed -i '' "s/\"version\": *\"[^\"]*\"/\"version\": \"$version\"/" "$file"
    else
        # Linux/Windows
        sed -i "s/\"version\": *\"[^\"]*\"/\"version\": \"$version\"/" "$file"
    fi

    print_success "Updated version in $file to $version"
}

# Report every mismatch before allowing any release side effects.
verify_version_update() {
    local expected_version=$1
    local failed=0
    local file actual package locked

    actual=$(get_current_version)
    if [[ "$actual" != "$expected_version" ]]; then
        print_error "src-tauri/tauri.conf.json: expected $expected_version, got $actual"
        failed=1
    fi
    for file in "${RELEASE_MANIFESTS[@]}"; do
        if [[ ! -f "$file" ]]; then
            print_error "$file: missing release manifest"
            failed=1
            continue
        fi
        actual=$(get_package_field "$file" version)
        package=$(get_package_field "$file" name)
        if [[ -z "$package" || "$actual" != "$expected_version" ]]; then
            print_error "$file [package] ($package): expected $expected_version, got ${actual:-missing}"
            failed=1
        fi
        locked=""
        if [[ -n "$package" && -f src-tauri/Cargo.lock ]]; then
            locked=$(awk -v package="$package" '
                /^\[\[package\]\]$/ { matches = 0 }
                /^name = / { matches = ($0 == "name = \"" package "\"") }
                matches && /^version = / {
                    sub(/^version = "/, ""); sub(/".*/, ""); print
                }
            ' src-tauri/Cargo.lock)
        fi
        if [[ "$locked" != "$expected_version" ]]; then
            print_error "src-tauri/Cargo.lock ($package): expected $expected_version, got ${locked:-missing}"
            failed=1
        fi
    done
    if [[ -f vcpkg.json ]]; then
        actual=$(grep '"version"' vcpkg.json | head -1 | sed 's/.*"version": *"\([^"]*\)".*/\1/')
        if [[ "$actual" != "$expected_version" ]]; then
            print_error "vcpkg.json: expected $expected_version, got ${actual:-missing}"
            failed=1
        fi
    fi
    if [[ "$failed" -ne 0 ]]; then
        print_error "Release version consistency check failed; release aborted."
        exit 1
    fi
    print_success "Version verification passed: $expected_version"
}

# Commit all version-related changes
commit_version_changes() {
    local version=$1

    # Run cargo to update Cargo.lock
    print_info "Updating Cargo.lock file..."
    cd src-tauri
    cargo metadata --format-version 1 > /dev/null 2>&1 || cargo generate-lockfile
    cd ..

    # Validate the generated lockfile before staging, committing, or tagging.
    verify_version_update "$version"
    local version_files=(src-tauri/tauri.conf.json "${RELEASE_MANIFESTS[@]}" src-tauri/Cargo.lock)
    if [[ -f vcpkg.json ]]; then
        version_files+=(vcpkg.json)
    fi
    if ! git diff --quiet -- "${version_files[@]}"; then
        print_info "Committing version changes..."

        git add -- "${version_files[@]}"

        # Commit with chore prefix
        git commit -m "chore: bump version to $version"

        print_success "Committed version changes"
    else
        print_info "No version changes to commit"
    fi
}

# Get target remote repository (prioritize 'github' or 'origin' with github.com URL)
get_target_remote() {
    if git remote get-url github >/dev/null 2>&1; then
        echo "github"
        return
    fi

    if git remote get-url origin >/dev/null 2>&1; then
        local origin_url=$(git remote get-url origin)
        if [[ "$origin_url" == *"github.com"* ]]; then
            echo "origin"
            return
        fi
    fi

    for remote in $(git remote); do
        local url=$(git remote get-url "$remote" 2>/dev/null || echo "")
        if [[ "$url" == *"github.com"* ]]; then
            echo "$remote"
            return
        fi
    done

    echo "origin"
}

# Create a local tag and push everything to remote
create_and_push_tag() {
    local version=$1
    local tag="v$version"
    local remote=$(get_target_remote)

    print_info "Target remote: $remote ($(git remote get-url $remote))"

    # Handle existing remote tags
    if git ls-remote --tags "$remote" | grep -q "refs/tags/$tag$"; then
        print_warning "Tag $tag already exists on remote $remote"
        read -p "Do you want to delete and recreate it? (y/N): " -n 1 -r
        echo
        if [[ $REPLY =~ ^[Yy]$ ]]; then
            print_info "Deleting remote tag $tag..."
            git push "$remote" ":refs/tags/$tag" || print_warning "Failed to delete remote tag"

            if git tag -l | grep -q "^$tag$"; then
                git tag -d "$tag"
                print_info "Deleted existing local tag $tag"
            fi
        else
            print_info "Skipping tag creation"
            return
        fi
    fi

    # Handle existing local tags
    if git tag -l | grep -q "^$tag$"; then
        print_warning "Tag $tag already exists locally"
        git tag -d "$tag"
        print_info "Deleted existing local tag $tag"
    fi

    # Push the version commit first
    print_info "Pushing version commit to remote $remote..."
    if git push "$remote" HEAD; then
        print_success "Successfully pushed version commit to $remote"
    else
        print_error "Failed to push version commit to $remote"
        exit 1
    fi

    sleep 2

    # Create tag on current HEAD
    print_info "Creating tag $tag on current HEAD..."
    git tag "$tag" HEAD
    print_success "Created tag $tag"

    # Verify tag points to the correct commit
    local tag_commit=$(git rev-list -n 1 "$tag")
    local head_commit=$(git rev-parse HEAD)

    if [[ "$tag_commit" != "$head_commit" ]]; then
        print_error "Tag $tag does not point to current HEAD"
        exit 1
    fi

    # Push the tag
    print_info "Pushing tag $tag to remote $remote..."
    if git push "$remote" "$tag"; then
        print_success "Successfully pushed tag $tag to $remote"
    else
        print_error "Failed to push tag $tag to $remote"
        print_info "You can try pushing manually: git push $remote $tag"
        exit 1
    fi
}

# Display usage guide
show_usage() {
    echo "Usage: $0 [version]"
    echo ""
    echo "Examples:"
    echo "  $0                    # Interactive mode"
    echo "  $0 1.0.0             # Release version 1.0.0"
    echo "  $0 v1.0.1-beta.1     # Release pre-release version"
    echo ""
    echo "The script will:"
    echo "  1. Check shared release versions before changing files"
    echo "  2. Update Tauri config and all six first-party Rust packages"
    echo "  3. Update vcpkg.json (if present) and Cargo.lock; verify again"
    echo "  4. Commit version files and create a Git tag (v{version})"
    echo "  5. Push tag to remote repository"
}

# Main entry point
main() {
    print_info "Chatspeed Release Script"
    print_info "========================"

    # Verify project structure
    if [[ ! -f "src-tauri/tauri.conf.json" ]] || [[ ! -f "src-tauri/Cargo.toml" ]]; then
        print_error "Please run this script from the project root directory"
        exit 1
    fi

    # Verify git repository
    if ! git rev-parse --git-dir > /dev/null 2>&1; then
        print_error "Not a Git repository"
        exit 1
    fi

    local version=""
    local current_version=$(get_current_version)

    # Parse arguments
    if [[ $# -eq 0 ]]; then
        # Interactive mode
        print_info "Current version: $current_version"
        echo -n "Enter new version (e.g., 1.0.0 or 1.0.0-beta.1): "
        read version

        if [[ -z "$version" ]]; then
            print_error "Version cannot be empty"
            exit 1
        fi
    elif [[ $# -eq 1 ]]; then
        if [[ "$1" == "-h" ]] || [[ "$1" == "--help" ]]; then
            show_usage
            exit 0
        fi
        version=$1
    else
        print_error "Too many arguments"
        show_usage
        exit 1
    fi

    # Fail before modifying files if the existing release packages have drifted.
    verify_version_update "$current_version"

    # Sanitize version string
    version=$(validate_version "$version")

    # Display summary
    echo ""
    print_info "Release Summary:"
    print_info "  Current version: $current_version"
    print_info "  New version: $version"
    print_info "  Tag: v$version"
    echo ""

    # User confirmation
    read -p "Do you want to proceed with the release? (y/N): " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        print_info "Release cancelled."
        exit 0
    fi

    print_info "Starting release process..."

    # Update all config files
    update_tauri_config "$version"
    update_cargo_toml "$version"
    update_vcpkg_json "$version"

    # Commit regenerates and verifies the lockfile before any Git mutation.
    commit_version_changes "$version"
    create_and_push_tag "$version"

    echo ""
    print_success "🎉 Release $version completed successfully!"
    print_info "GitHub Actions will now build and create the release automatically."
    print_info "Check progress at: https://github.com/aidyou/chatspeed/actions"
}

main "$@"