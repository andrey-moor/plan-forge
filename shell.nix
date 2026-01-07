{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell {
  name = "plan-forge-dev";

  buildInputs = with pkgs; [
    # Rust toolchain
    rustup

    # Build dependencies
    pkg-config
    openssl
    libiconv

    # Development tools
    just
    jq
  ];

  shellHook = ''
    # Ensure rustup is set up
    export RUSTUP_HOME="$HOME/.rustup"
    export CARGO_HOME="$HOME/.cargo"
    export PATH="$CARGO_HOME/bin:$PATH"

    # Set up rust toolchain if needed
    if ! rustup show active-toolchain &>/dev/null; then
      rustup default stable
    fi

    # OpenSSL configuration for builds
    export PKG_CONFIG_PATH="${pkgs.openssl.dev}/lib/pkgconfig"

    echo "plan-forge development environment loaded"
    echo "Rust: $(rustc --version)"
    echo ""
    echo "Commands:"
    echo "  cargo build          - Build the project"
    echo "  cargo test           - Run tests"
    echo "  cargo run -- --help  - Run CLI"
  '';

  # Environment variables
  RUST_BACKTRACE = "1";
  RUST_LOG = "info";
}
