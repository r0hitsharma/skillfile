//! Shared SKILL.md preview rendering for both TUI modules.
//!
//! Single source of truth for frontmatter parsing, risk icons,
//! markdown line styling, and skill content line building.

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

// ===========================================================================
// Types
// ===========================================================================

/// Parsed SKILL.md frontmatter + body excerpt for the preview pane.
#[derive(Debug, Clone)]
pub struct PreviewContent {
    pub name: Option<String>,
    pub description: Option<String>,
    pub risk: Option<String>,
    pub source: Option<String>,
    pub body_excerpt: Option<String>,
}

// ===========================================================================
// Parsing
// ===========================================================================

/// Parse SKILL.md frontmatter and extract a body excerpt.
///
/// Lightweight: splits on `---` markers, matches `key: value` lines.
/// No YAML crate needed.
pub fn parse_skill_frontmatter(content: &str) -> PreviewContent {
    let trimmed = content.trim_start();

    if let Some(after_opening) = trimmed.strip_prefix("---") {
        if let Some(end) = after_opening.find("\n---") {
            let frontmatter = &after_opening[..end];
            let mut content = parse_frontmatter_fields(frontmatter);
            let body_start = end + 4; // skip \n---
            content.body_excerpt = extract_body_excerpt(&after_opening[body_start..]);
            return content;
        }
    }

    // No frontmatter — treat entire content as body excerpt.
    let body_excerpt = if content.trim().is_empty() {
        None
    } else {
        extract_body_excerpt(trimmed)
    };

    PreviewContent {
        name: None,
        description: None,
        risk: None,
        source: None,
        body_excerpt,
    }
}

/// Parse key-value pairs from frontmatter text into a `PreviewContent` (no body).
fn parse_frontmatter_fields(frontmatter: &str) -> PreviewContent {
    let mut content = PreviewContent {
        name: None,
        description: None,
        risk: None,
        source: None,
        body_excerpt: None,
    };
    for line in frontmatter.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim().to_string();
        match key.trim().to_lowercase().as_str() {
            "name" => content.name = Some(value),
            "description" => content.description = Some(value),
            "risk" => content.risk = Some(value),
            "source" => content.source = Some(value),
            _ => {}
        }
    }
    content
}

/// Extract a body excerpt (first 20 lines) from content after frontmatter.
fn extract_body_excerpt(body: &str) -> Option<String> {
    let body = body.trim_start();
    if body.is_empty() {
        return None;
    }
    Some(body.lines().take(20).collect::<Vec<_>>().join("\n"))
}

// ===========================================================================
// Rendering
// ===========================================================================

/// Horizontal rule for separating metadata from body content.
pub(super) const PREVIEW_HR: &str =
    "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}";

/// Map a risk level to an icon and color.
pub(super) fn risk_icon(risk: &str) -> (&'static str, Color) {
    match risk.to_lowercase().as_str() {
        "low" => ("\u{2713}", Color::Green),     // ✓
        "medium" => ("\u{26a0}", Color::Yellow), // ⚠
        "high" => ("\u{2717}", Color::Red),      // ✗
        _ => ("\u{2022}", Color::White),         // •
    }
}

/// Apply basic markdown styling to a single line for the preview pane.
///
/// Recognizes: `#`..`####` headings (stripped), `- ` / `* ` list items
/// (gray bullet), ``` ``` code fences (dark gray), and `---` horizontal rules.
pub(super) fn style_markdown_line(line: &str) -> Line<'static> {
    let trimmed = line.trim_start();
    let heading_style = |level: usize| {
        let color = if level == 1 { Color::Cyan } else { Color::Blue };
        Style::default().fg(color).add_modifier(Modifier::BOLD)
    };

    if let Some(text) = trimmed.strip_prefix("#### ") {
        Line::from(Span::styled(text.to_string(), heading_style(4)))
    } else if let Some(text) = trimmed.strip_prefix("### ") {
        Line::from(Span::styled(text.to_string(), heading_style(3)))
    } else if let Some(text) = trimmed.strip_prefix("## ") {
        Line::from(Span::styled(text.to_string(), heading_style(2)))
    } else if let Some(text) = trimmed.strip_prefix("# ") {
        Line::from(Span::styled(text.to_string(), heading_style(1)))
    } else if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
        let indent = line.len() - trimmed.len();
        let prefix = " ".repeat(indent);
        Line::from(vec![
            Span::raw(prefix),
            Span::styled("  \u{2022} ", Style::default().fg(Color::DarkGray)),
            Span::raw(trimmed[2..].to_string()),
        ])
    } else if trimmed.starts_with("```") {
        Line::from(Span::styled(
            line.to_string(),
            Style::default().fg(Color::DarkGray),
        ))
    } else if trimmed == "---" {
        Line::from(Span::styled(
            PREVIEW_HR,
            Style::default().fg(Color::DarkGray),
        ))
    } else {
        Line::from(line.to_string())
    }
}

