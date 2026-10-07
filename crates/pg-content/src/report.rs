//! Validation reports (Blueprint §12.3, §17): structured errors and warnings with stable codes.

use pg_canon::{Canon, ToCanon};
use std::fmt;

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
}

/// One finding. `code` is stable and machine-readable; `path` locates it (`file`, `template.field`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub severity: Severity,
    pub code: &'static str,
    pub path: String,
    pub message: String,
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sev = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        if self.path.is_empty() {
            write!(f, "{sev}[{}]: {}", self.code, self.message)
        } else {
            write!(f, "{sev}[{}] {}: {}", self.code, self.path, self.message)
        }
    }
}

/// An ordered collection of issues. Errors block use of the content; warnings do not.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ValidationReport {
    issues: Vec<Issue>,
}

impl ValidationReport {
    pub fn new() -> ValidationReport {
        ValidationReport::default()
    }

    pub fn error(
        &mut self,
        code: &'static str,
        path: impl Into<String>,
        message: impl Into<String>,
    ) {
        self.issues.push(Issue {
            severity: Severity::Error,
            code,
            path: path.into(),
            message: message.into(),
        });
    }

    pub fn warn(
        &mut self,
        code: &'static str,
        path: impl Into<String>,
        message: impl Into<String>,
    ) {
        self.issues.push(Issue {
            severity: Severity::Warning,
            code,
            path: path.into(),
            message: message.into(),
        });
    }

    pub fn merge(&mut self, other: ValidationReport) {
        self.issues.extend(other.issues);
    }

    pub fn issues(&self) -> &[Issue] {
        &self.issues
    }

    pub fn errors(&self) -> impl Iterator<Item = &Issue> {
        self.issues.iter().filter(|i| i.severity == Severity::Error)
    }

    pub fn warnings(&self) -> impl Iterator<Item = &Issue> {
        self.issues
            .iter()
            .filter(|i| i.severity == Severity::Warning)
    }

    pub fn error_count(&self) -> usize {
        self.errors().count()
    }

    /// No errors (warnings are allowed).
    pub fn is_ok(&self) -> bool {
        self.error_count() == 0
    }

    pub fn is_empty(&self) -> bool {
        self.issues.is_empty()
    }

    /// Whether any issue carries `code`.
    pub fn has_code(&self, code: &str) -> bool {
        self.issues.iter().any(|i| i.code == code)
    }
}

impl fmt::Display for ValidationReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, issue) in self.issues.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{issue}")?;
        }
        Ok(())
    }
}

impl ToCanon for ValidationReport {
    fn to_canon(&self) -> Canon {
        Canon::map([
            ("errors", Canon::Int(self.error_count() as i128)),
            ("warnings", Canon::Int(self.warnings().count() as i128)),
            (
                "issues",
                Canon::List(
                    self.issues
                        .iter()
                        .map(|i| {
                            Canon::map([
                                (
                                    "severity",
                                    Canon::str(if i.severity == Severity::Error {
                                        "error"
                                    } else {
                                        "warning"
                                    }),
                                ),
                                ("code", Canon::str(i.code)),
                                ("path", Canon::str(i.path.clone())),
                                ("message", Canon::str(i.message.clone())),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_block_warnings_do_not() {
        let mut r = ValidationReport::new();
        assert!(r.is_ok() && r.is_empty());
        r.warn("w1", "a", "careful");
        assert!(r.is_ok() && !r.is_empty());
        r.error("e1", "b.c", "broken");
        assert!(!r.is_ok());
        assert_eq!((r.error_count(), r.warnings().count()), (1, 1));
        assert!(r.has_code("w1") && r.has_code("e1") && !r.has_code("zzz"));
    }

    #[test]
    fn display_is_one_line_per_issue() {
        let mut r = ValidationReport::new();
        r.error(
            "bad_value",
            "furniture.drawer.value.base",
            "must be 0..=100",
        );
        r.warn("note", "", "no path");
        assert_eq!(
            r.to_string(),
            "error[bad_value] furniture.drawer.value.base: must be 0..=100\nwarning[note]: no path"
        );
    }

    #[test]
    fn merge_keeps_order() {
        let mut a = ValidationReport::new();
        a.error("first", "", "");
        let mut b = ValidationReport::new();
        b.error("second", "", "");
        a.merge(b);
        assert_eq!(
            a.issues().iter().map(|i| i.code).collect::<Vec<_>>(),
            ["first", "second"]
        );
    }
}
