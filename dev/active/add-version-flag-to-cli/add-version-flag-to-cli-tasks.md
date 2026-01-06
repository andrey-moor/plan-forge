# Add --version flag to CLI - Tasks

## Progress

- **Total**: 8
- **Completed**: 0 ✅
- **In Progress**: 0 🔄

---

## Verification

- [ ] **Run `cargo run -- --version` and confirm output shows 'plan-review-cli 0.1.0'**
  - Location: `src/main.rs`, `Cargo.toml`
  - Notes: The clap derive macro with #[command(version)] automatically reads CARGO_PKG_VERSION which Cargo sets from Cargo.toml at compile time
  - Validation: Both --version and -V output 'plan-review-cli 0.1.0'

- [ ] **Run `cargo run -- -V` to confirm short flag also works and outputs identical result**
  - Location: `src/main.rs`
  - Notes: clap automatically provides -V as a short alias for --version
  - Validation: Both --version and -V output 'plan-review-cli 0.1.0'

- [ ] **Temporarily change version in Cargo.toml from '0.1.0' to '0.1.1-test'**
  - Location: `Cargo.toml`
  - Notes: Edit the version field in [package] section. IMPORTANT: If any subsequent step fails, immediately revert this change before investigating.
  - Validation: Version output changes when Cargo.toml version changes and rebuild occurs; Cargo.toml is restored to original state

- [ ] **Run `cargo build` followed by `cargo run -- --version` and confirm output shows 'plan-review-cli 0.1.1-test'**
  - Location: `Cargo.toml`
  - Notes: Rebuild is required because CARGO_PKG_VERSION is embedded at compile time
  - Validation: Version output changes when Cargo.toml version changes and rebuild occurs; Cargo.toml is restored to original state

- [ ] **Revert Cargo.toml version back to '0.1.0' and rebuild**
  - Location: `Cargo.toml`
  - Notes: Restore original version to leave codebase unchanged. This step is mandatory regardless of previous step outcomes.
  - Validation: Version output changes when Cargo.toml version changes and rebuild occurs; Cargo.toml is restored to original state

## Documentation

- [ ] **Run `cargo run -- --help` and verify --version/-V flags are listed in the Options section**
  - Location: `src/main.rs`
  - Notes: clap automatically includes --version/-V in the Options section of help output when #[command(version)] is present
  - Validation: Help output lists -V/--version in the Options section alongside -h/--help

- [ ] **Add a new '### Global Options' section after line 39 (end of current CLI Options section) with entries for --help/-h and --version/-V**
  - Location: `CLAUDE.md`
  - Notes: Insert after line 39. The new section should be titled '### Global Options' to distinguish from the existing '### CLI Options' which documents subcommand-specific options. Include both --help/-h and --version/-V for completeness. Format: '- `-V, --version`: Display version information' and '- `-h, --help`: Display help information'
  - Validation: CLAUDE.md has a 'Global Options' section documenting --version/-V and --help/-h, and the existing options section is renamed to clarify it covers 'run' subcommand options

- [ ] **Rename the existing '### CLI Options' header (line 30) to '### Run Subcommand Options' for clarity**
  - Location: `CLAUDE.md`
  - Notes: Change line 30 from '### CLI Options' to '### Run Subcommand Options' to clarify these are options for the 'run' subcommand, not global options
  - Validation: CLAUDE.md has a 'Global Options' section documenting --version/-V and --help/-h, and the existing options section is renamed to clarify it covers 'run' subcommand options

---
Progress: 0/8 tasks complete
