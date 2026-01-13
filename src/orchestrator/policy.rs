//! Multi-format policy extraction from CLAUDE.md and AGENT.md files.
//!
//! This module extracts enforceable rules from policy files:
//! - CLAUDE.md: Simple headers, Cargo build system → CLAUDE-xxx rule IDs
//! - AGENT.md: Emoji headers, structured sections, Bazel → BZL-xxx, AGENT-xxx rule IDs

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

use crate::models::Instruction;

// ============================================================================
// Policy Types
// ============================================================================

/// Detected format of the policy file
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PolicyFileFormat {
    /// CLAUDE.md format (simple headers, Cargo build system)
    ClaudeMd,
    /// AGENT.md format (emoji headers, structured sections, Bazel build system)
    AgentMd,
}

/// Category for grouping and filtering policy rules
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PolicyCategory {
    /// Test requirements (cargo test, bazel test)
    Testing,
    /// Build requirements (cargo build, bazel build)
    Build,
    /// Security rules (credentials, keys)
    Security,
    /// Code style (cargo fmt, formatting)
    Style,
    /// Dependency management (bazel sync, cargo update)
    Dependencies,
    /// Environment setup (WSL, paths)
    Environment,
    /// Process rules (commits, PRs)
    Workflow,
    /// Explicitly forbidden actions
    Prohibited,
}

/// Severity level for policy rules
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PolicySeverity {
    /// Blocks plan approval
    Critical,
    /// Should be addressed but doesn't block
    Warning,
    /// Informational, for context
    Info,
}

impl Default for PolicySeverity {
    fn default() -> Self {
        Self::Warning
    }
}

/// A policy rule extracted from CLAUDE.md or AGENT.md
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    /// Stable rule ID (e.g., "CLAUDE-001", "BZL-001", "AGENT-001")
    pub id: String,
    /// Source file (e.g., "CLAUDE.md", "AGENT.md")
    pub source_file: String,
    /// Line number in source file where rule was extracted
    pub source_line: Option<usize>,
    /// Category for grouping and filtering
    pub category: PolicyCategory,
    /// Human-readable description of the rule
    pub description: String,
    /// Keywords that triggered this rule extraction
    pub keywords: Vec<String>,
    /// Regex pattern to verify compliance in generated plans
    /// Applied to instruction params (e.g., "cargo test" must appear)
    pub enforcement_pattern: Option<String>,
    /// Regex pattern that indicates violation
    /// Applied to instruction params (e.g., "python -m pytest" is forbidden)
    pub violation_pattern: Option<String>,
    /// Severity level
    pub severity: PolicySeverity,
    /// Original rule header/title from the file
    pub original_title: Option<String>,
}

impl Default for PolicyRule {
    fn default() -> Self {
        Self {
            id: String::new(),
            source_file: String::new(),
            source_line: None,
            category: PolicyCategory::Workflow,
            description: String::new(),
            keywords: Vec::new(),
            enforcement_pattern: None,
            violation_pattern: None,
            severity: PolicySeverity::Warning,
            original_title: None,
        }
    }
}

/// A set of policies extracted from a file
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PolicySet {
    /// Detected format of the policy file
    pub format: Option<PolicyFileFormat>,
    /// Source file path
    pub source_path: String,
    /// Extracted rules
    pub rules: Vec<PolicyRule>,
    /// Build system detected (Cargo, Bazel, npm, etc.)
    pub build_system: Option<String>,
    /// Extraction timestamp
    pub extracted_at: String,
}

/// A violation detected when verifying a plan against policies
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyViolation {
    /// Rule ID that was violated
    pub rule_id: String,
    /// Instruction ID that caused the violation (if applicable)
    pub instruction_id: Option<String>,
    /// Human-readable description
    pub message: String,
    /// Severity level
    pub severity: PolicySeverity,
}

// ============================================================================
// Format Detection
// ============================================================================

