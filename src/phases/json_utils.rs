//! JSON extraction utilities for parsing LLM responses.

/// Extract a JSON block from LLM response text.
///
/// Handles two common patterns:
/// 1. JSON wrapped in ```json ... ``` code blocks
/// 2. Raw JSON objects (finds first { to last })
pub fn extract_json_block(text: &str) -> Option<&str> {
    // Look for ```json ... ``` blocks
    if let Some(start) = text.find("```json") {
        let content_start = start + 7;
        if let Some(end) = text[content_start..].find("```") {
            return Some(text[content_start..content_start + end].trim());
        }
    }

    // Try finding raw JSON object
    if let Some(start) = text.find('{')
        && let Some(end) = text.rfind('}')
    {
        return Some(&text[start..=end]);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json_from_code_block() {
        let text = r#"Here's the response:
```json
{"key": "value"}
```
"#;
        assert_eq!(extract_json_block(text), Some(r#"{"key": "value"}"#));
    }

    #[test]
    fn test_extract_raw_json() {
        let text = r#"Some text before {"key": "value"} and after"#;
        assert_eq!(extract_json_block(text), Some(r#"{"key": "value"}"#));
    }

    #[test]
    fn test_extract_nested_json() {
        let text = r#"{"outer": {"inner": "value"}}"#;
        assert_eq!(
            extract_json_block(text),
            Some(r#"{"outer": {"inner": "value"}}"#)
        );
    }

    #[test]
    fn test_no_json() {
        let text = "Just plain text without any JSON";
        assert_eq!(extract_json_block(text), None);
    }
}
