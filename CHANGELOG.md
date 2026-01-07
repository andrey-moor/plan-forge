# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- MCP server with planning tools (`plan_run`, `plan_status`, `plan_list`, `plan_get`, `plan_approve`)
- Claude Code integration with skills, commands, and agents
- LLM-based task slug generation
- Environment variable support (`PLAN_FORGE_*`)
- Bundled recipes (no external files required)
- Config file auto-detection (`.plan-forge/config.yaml`, `plan-forge.yaml`, etc.)

### Fixed
- CI system dependencies for libxcb and libdbus on Linux

## [0.1.0] - 2026-01-06

### Added
- Initial release of plan-forge CLI tool
- Iterative plan-review feedback loop with configurable threshold
- Planner and Reviewer agents using goose crate
- Support for Anthropic, OpenAI, and LiteLLM providers
- Resume workflow for providing feedback on generated plans
- Structured plan output with phases, tasks, and acceptance criteria
- Hard checks for plan validation (structure, completeness)
- YAML recipe system for customizing LLM behavior
- MCP extension support in recipes
- Markdown output to `./dev/active/<task-slug>/`

[Unreleased]: https://github.com/andrey-moor/plan-forge/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/andrey-moor/plan-forge/releases/tag/v0.1.0