/// Detect whether the content is CLAUDE.md or AGENT.md format
pub fn detect_format(content: &str, filename: &str) -> PolicyFileFormat {
    // Check filename first
    let lower_name = filename.to_lowercase();
    if lower_name.contains("agent") {
        return PolicyFileFormat::AgentMd;
    }
    if lower_name.contains("claude") {
        return PolicyFileFormat::ClaudeMd;
    }

    // Check for AGENT.md markers
    let agent_markers = [
        "## 🏠 Rule:",
        "## 🔧 Rule:",
        "## ⚠️ Rule:",
        "### ❌ DO NOT DO THIS:",
        "### ✅ ALWAYS DO THIS:",
        "bazel test",
        "bazel build",
        ".pre-commit-config.yaml",
    ];

    let agent_score: usize = agent_markers.iter().filter(|m| content.contains(*m)).count();

    // Check for CLAUDE.md markers
    let claude_markers = [
        "cargo test",
        "cargo build",
        "cargo clippy",
        "cargo fmt",
        "# Build and Development Commands",
        "cargo run",
        "cargo check",
    ];

    let claude_score: usize = claude_markers
        .iter()
        .filter(|m| content.contains(*m))
        .count();

    if agent_score > claude_score {
        PolicyFileFormat::AgentMd
    } else {
        PolicyFileFormat::ClaudeMd
    }
}

// ============================================================================
// Keyword Extraction
// ============================================================================

/// Extract keywords from text that indicate rule importance
fn extract_keywords(text: &str) -> Vec<String> {
    let keywords = [
        "CRITICAL",
        "ALWAYS",
        "NEVER",
        "MUST",
        "DO NOT",
        "NON-NEGOTIABLE",
        "REQUIRED",
        "FORBIDDEN",
        "IMPORTANT",
    ];
    keywords
        .iter()
        .filter(|k| text.to_uppercase().contains(*k))
        .map(|k| k.to_string())
        .collect()
}

/// Determine severity based on keywords in text
fn determine_severity(text: &str) -> PolicySeverity {
    let text_upper = text.to_uppercase();

    if text_upper.contains("CRITICAL")
        || text_upper.contains("NEVER")
        || text_upper.contains("ALWAYS")
        || text_upper.contains("MUST")
        || text_upper.contains("NON-NEGOTIABLE")
        || text_upper.contains("FORBIDDEN")
    {
        PolicySeverity::Critical
    } else if text_upper.contains("SHOULD") || text_upper.contains("RECOMMEND") {
        PolicySeverity::Warning
    } else {
        PolicySeverity::Info
    }
}

// ============================================================================
// AGENT.md Extraction
// ============================================================================

/// Categorize a rule from AGENT.md based on title and content
fn categorize_agent_rule(title: &str) -> (PolicyCategory, &'static str) {
    let title_lower = title.to_lowercase();

    // Bazel-specific rules
    if title_lower.contains("bazel") || title_lower.contains("hermetic") {
        return (PolicyCategory::Build, "BZL");
    }

    // Test rules
    if title_lower.contains("test") || title_lower.contains("integration") {
        return (PolicyCategory::Testing, "AGENT");
    }

    // Security rules
    if title_lower.contains("secret")
        || title_lower.contains("credential")
        || title_lower.contains("key")
        || title_lower.contains("password")
        || title_lower.contains("token")
    {
        return (PolicyCategory::Security, "AGENT");
    }

    // Dependency rules
    if title_lower.contains("dependency") || title_lower.contains("sync") {
        return (PolicyCategory::Dependencies, "BZL");
    }

    // Prohibition rules
    if title_lower.contains("never")
        || title_lower.contains("do not")
        || title_lower.contains("disable")
        || title_lower.contains("skip")
    {
        return (PolicyCategory::Prohibited, "AGENT");
    }

    // Style rules
    if title_lower.contains("format") || title_lower.contains("lint") || title_lower.contains("style") {
        return (PolicyCategory::Style, "AGENT");
    }

    // Environment rules
    if title_lower.contains("wsl") || title_lower.contains("environment") || title_lower.contains("path") {
        return (PolicyCategory::Environment, "AGENT");
    }

    // Default
    (PolicyCategory::Workflow, "AGENT")
}

