// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Cilium-identical ANSI palette and status-summary helpers.
//! Raw escapes only — do not route through `tui::colors::cli` (✓/✗).

pub const RED: &str = "\x1b[31m";
pub const YELLOW: &str = "\x1b[33m";
pub const BLUE: &str = "\x1b[34m";
pub const GREEN: &str = "\x1b[32m";
pub const MAGENTA: &str = "\x1b[35m";
pub const CYAN: &str = "\x1b[36m";
pub const RESET: &str = "\x1b[0m";

/// Aggregated error/warning/disabled state for one banner component.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ErrorCount {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub disabled: bool,
}

impl ErrorCount {
    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if !self.errors.is_empty() {
            parts.push(format!("❌ {}{} errors{}", RED, self.errors.len(), RESET));
        }
        if !self.warnings.is_empty() {
            parts.push(format!(
                "⚠️  {}{} warnings{}",
                YELLOW,
                self.warnings.len(),
                RESET
            ));
        }
        if self.disabled {
            parts.push(format!("ℹ️  {}disabled{}", CYAN, RESET));
        }
        if parts.is_empty() {
            format!("✅ {}OK{}", GREEN, RESET)
        } else {
            parts.join(", ")
        }
    }
}

/// Strip ANSI CSI sequences for tests / stable snapshots.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for ch in chars.by_ref() {
                    if ch.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_ok_when_empty() {
        assert_eq!(strip_ansi(&ErrorCount::default().summary()), "✅ OK");
    }

    #[test]
    fn summary_disabled() {
        let e = ErrorCount {
            disabled: true,
            ..Default::default()
        };
        assert_eq!(strip_ansi(&e.summary()), "ℹ️  disabled");
    }

    #[test]
    fn strip_ansi_removes_colors() {
        let s = format!("{}OK{}", GREEN, RESET);
        assert_eq!(strip_ansi(&s), "OK");
    }

    #[test]
    fn summary_errors_and_warnings() {
        let e = ErrorCount {
            errors: vec!["a".into()],
            warnings: vec!["b".into()],
            disabled: false,
        };
        let text = strip_ansi(&e.summary());
        assert!(text.contains("❌ 1 errors"));
        assert!(text.contains("⚠️  1 warnings"));
    }
}
