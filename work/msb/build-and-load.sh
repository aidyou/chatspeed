#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

usage() {
  cat <<'EOF'
Usage: ./build-and-load.sh [--cn] [name ...]

Build Dockerfiles in this directory, export them as Docker archives, and load them into msb.

Options:
  --cn              Use China mirrors for package downloads during image builds.
  -h, --help        Show this help.

Arguments:
  No arguments       Build and load every *.Dockerfile in this directory.
  name [name ...]    Build and load <name>.Dockerfile only.

Examples:
  ./build-and-load.sh
  ./build-and-load.sh --cn
  ./build-and-load.sh php
  ./build-and-load.sh --cn node python-slim

Images are tagged in msb as: <name>:latest
EOF
}

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    printf 'Error: required command not found: %s\n' "$1" >&2
    exit 1
  fi
}

build_and_load() {
  local dockerfile="$1"
  local use_cn_mirrors="$2"
  local filename name image_ref temp_dir archive

  filename="$(basename "$dockerfile")"
  name="${filename%.Dockerfile}"
  image_ref="${name}:latest"
  temp_dir="$(mktemp -d "${TMPDIR:-/tmp}/msb-${name}.XXXXXX")"
  archive="$temp_dir/image.tar"

  printf '\n==> Building %s as %s\n' "$filename" "$image_ref"
  docker buildx build \
    --file "$dockerfile" \
    --build-arg "USE_CN_MIRRORS=${use_cn_mirrors}" \
    --tag "$image_ref" \
    --load \
    "$SCRIPT_DIR"

  printf '==> Exporting %s from Docker\n' "$image_ref"
  docker save --output "$archive" "$image_ref"

  printf '==> Loading %s into msb\n' "$image_ref"
  msb image load --input "$archive" --tag "$image_ref"

  rm -rf "$temp_dir"
}

main() {
  local -a dockerfiles=()
  local name dockerfile use_cn_mirrors=0

  while (( $# > 0 )); do
    case "$1" in
      --cn)
        use_cn_mirrors=1
        shift
        ;;
      -h|--help)
        usage
        exit 0
        ;;
      --)
        shift
        break
        ;;
      -*)
        printf 'Error: unknown option: %s\n' "$1" >&2
        usage >&2
        exit 1
        ;;
      *)
        break
        ;;
    esac
  done

  require_command docker
  require_command msb

  if ! docker buildx version >/dev/null 2>&1; then
    printf 'Error: Docker Buildx is required to build and load images.\n' >&2
    exit 1
  fi

  if (( $# == 0 )); then
    while IFS= read -r dockerfile; do
      dockerfiles+=("$dockerfile")
    done < <(find "$SCRIPT_DIR" -maxdepth 1 -type f -name '*.Dockerfile' -print | sort)
  else
    for name in "$@"; do
      dockerfile="$SCRIPT_DIR/${name}.Dockerfile"
      if [[ ! -f "$dockerfile" ]]; then
        printf 'Error: Dockerfile not found: %s\n' "$dockerfile" >&2
        exit 1
      fi
      dockerfiles+=("$dockerfile")
    done
  fi

  if (( ${#dockerfiles[@]} == 0 )); then
    printf 'Error: no *.Dockerfile files found in %s\n' "$SCRIPT_DIR" >&2
    exit 1
  fi

  for dockerfile in "${dockerfiles[@]}"; do
    build_and_load "$dockerfile" "$use_cn_mirrors"
  done

  printf '\nDone. View imported images with: msb image list\n'
}

main "$@"