/// Extract policies from AGENT.md structured format
pub fn extract_agent_md_policies(content: &str, source_path: &str) -> Vec<PolicyRule> {
    let mut rules = vec![];
    let mut rule_counter: HashMap<&str, u32> = HashMap::new();

    // Pattern for emoji rule headers: "## 🏠 Rule: TITLE" or similar
    // Using a simpler pattern that matches ## followed by any emoji-like content and Rule:
    let rule_header_re =
        Regex::new(r"(?m)^##\s+[^\n]*Rule:\s*(.+)$").expect("Invalid regex pattern");

    // Extract rules from headers
    for cap in rule_header_re.captures_iter(content) {
        let title = cap.get(1).map(|m| m.as_str().trim()).unwrap_or("");
        if title.is_empty() {
            continue;
        }

        let match_start = cap.get(0).unwrap().start();
        let line_num = content[..match_start].lines().count() + 1;

        // Determine category and rule ID prefix
        let (category, prefix) = categorize_agent_rule(title);

        // Generate stable rule ID
        let counter = rule_counter.entry(prefix).or_insert(0);
        *counter += 1;
        let rule_id = format!("{}-{:03}", prefix, counter);

        // Extract section content for keyword analysis
        let section_end = content[match_start..]
            .find("\n## ")
            .map(|p| match_start + p)
            .unwrap_or(content.len());
        let section_content = &content[match_start..section_end];

        // Extract keywords from section
        let keywords = extract_keywords(section_content);

        // Generate patterns based on rule type
        let (enforcement, violation) = generate_agent_patterns(title, section_content);

        rules.push(PolicyRule {
            id: rule_id,
            source_file: source_path.to_string(),
            source_line: Some(line_num),
            category,
            description: title.to_string(),
            keywords,
            enforcement_pattern: enforcement,
            violation_pattern: violation,
            severity: determine_severity(section_content),
            original_title: Some(title.to_string()),
        });
    }

    rules
}

/// Generate enforcement and violation patterns for AGENT.md rules
fn generate_agent_patterns(title: &str, content: &str) -> (Option<String>, Option<String>) {
    let title_lower = title.to_lowercase();
    let content_lower = content.to_lowercase();

    // Bazel-specific patterns
    if title_lower.contains("bazel") {
        if title_lower.contains("test") || content_lower.contains("run tests through bazel") {
            return (
                Some(r"bazel test".to_string()),
                Some(r"python -m pytest|pytest\s".to_string()),
            );
        }
        if title_lower.contains("root") || content_lower.contains("from monorepo root") {
            return (
                Some(r"bazel (test|build|run|query)".to_string()),
                Some(r"cd .+ && bazel".to_string()),
            );
        }
        if title_lower.contains("sync") || content_lower.contains("dependency") {
            return (Some(r"bazel sync".to_string()), None);
        }
    }

    // Test rules
    if title_lower.contains("never") && (title_lower.contains("skip") || title_lower.contains("disable")) {
        return (
            None,
            Some(r"--skip|--no-tests|pytest\.mark\.skip|@pytest\.mark\.skip".to_string()),
        );
    }

    (None, None)
}

// ============================================================================
// CLAUDE.md Extraction
// ============================================================================

