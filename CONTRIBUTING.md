# Contributing to plan-forge

Thank you for your interest in contributing to plan-forge! This document provides guidelines and instructions for contributing.

## Code of Conduct

This project adheres to the [Contributor Covenant Code of Conduct](CODE_OF_CONDUCT.md). By participating, you are expected to uphold this code.

## How to Contribute

### Reporting Bugs

Before creating a bug report, please check existing issues to avoid duplicates. When creating a bug report, include:

- A clear, descriptive title
- Steps to reproduce the issue
- Expected behavior vs actual behavior
- Your environment (OS, Rust version, provider)
- Relevant logs or error messages

### Suggesting Features

Feature requests are welcome! Please:

- Check existing issues and discussions first
- Describe the use case and problem it solves
- Provide examples of how it would work

### Pull Requests

1. **Fork the repository** and create your branch from `main`
2. **Set up development environment** (see below)
3. **Make your changes** following the code style guidelines
4. **Add tests** if applicable
5. **Run the test suite** to ensure nothing is broken
6. **Submit a pull request** with a clear description

## Development Setup

### Prerequisites

- Rust 1.91+ (edition 2024)
- An API key for testing (Anthropic, OpenAI, or LiteLLM)

### Getting Started

```bash
# Clone your fork
git clone https://github.com/YOUR_USERNAME/plan-forge.git
cd plan-forge

# Build the project
cargo build

# Run tests
cargo test

# Set up your API key
export ANTHROPIC_API_KEY="your-key"

# Test a run
cargo run -- run --task "Test task" --max-iterations 1
```

### Project Structure

```text
plan-forge/
├── src/
│   ├── main.rs              # CLI entry point
│   ├── lib.rs               # Public API
│   ├── config/              # Configuration handling
│   ├── models/              # Data structures (Plan, Review)
│   ├── orchestrator/        # Loop controller
│   ├── phases/              # Planner, Reviewer, Updater
│   └── output/              # File output
├── recipes/                 # LLM agent configurations
│   ├── planner.yaml
│   └── reviewer.yaml
├── config/
│   └── default.yaml         # Default configuration
└── dev/active/              # Generated plans output
```

## Code Style

### Formatting

All code must be formatted with `rustfmt`:

```bash
cargo fmt
```

### Linting

Code should pass `clippy` without warnings:

```bash
cargo clippy -- -D warnings
```

### Guidelines

- Follow Rust naming conventions (snake_case for functions/variables, CamelCase for types)
- Write descriptive commit messages
- Keep functions focused and reasonably sized
- Add doc comments for public APIs
- Use `anyhow` for error handling in application code
- Use `thiserror` for library error types

### Testing

```bash
# Run all tests
cargo test

# Run specific test
cargo test test_name

# Run with output
cargo test -- --nocapture
```

## Commit Messages

Use clear, descriptive commit messages:

```text
type: short description

Longer description if needed. Explain what and why,
not how (the code shows how).

Fixes #123
```

Types: `feat`, `fix`, `docs`, `style`, `refactor`, `test`, `chore`

Examples:
- `feat: add OpenAI provider support`
- `fix: handle empty task descriptions`
- `docs: update README with LiteLLM setup`

## Pull Request Process

1. Update documentation if you're changing behavior
2. Add tests for new functionality
3. Ensure CI passes (cargo check, test, clippy, fmt)
4. Request review from maintainers
5. Address feedback promptly

## Release Process

Releases are managed by maintainers. Version bumps follow [Semantic Versioning](https://semver.org/):

- **MAJOR**: Breaking changes
- **MINOR**: New features, backwards compatible
- **PATCH**: Bug fixes, backwards compatible

## Questions?

- Open a [Discussion](https://github.com/andrey-moor/plan-forge/discussions) for questions
- Check existing [Issues](https://github.com/andrey-moor/plan-forge/issues) for known problems
- Review the [README](README.md) for usage information

Thank you for contributing!
