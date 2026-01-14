#!/bin/sh
# plan-forge installer
# Usage: curl -sSL https://raw.githubusercontent.com/andrey-moor/plan-forge/main/install.sh | sh
#
# Environment variables:
#   INSTALL_DIR - Custom installation directory (default: ~/.local/bin or /usr/local/bin)
#   VERSION     - Specific version to install (default: latest)

set -eu

# Colors (disabled if not a terminal)
if [ -t 1 ]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[0;33m'
    BLUE='\033[0;34m'
    NC='\033[0m' # No Color
else
    RED=''
    GREEN=''
    YELLOW=''
    BLUE=''
    NC=''
fi

info() {
    printf "${BLUE}info:${NC} %s\n" "$1"
}

success() {
    printf "${GREEN}success:${NC} %s\n" "$1"
}

warn() {
    printf "${YELLOW}warning:${NC} %s\n" "$1"
}

error() {
    printf "${RED}error:${NC} %s\n" "$1" >&2
}

# Show usage
usage() {
    cat <<EOF
plan-forge installer

USAGE:
    curl -sSL https://raw.githubusercontent.com/andrey-moor/plan-forge/main/install.sh | sh

OPTIONS:
    --help      Show this help message

ENVIRONMENT VARIABLES:
    INSTALL_DIR     Custom installation directory
                    Default: ~/.local/bin (if exists) or /usr/local/bin

    VERSION         Specific version to install (e.g., v0.1.0)
                    Default: latest release

EXAMPLES:
    # Install latest version
    curl -sSL https://raw.githubusercontent.com/andrey-moor/plan-forge/main/install.sh | sh

    # Install specific version
    VERSION=v0.1.0 curl -sSL https://raw.githubusercontent.com/andrey-moor/plan-forge/main/install.sh | sh

    # Install to custom directory
    INSTALL_DIR=/opt/bin curl -sSL https://raw.githubusercontent.com/andrey-moor/plan-forge/main/install.sh | sh

EOF
    exit 0
}

# Check for --help
for arg in "$@"; do
    case "$arg" in
        --help|-h)
            usage
            ;;
    esac
done

# Check prerequisites
check_prerequisites() {
    if ! command -v curl >/dev/null 2>&1; then
        error "curl is required but not installed"
        exit 1
    fi

    if ! command -v tar >/dev/null 2>&1; then
        error "tar is required but not installed"
        exit 1
    fi
}

# Detect OS
detect_os() {
    OS=$(uname -s)
    case "$OS" in
        Linux)
            echo "linux"
            ;;
        Darwin)
            echo "darwin"
            ;;
        *)
            error "Unsupported operating system: $OS"
            error "Supported: Linux, macOS"
            exit 1
            ;;
    esac
}

# Detect architecture
detect_arch() {
    ARCH=$(uname -m)
    case "$ARCH" in
        x86_64|amd64)
            echo "x86_64"
            ;;
        aarch64|arm64)
            echo "aarch64"
            ;;
        *)
            error "Unsupported architecture: $ARCH"
            error "Supported: x86_64, aarch64 (arm64)"
            exit 1
            ;;
    esac
}

# Get Rust target triple
get_target() {
    local os=$1
    local arch=$2

    case "${os}-${arch}" in
        linux-x86_64)
            echo "x86_64-unknown-linux-gnu"
            ;;
        linux-aarch64)
            echo "aarch64-unknown-linux-gnu"
            ;;
        darwin-x86_64)
            echo "x86_64-apple-darwin"
            ;;
        darwin-aarch64)
            echo "aarch64-apple-darwin"
            ;;
        *)
            error "Unsupported platform: ${os}-${arch}"
            exit 1
            ;;
    esac
}

# Get latest release version from GitHub API
get_latest_version() {
    local api_url="https://api.github.com/repos/andrey-moor/plan-forge/releases/latest"
    local version

    version=$(curl -sSL "$api_url" | grep '"tag_name":' | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')

    if [ -z "$version" ]; then
        error "Failed to fetch latest version from GitHub"
        exit 1
    fi

    echo "$version"
}

# Get install directory
get_install_dir() {
    if [ -n "${INSTALL_DIR:-}" ]; then
        echo "$INSTALL_DIR"
        return
    fi

    # Prefer ~/.local/bin if it exists
    if [ -d "$HOME/.local/bin" ]; then
        echo "$HOME/.local/bin"
    else
        echo "/usr/local/bin"
    fi
}

# Main installation
main() {
    info "plan-forge installer"
    echo ""

    check_prerequisites

    OS=$(detect_os)
    ARCH=$(detect_arch)
    TARGET=$(get_target "$OS" "$ARCH")

    info "Detected platform: ${OS}-${ARCH} (${TARGET})"

    # Get version
    if [ -n "${VERSION:-}" ]; then
        RELEASE_VERSION="$VERSION"
        info "Installing version: $RELEASE_VERSION"
    else
        info "Fetching latest release..."
        RELEASE_VERSION=$(get_latest_version)
        info "Latest version: $RELEASE_VERSION"
    fi

    # Download URL
    DOWNLOAD_URL="https://github.com/andrey-moor/plan-forge/releases/download/${RELEASE_VERSION}/plan-forge-${TARGET}.tar.gz"

    # Create temp directory
    TMP_DIR=$(mktemp -d)
    trap 'rm -rf "$TMP_DIR"' EXIT

    info "Downloading from: $DOWNLOAD_URL"
    if ! curl -fsSL "$DOWNLOAD_URL" -o "$TMP_DIR/plan-forge.tar.gz"; then
        error "Failed to download plan-forge"
        error "URL: $DOWNLOAD_URL"
        error "Make sure the version exists and supports your platform"
        exit 1
    fi

    info "Extracting archive..."
    tar xzf "$TMP_DIR/plan-forge.tar.gz" -C "$TMP_DIR"

    # Find the binary (might be in a subdirectory)
    BINARY_PATH=$(find "$TMP_DIR" -name "plan-forge" -type f | head -1)
    if [ -z "$BINARY_PATH" ]; then
        error "Binary not found in archive"
        exit 1
    fi

    # Get install directory
    INSTALL_DIR=$(get_install_dir)

    # Create install directory if needed
    if [ ! -d "$INSTALL_DIR" ]; then
        info "Creating directory: $INSTALL_DIR"
        mkdir -p "$INSTALL_DIR"
    fi

    # Install binary
    info "Installing to: $INSTALL_DIR/plan-forge"
    cp "$BINARY_PATH" "$INSTALL_DIR/plan-forge"
    chmod +x "$INSTALL_DIR/plan-forge"

    echo ""
    success "plan-forge $RELEASE_VERSION installed successfully!"
    echo ""

    # Check if install dir is in PATH
    case ":$PATH:" in
        *":$INSTALL_DIR:"*)
            info "Run 'plan-forge --version' to verify"
            ;;
        *)
            warn "$INSTALL_DIR is not in your PATH"
            echo ""
            echo "Add it to your shell configuration:"
            echo ""
            echo "  # For bash (~/.bashrc)"
            echo "  export PATH=\"\$PATH:$INSTALL_DIR\""
            echo ""
            echo "  # For zsh (~/.zshrc)"
            echo "  export PATH=\"\$PATH:$INSTALL_DIR\""
            echo ""
            ;;
    esac
}

main "$@"