/// Extract policies from CLAUDE.md (simpler format)
pub fn extract_claude_md_policies(content: &str, source_path: &str) -> Vec<PolicyRule> {
    let mut rules = vec![];
    let mut counter = 0u32;

    // Cargo-specific rules with their patterns
    let cargo_rules: Vec<(&str, PolicyCategory, &str, Option<&str>)> = vec![
        (
            "cargo test",
            PolicyCategory::Testing,
            "Run tests to verify changes",
            None,
        ),
        (
            "cargo clippy",
            PolicyCategory::Style,
            "Run linter for code quality",
            None,
        ),
        (
            "cargo fmt",
            PolicyCategory::Style,
            "Format code consistently",
            None,
        ),
        (
            "cargo check",
            PolicyCategory::Build,
            "Check for compilation errors",
            None,
        ),
        (
            "cargo build",
            PolicyCategory::Build,
            "Build the project",
            None,
        ),
    ];

    for (command, category, description, violation) in cargo_rules {
        if content.contains(command) {
            counter += 1;
            rules.push(PolicyRule {
                id: format!("CLAUDE-{:03}", counter),
                source_file: source_path.to_string(),
                source_line: find_line_number(content, command),
                category,
                description: description.to_string(),
                keywords: extract_keywords_near(content, command),
                enforcement_pattern: Some(regex::escape(command)),
                violation_pattern: violation.map(|s| s.to_string()),
                severity: PolicySeverity::Warning,
                original_title: None,
            });
        }
    }

    // Extract rules from sections with strong keywords
    let keyword_re = Regex::new(r"(?i)(CRITICAL|ALWAYS|NEVER|MUST|DO NOT)[:\s]+([^\n]+)").unwrap();

    for cap in keyword_re.captures_iter(content) {
        let keyword = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        let rule_text = cap.get(2).map(|m| m.as_str().trim()).unwrap_or("");

        if rule_text.is_empty() || rule_text.len() < 10 {
            continue;
        }

        // Skip if we already have a rule for this
        let already_covered = rules.iter().any(|r| {
            r.description
                .to_lowercase()
                .contains(&rule_text.to_lowercase())
        });

        if !already_covered {
            counter += 1;
            let match_start = cap.get(0).unwrap().start();
            rules.push(PolicyRule {
                id: format!("CLAUDE-{:03}", counter),
                source_file: source_path.to_string(),
                source_line: Some(content[..match_start].lines().count() + 1),
                category: categorize_keyword_rule(keyword, rule_text),
                description: rule_text.to_string(),
                keywords: vec![keyword.to_uppercase()],
                enforcement_pattern: None,
                violation_pattern: None,
                severity: determine_severity(keyword),
                original_title: None,
            });
        }
    }

    rules
}

/// Find line number of a needle in content
fn find_line_number(content: &str, needle: &str) -> Option<usize> {
    content
        .find(needle)
        .map(|pos| content[..pos].lines().count() + 1)
}

/// Extract keywords from the section containing a needle
fn extract_keywords_near(content: &str, needle: &str) -> Vec<String> {
    if let Some(pos) = content.find(needle) {
        let start = content[..pos].rfind("\n#").unwrap_or(0);
        let end = content[pos..]
            .find("\n#")
            .map(|p| pos + p)
            .unwrap_or(content.len());
        let section = &content[start..end];
        extract_keywords(section)
    } else {
        vec![]
    }
}

/// Categorize a rule based on keywords
fn categorize_keyword_rule(keyword: &str, text: &str) -> PolicyCategory {
    let text_lower = text.to_lowercase();
    let keyword_upper = keyword.to_uppercase();

    if keyword_upper == "NEVER" || text_lower.contains("forbidden") || text_lower.contains("prohibited") {
        return PolicyCategory::Prohibited;
    }

    if text_lower.contains("test") {
        return PolicyCategory::Testing;
    }
    if text_lower.contains("build") || text_lower.contains("compile") {
        return PolicyCategory::Build;
    }
    if text_lower.contains("security") || text_lower.contains("credential") || text_lower.contains("secret") {
        return PolicyCategory::Security;
    }
    if text_lower.contains("format") || text_lower.contains("lint") || text_lower.contains("style") {
        return PolicyCategory::Style;
    }

    PolicyCategory::Workflow
}

// ============================================================================
// Unified Policy Extractor
// ============================================================================

