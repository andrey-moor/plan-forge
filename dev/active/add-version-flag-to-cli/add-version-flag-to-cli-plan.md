# Add --version flag to CLI

**Status:** In Progress
**Created:** 2026-01-06T13:48:00Z
**Last Updated:** 2026-01-06T13:52:00Z
**Iteration:** 3

## Overview

Verify and document the existing --version flag implementation. The feature is already implemented via clap's derive macro with #[command(version)], which automatically extracts the version from Cargo.toml via CARGO_PKG_VERSION. This plan focuses on verification and documentation updates, as no code changes are required. Documentation will add a new 'Global Options' section to distinguish top-level options (--version, --help) from subcommand-specific options.

## Current State

Add a --version flag that displays the version from Cargo.toml. Upon investigation, this feature already exists through clap's derive macro. This plan verifies the existing implementation and updates documentation to reflect this capability, including proper placement of global options separate from subcommand options.

**Constraints:**
- Must read version from Cargo.toml (single source of truth)
- Must follow existing CLI patterns using clap derive macros
- Must work with both --version and -V flags
- No code changes required - feature already implemented
- Documentation must distinguish between global options and subcommand-specific options

**Assumptions:**
- The clap crate with derive feature is available (confirmed in Cargo.toml)
- Standard Rust/Cargo build process is used (CARGO_PKG_VERSION is set at compile time)
- CLAUDE.md serves as primary documentation
- The CLI uses subcommands (run, mcp) and global options apply at the top level

## Phases

1. **Verification**: Confirm the --version flag already works as expected and verify automatic version sync behavior
2. **Documentation**: Update project documentation to include --version flag usage in a new Global Options section

## Key Files

- `Cargo.toml` - Contains version field in [package] section, which is the single source of truth for the version
- `src/main.rs` - Contains #[command(version)] on the Cli struct which enables clap's automatic version flag using CARGO_PKG_VERSION
- `CLAUDE.md` - Rename existing CLI Options section to 'Run Subcommand Options' and add new 'Global Options' section after line 39 documenting --version/-V and --help/-h

## Risks

⚠️ **Risk**: Feature already implemented - no code changes required - **Mitigation**: This plan focuses on verification and documentation. The --version flag already works via clap's derive macro.
⚠️ **Risk**: Version sync relies on Cargo build process - **Mitigation**: CARGO_PKG_VERSION is a standard Cargo feature and is reliable. The version is embedded at compile time, ensuring consistency. Users must rebuild after changing Cargo.toml version.
⚠️ **Risk**: Verification step temporarily modifies Cargo.toml - **Mitigation**: If any step fails during verification, immediately revert Cargo.toml version back to '0.1.0' before investigating the failure. The plan explicitly includes reverting as a mandatory final step.

## Success Criteria

- Running `plan-review-cli --version` outputs the version string in format 'plan-review-cli X.Y.Z'
- Running `plan-review-cli -V` outputs the same version string as --version
- Version displayed matches the version field in Cargo.toml
- Version updates automatically when Cargo.toml version changes and project is rebuilt
- CLAUDE.md contains a 'Global Options' section documenting --version/-V and --help/-h
- CLAUDE.md existing options section is renamed to 'Run Subcommand Options' to distinguish from global options