/// Build styled lines for SKILL.md content (frontmatter metadata + body).
///
/// Renders: Name, Description, Risk (with icon), Source, HR, Body (styled).
/// Shows "No metadata available." when all fields are None.
pub(super) fn build_skill_content_lines(content: &PreviewContent) -> Vec<Line<'static>> {
    let label_style = Style::default().fg(Color::DarkGray);
    let mut lines: Vec<Line<'static>> = Vec::new();

    if let Some(name) = &content.name {
        lines.push(Line::from(vec![
            Span::styled("Name:        ", label_style),
            Span::styled(name.clone(), Style::default().add_modifier(Modifier::BOLD)),
        ]));
    }
    if let Some(desc) = &content.description {
        lines.push(Line::from(vec![
            Span::styled("Description: ", label_style),
            Span::raw(desc.clone()),
        ]));
    }
    if let Some(risk) = &content.risk {
        let (icon, color) = risk_icon(risk);
        lines.push(Line::from(vec![
            Span::styled("Risk:        ", label_style),
            Span::styled(
                format!("{icon} {risk}"),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
        ]));
    }
    if let Some(source) = &content.source {
        lines.push(Line::from(vec![
            Span::styled("Source:      ", label_style),
            Span::styled(source.clone(), Style::default().fg(Color::Magenta)),
        ]));
    }
    append_body_and_fallback(&mut lines, content);
    lines
}