/// Extract policies from any supported policy file
pub fn extract_policies(content: &str, source_path: &str) -> PolicySet {
    let format = detect_format(content, source_path);
    let filename = std::path::Path::new(source_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");

    let rules = match format {
        PolicyFileFormat::AgentMd => extract_agent_md_policies(content, filename),
        PolicyFileFormat::ClaudeMd => extract_claude_md_policies(content, filename),
    };

    let build_system = match format {
        PolicyFileFormat::AgentMd => Some("bazel".to_string()),
        PolicyFileFormat::ClaudeMd => Some("cargo".to_string()),
    };

    PolicySet {
        format: Some(format),
        source_path: source_path.to_string(),
        rules,
        build_system,
        extracted_at: chrono::Utc::now().to_rfc3339(),
    }
}

/// Discover and extract policies from standard locations
pub fn discover_policies(project_root: &Path) -> Vec<PolicySet> {
    let mut policy_sets = vec![];

    // Check for AGENT.md (monorepo pattern)
    let agent_md = project_root.join("AGENT.md");
    if agent_md.exists() {
        if let Ok(content) = std::fs::read_to_string(&agent_md) {
            policy_sets.push(extract_policies(&content, agent_md.to_str().unwrap_or("AGENT.md")));
        }
    }

    // Check for CLAUDE.md
    let claude_md = project_root.join("CLAUDE.md");
    if claude_md.exists() {
        if let Ok(content) = std::fs::read_to_string(&claude_md) {
            policy_sets.push(extract_policies(&content, claude_md.to_str().unwrap_or("CLAUDE.md")));
        }
    }

    // Check for .claude/ directory with custom policies
    let claude_dir = project_root.join(".claude");
    if claude_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&claude_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map(|e| e == "md").unwrap_or(false) {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        policy_sets.push(extract_policies(&content, path.to_str().unwrap_or("policy.md")));
                    }
                }
            }
        }
    }

    policy_sets
}

// ============================================================================
// Policy Verification
// ============================================================================

/// Verify a plan's instructions against extracted policies
pub fn verify_policies(instructions: &[Instruction], policies: &PolicySet) -> Vec<PolicyViolation> {
    let mut violations = vec![];

    for rule in &policies.rules {
        // Check enforcement patterns (must be present for critical rules)
        if let Some(ref pattern) = rule.enforcement_pattern {
            if let Ok(re) = Regex::new(pattern) {
                let found = instructions.iter().any(|instr| {
                    let params_str = serde_json::to_string(&instr.params).unwrap_or_default();
                    re.is_match(&params_str) || re.is_match(&instr.description)
                });

                // Only flag missing enforcement for critical rules
                if !found && rule.severity == PolicySeverity::Critical {
                    violations.push(PolicyViolation {
                        rule_id: rule.id.clone(),
                        instruction_id: None,
                        message: format!("Missing required action: {}", rule.description),
                        severity: rule.severity.clone(),
                    });
                }
            }
        }

        // Check violation patterns (must NOT be present)
        if let Some(ref pattern) = rule.violation_pattern {
            if let Ok(re) = Regex::new(pattern) {
                for instr in instructions {
                    let params_str = serde_json::to_string(&instr.params).unwrap_or_default();
                    if re.is_match(&params_str) || re.is_match(&instr.description) {
                        violations.push(PolicyViolation {
                            rule_id: rule.id.clone(),
                            instruction_id: Some(instr.id.clone()),
                            message: format!("Forbidden action detected: {}", rule.description),
                            severity: rule.severity.clone(),
                        });
                    }
                }
            }
        }
    }

    violations
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::OpCode;

    #[test]
    fn test_claude_md_format_detection() {
        let content = "# Build Commands\ncargo test\ncargo clippy";
        assert_eq!(
            detect_format(content, "CLAUDE.md"),
            PolicyFileFormat::ClaudeMd
        );
    }

    #[test]
    fn test_agent_md_format_detection_by_filename() {
        let content = "Some content";
        assert_eq!(
            detect_format(content, "AGENT.md"),
            PolicyFileFormat::AgentMd
        );
    }

    #[test]
    fn test_agent_md_format_detection_by_content() {
        let content = "## 🏠 Rule: ALWAYS Run Bazel Commands\nbazel test //...";
        assert_eq!(
            detect_format(content, "policy.md"),
            PolicyFileFormat::AgentMd
        );
    }

    #[test]
    fn test_claude_md_extraction() {
        let content = r#"
# Build and Development Commands

## Running Tests

```bash
# Run tests
cargo test

# Check for errors
cargo check

# Format code
cargo fmt
```

## Important Notes

ALWAYS run tests before committing.
NEVER commit untested code.
"#;
        let policies = extract_policies(content, "CLAUDE.md");

        assert_eq!(policies.format, Some(PolicyFileFormat::ClaudeMd));
        assert!(!policies.rules.is_empty());

        // Should have rules for cargo commands
        let rule_ids: Vec<&str> = policies.rules.iter().map(|r| r.id.as_str()).collect();
        assert!(rule_ids.iter().any(|id| id.starts_with("CLAUDE-")));
    }

    #[test]
    fn test_agent_md_emoji_header_extraction() {
        let content = r#"
# Agent Guidelines

## 🏠 Rule: ALWAYS Run Bazel Commands from Monorepo Root

**CRITICAL: ALL Bazel commands MUST be executed from the monorepo root directory.**

### ✅ ALWAYS DO THIS:
```bash
bazel test //services/...
```

### ❌ DO NOT DO THIS:
```bash
cd services && bazel test //...
```

## 🔧 Rule: NEVER Skip Tests

Tests must always pass before merging.
"#;
        let policies = extract_policies(content, "AGENT.md");

        assert_eq!(policies.format, Some(PolicyFileFormat::AgentMd));
        assert!(!policies.rules.is_empty());

        // Should detect Bazel rule
        let bzl_rules: Vec<_> = policies
            .rules
            .iter()
            .filter(|r| r.id.starts_with("BZL-"))
            .collect();
        assert!(!bzl_rules.is_empty(), "Should extract BZL rules");

        // Check keywords
        assert!(policies.rules[0].keywords.contains(&"CRITICAL".to_string())
            || policies.rules[0].keywords.contains(&"ALWAYS".to_string()));
    }

    #[test]
    fn test_agent_md_bazel_patterns() {
        let content = r#"
## 🏠 Rule: ALWAYS Run Tests Through Bazel

**CRITICAL: NEVER run pytest directly. ALL tests MUST go through Bazel.**

```bash
# Correct
bazel test //...

# Wrong - DO NOT DO THIS
python -m pytest tests/
```
"#;
        let policies = extract_policies(content, "AGENT.md");

        // Should have violation pattern for pytest
        let has_pytest_violation = policies.rules.iter().any(|r| {
            r.violation_pattern
                .as_ref()
                .map(|p| p.contains("pytest"))
                .unwrap_or(false)
        });
        assert!(
            has_pytest_violation,
            "Should have violation pattern for pytest"
        );
    }

    #[test]
    fn test_violation_detection() {
        let policies = PolicySet {
            format: Some(PolicyFileFormat::AgentMd),
            source_path: "AGENT.md".to_string(),
            rules: vec![PolicyRule {
                id: "BZL-001".to_string(),
                source_file: "AGENT.md".to_string(),
                source_line: Some(10),
                category: PolicyCategory::Testing,
                description: "Run tests through Bazel".to_string(),
                keywords: vec!["CRITICAL".to_string()],
                enforcement_pattern: Some(r"bazel test".to_string()),
                violation_pattern: Some(r"python -m pytest".to_string()),
                severity: PolicySeverity::Critical,
                original_title: Some("ALWAYS Run Tests Through Bazel".to_string()),
            }],
            build_system: Some("bazel".to_string()),
            extracted_at: "2024-01-01T00:00:00Z".to_string(),
        };

        let instructions = vec![Instruction {
            id: "test_step".to_string(),
            op: OpCode::RunCommand,
            params: serde_json::json!({"command": "python -m pytest tests/"}),
            dependencies: vec![],
            description: "Run tests".to_string(),
            ..Default::default()
        }];

        let violations = verify_policies(&instructions, &policies);
        assert!(!violations.is_empty());
        assert_eq!(violations[0].rule_id, "BZL-001");
    }

    #[test]
    fn test_enforcement_pattern_detection() {
        let policies = PolicySet {
            format: Some(PolicyFileFormat::ClaudeMd),
            source_path: "CLAUDE.md".to_string(),
            rules: vec![PolicyRule {
                id: "CLAUDE-001".to_string(),
                source_file: "CLAUDE.md".to_string(),
                source_line: Some(5),
                category: PolicyCategory::Testing,
                description: "Run cargo test".to_string(),
                keywords: vec!["ALWAYS".to_string()],
                enforcement_pattern: Some(r"cargo test".to_string()),
                violation_pattern: None,
                severity: PolicySeverity::Critical,
                original_title: None,
            }],
            build_system: Some("cargo".to_string()),
            extracted_at: "2024-01-01T00:00:00Z".to_string(),
        };

        // Plan without cargo test should fail
        let instructions = vec![Instruction {
            id: "build".to_string(),
            op: OpCode::RunCommand,
            params: serde_json::json!({"command": "cargo build"}),
            dependencies: vec![],
            description: "Build project".to_string(),
            ..Default::default()
        }];

        let violations = verify_policies(&instructions, &policies);
        assert!(!violations.is_empty(), "Should detect missing cargo test");
        assert!(violations[0].message.contains("Missing required action"));
    }

    #[test]
    fn test_no_violations_when_compliant() {
        let policies = PolicySet {
            format: Some(PolicyFileFormat::ClaudeMd),
            source_path: "CLAUDE.md".to_string(),
            rules: vec![PolicyRule {
                id: "CLAUDE-001".to_string(),
                source_file: "CLAUDE.md".to_string(),
                source_line: Some(5),
                category: PolicyCategory::Testing,
                description: "Run cargo test".to_string(),
                keywords: vec!["ALWAYS".to_string()],
                enforcement_pattern: Some(r"cargo test".to_string()),
                violation_pattern: None,
                severity: PolicySeverity::Critical,
                original_title: None,
            }],
            build_system: Some("cargo".to_string()),
            extracted_at: "2024-01-01T00:00:00Z".to_string(),
        };

        let instructions = vec![
            Instruction {
                id: "build".to_string(),
                op: OpCode::RunCommand,
                params: serde_json::json!({"command": "cargo build"}),
                dependencies: vec![],
                description: "Build project".to_string(),
                ..Default::default()
            },
            Instruction {
                id: "test".to_string(),
                op: OpCode::RunTest,
                params: serde_json::json!({"command": "cargo test"}),
                dependencies: vec!["build".to_string()],
                description: "Run cargo test".to_string(),
                ..Default::default()
            },
        ];

        let violations = verify_policies(&instructions, &policies);
        assert!(violations.is_empty(), "Should not have violations when compliant");
    }

    #[test]
    fn test_keyword_extraction() {
        let text = "CRITICAL: This is important. ALWAYS do this. NEVER skip it.";
        let keywords = extract_keywords(text);

        assert!(keywords.contains(&"CRITICAL".to_string()));
        assert!(keywords.contains(&"ALWAYS".to_string()));
        assert!(keywords.contains(&"NEVER".to_string()));
    }

    #[test]
    fn test_severity_determination() {
        assert_eq!(determine_severity("CRITICAL rule"), PolicySeverity::Critical);
        assert_eq!(determine_severity("NEVER do this"), PolicySeverity::Critical);
        assert_eq!(determine_severity("You should do this"), PolicySeverity::Warning);
        assert_eq!(determine_severity("General info"), PolicySeverity::Info);
    }

    #[test]
    fn test_policy_rule_default() {
        let rule = PolicyRule::default();
        assert!(rule.id.is_empty());
        assert_eq!(rule.severity, PolicySeverity::Warning);
        assert_eq!(rule.category, PolicyCategory::Workflow);
    }

    #[test]
    fn test_policy_set_default() {
        let set = PolicySet::default();
        assert!(set.format.is_none());
        assert!(set.rules.is_empty());
        assert!(set.build_system.is_none());
    }
}