/// Append body excerpt (with HR separator) or fallback message.
fn append_body_and_fallback(lines: &mut Vec<Line<'static>>, content: &PreviewContent) {
    if let Some(body) = &content.body_excerpt {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            PREVIEW_HR,
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(""));
        lines.extend(body.lines().map(style_markdown_line));
    }
    if content.name.is_none() && content.description.is_none() && content.body_excerpt.is_none() {
        lines.push(Line::from(Span::styled(
            "No metadata available.",
            Style::default().fg(Color::DarkGray),
        )));
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -- Frontmatter parsing tests (moved from add_tui) -----------------------

    #[test]
    fn parse_frontmatter_full() {
        let content = "\
---
name: Browser Automation
description: Automate web browsing tasks
risk: medium
source: community
---

## Use this skill when
- You need to interact with web pages
";
        let preview = parse_skill_frontmatter(content);
        assert_eq!(preview.name.as_deref(), Some("Browser Automation"));
        assert_eq!(
            preview.description.as_deref(),
            Some("Automate web browsing tasks")
        );
        assert_eq!(preview.risk.as_deref(), Some("medium"));
        assert_eq!(preview.source.as_deref(), Some("community"));
        assert!(preview.body_excerpt.is_some());
        assert!(preview.body_excerpt.unwrap().contains("Use this skill"));
    }

    #[test]
    fn parse_frontmatter_missing_fields() {
        let content = "\
---
name: Simple Skill
---

Some body text.
";
        let preview = parse_skill_frontmatter(content);
        assert_eq!(preview.name.as_deref(), Some("Simple Skill"));
        assert!(preview.description.is_none());
        assert!(preview.risk.is_none());
        assert!(preview.source.is_none());
        assert!(preview.body_excerpt.is_some());
    }

    #[test]
    fn parse_frontmatter_no_frontmatter() {
        let content = "# Just a heading\n\nSome body text.\n";
        let preview = parse_skill_frontmatter(content);
        assert!(preview.name.is_none());
        assert!(preview.body_excerpt.is_some());
        assert!(preview.body_excerpt.unwrap().contains("Just a heading"));
    }

    #[test]
    fn parse_frontmatter_empty_content() {
        let preview = parse_skill_frontmatter("");
        assert!(preview.name.is_none());
        assert!(preview.body_excerpt.is_none());
    }

    #[test]
    fn parse_frontmatter_only_whitespace() {
        let preview = parse_skill_frontmatter("   \n  \n  ");
        assert!(preview.name.is_none());
        assert!(preview.body_excerpt.is_none());
    }

    #[test]
    fn parse_frontmatter_body_truncated_to_20_lines() {
        use std::fmt::Write as _;
        let mut content = "---\nname: Test\n---\n".to_string();
        for i in 0..30 {
            let _ = writeln!(content, "Line {i}");
        }
        let preview = parse_skill_frontmatter(&content);
        let body = preview.body_excerpt.unwrap();
        let line_count = body.lines().count();
        assert_eq!(line_count, 20);
    }

    // -- Risk icon tests (moved from add_tui) ---------------------------------

    #[test]
    fn risk_icon_mapping() {
        assert_eq!(risk_icon("low"), ("\u{2713}", Color::Green));
        assert_eq!(risk_icon("medium"), ("\u{26a0}", Color::Yellow));
        assert_eq!(risk_icon("high"), ("\u{2717}", Color::Red));
        assert_eq!(risk_icon("unknown"), ("\u{2022}", Color::White));
    }

    #[test]
    fn risk_icon_case_insensitive() {
        assert_eq!(risk_icon("LOW").1, Color::Green);
        assert_eq!(risk_icon("Medium").1, Color::Yellow);
        assert_eq!(risk_icon("HIGH").1, Color::Red);
    }

    // -- style_markdown_line tests ---------------------------------------------

    #[test]
    fn style_h1_cyan_bold_stripped() {
        let line = style_markdown_line("# Heading One");
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "Heading One");
        assert_eq!(line.spans[0].style.fg, Some(Color::Cyan));
    }

    #[test]
    fn style_h2_blue_bold_stripped() {
        let line = style_markdown_line("## Heading Two");
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "Heading Two");
        assert_eq!(line.spans[0].style.fg, Some(Color::Blue));
    }

    #[test]
    fn style_h3_blue_bold_stripped() {
        let line = style_markdown_line("### Heading Three");
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "Heading Three");
        assert_eq!(line.spans[0].style.fg, Some(Color::Blue));
    }

    #[test]
    fn style_h4_blue_bold_stripped() {
        let line = style_markdown_line("#### Heading Four");
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "Heading Four");
        assert_eq!(line.spans[0].style.fg, Some(Color::Blue));
    }

    #[test]
    fn style_list_bullet_gray_dot() {
        let line = style_markdown_line("- List item");
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains('\u{2022}'));
        assert!(text.contains("List item"));
    }

    #[test]
    fn style_code_fence_dark_gray() {
        let line = style_markdown_line("```rust");
        assert_eq!(line.spans[0].style.fg, Some(Color::DarkGray));
    }

    #[test]
    fn style_hr_becomes_preview_hr() {
        let line = style_markdown_line("---");
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, PREVIEW_HR);
    }

    #[test]
    fn style_plain_text_unchanged() {
        let line = style_markdown_line("Just some text");
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "Just some text");
    }

    #[test]
    fn style_indented_list_preserves_indent() {
        let line = style_markdown_line("  - Indented item");
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.starts_with("  "));
        assert!(text.contains("Indented item"));
    }

    // -- build_skill_content_lines tests ---------------------------------------

    #[test]
    fn content_lines_full_metadata() {
        let content = PreviewContent {
            name: Some("Test Skill".to_string()),
            description: Some("A test skill".to_string()),
            risk: Some("low".to_string()),
            source: Some("community".to_string()),
            body_excerpt: Some("## Usage\n- Step one".to_string()),
        };
        let lines = build_skill_content_lines(&content);
        let text: String = lines
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Test Skill"));
        assert!(text.contains("A test skill"));
        assert!(text.contains("\u{2713} low"));
        assert!(text.contains("community"));
        assert!(text.contains("Usage"));
        assert!(text.contains("Step one"));
    }

    #[test]
    fn content_lines_no_metadata_fallback() {
        let content = PreviewContent {
            name: None,
            description: None,
            risk: None,
            source: None,
            body_excerpt: None,
        };
        let lines = build_skill_content_lines(&content);
        let text: String = lines
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("No metadata available."));
    }

    #[test]
    fn content_lines_name_only() {
        let content = PreviewContent {
            name: Some("Just a Name".to_string()),
            description: None,
            risk: None,
            source: None,
            body_excerpt: None,
        };
        let lines = build_skill_content_lines(&content);
        let text: String = lines
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Just a Name"));
        assert!(!text.contains("No metadata available."));
    }

    #[test]
    fn content_lines_body_only() {
        let content = PreviewContent {
            name: None,
            description: None,
            risk: None,
            source: None,
            body_excerpt: Some("# Title\nSome body text".to_string()),
        };
        let lines = build_skill_content_lines(&content);
        let text: String = lines
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Title"));
        assert!(!text.contains("No metadata available."));
    }

    #[test]
    fn content_lines_risk_with_icon() {
        let content = PreviewContent {
            name: None,
            description: None,
            risk: Some("high".to_string()),
            source: None,
            body_excerpt: None,
        };
        let lines = build_skill_content_lines(&content);
        let text: String = lines
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("\u{2717} high"));
    }
}
