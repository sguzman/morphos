use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DiagnosticCode(pub String);

impl DiagnosticCode {
    pub fn source_parse() -> Self {
        Self("MORPHOS_SOURCE_PARSE".to_owned())
    }
    pub fn unsupported_schema_version() -> Self {
        Self("MORPHOS_UNSUPPORTED_SCHEMA_VERSION".to_owned())
    }
    pub fn missing_field() -> Self {
        Self("MORPHOS_MISSING_FIELD".to_owned())
    }
    pub fn unknown_field() -> Self {
        Self("MORPHOS_UNKNOWN_FIELD".to_owned())
    }
    pub fn invalid_id() -> Self {
        Self("MORPHOS_INVALID_ID".to_owned())
    }
    pub fn invalid_value() -> Self {
        Self("MORPHOS_INVALID_VALUE".to_owned())
    }
    pub fn broken_node_reference() -> Self {
        Self("MORPHOS_BROKEN_NODE_REFERENCE".to_owned())
    }
    pub fn broken_parameter_reference() -> Self {
        Self("MORPHOS_BROKEN_PARAMETER_REFERENCE".to_owned())
    }
    pub fn invalid_root() -> Self {
        Self("MORPHOS_INVALID_ROOT".to_owned())
    }
    pub fn dependency_cycle() -> Self {
        Self("MORPHOS_DEPENDENCY_CYCLE".to_owned())
    }
    pub fn unknown_output() -> Self {
        Self("MORPHOS_UNKNOWN_OUTPUT".to_owned())
    }
    pub fn unknown_parameter() -> Self {
        Self("MORPHOS_UNKNOWN_PARAMETER".to_owned())
    }
    pub fn nonfinite_value() -> Self {
        Self("MORPHOS_NONFINITE_VALUE".to_owned())
    }
    pub fn invalid_scale() -> Self {
        Self("MORPHOS_INVALID_SCALE".to_owned())
    }
    pub fn invalid_primitive() -> Self {
        Self("MORPHOS_INVALID_PRIMITIVE".to_owned())
    }
    pub fn invalid_composition() -> Self {
        Self("MORPHOS_INVALID_COMPOSITION".to_owned())
    }
    pub fn unsupported_geometry() -> Self {
        Self("MORPHOS_UNSUPPORTED_GEOMETRY".to_owned())
    }
    pub fn invalid_mesh() -> Self {
        Self("MORPHOS_INVALID_MESH".to_owned())
    }
    pub fn empty_geometry() -> Self {
        Self("MORPHOS_EMPTY_GEOMETRY".to_owned())
    }
    pub fn geometry_backend() -> Self {
        Self("MORPHOS_GEOMETRY_BACKEND".to_owned())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticSourceSpan {
    pub start: usize,
    pub end: usize,
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticSourceLocation {
    pub path: Option<String>,
    pub span: Option<DiagnosticSourceSpan>,
    pub byte_offset: Option<usize>,
    pub line: Option<usize>,
    pub column: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticTiming {
    pub parse_millis: Option<u64>,
    pub validation_millis: Option<u64>,
    pub evaluation_millis: Option<u64>,
    pub total_millis: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub severity: DiagnosticSeverity,
    pub code: DiagnosticCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<DiagnosticSourceLocation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameter_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
    pub blocking: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<DiagnosticTiming>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub context: BTreeMap<String, String>,
}

impl Diagnostic {
    pub fn error(code: DiagnosticCode, message: impl Into<String>) -> Self {
        Self {
            severity: DiagnosticSeverity::Error,
            code,
            message: message.into(),
            source: None,
            node_id: None,
            parameter_id: None,
            notes: Vec::new(),
            remediation: None,
            blocking: true,
            telemetry: None,
            context: BTreeMap::new(),
        }
    }

    pub fn with_source(mut self, source: DiagnosticSourceLocation) -> Self {
        self.source = Some(source);
        self
    }

    pub fn with_node_id(mut self, node_id: impl Into<String>) -> Self {
        self.node_id = Some(node_id.into());
        self
    }

    pub fn with_parameter_id(mut self, parameter_id: impl Into<String>) -> Self {
        self.parameter_id = Some(parameter_id.into());
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    pub fn with_remediation(mut self, remediation: impl Into<String>) -> Self {
        self.remediation = Some(remediation.into());
        self
    }

    pub fn with_context(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.context.insert(key.into(), value.into());
        self
    }

    pub fn with_telemetry(mut self, telemetry: DiagnosticTiming) -> Self {
        self.telemetry = Some(telemetry);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticReport {
    pub diagnostics: Vec<Diagnostic>,
}

impl DiagnosticReport {
    pub fn new(diagnostics: Vec<Diagnostic>) -> Self {
        Self { diagnostics }
    }

    pub fn is_empty(&self) -> bool {
        self.diagnostics.is_empty()
    }

    pub fn has_blocking(&self) -> bool {
        self.diagnostics.iter().any(|diagnostic| diagnostic.blocking)
    }

    pub fn primary_message(&self) -> Option<&str> {
        self.diagnostics.first().map(|diagnostic| diagnostic.message.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_round_trips_through_json() {
        let diagnostic = Diagnostic::error(DiagnosticCode::invalid_value(), "bad value")
            .with_node_id("box_a")
            .with_parameter_id("size")
            .with_note("value must be finite")
            .with_remediation("replace NaN with a finite scalar")
            .with_context("stage", "scene")
            .with_telemetry(DiagnosticTiming {
                parse_millis: Some(2),
                validation_millis: Some(4),
                evaluation_millis: None,
                total_millis: Some(6),
            })
            .with_source(DiagnosticSourceLocation {
                path: Some("source/scene.toml".to_owned()),
                span: Some(DiagnosticSourceSpan {
                    start: 1,
                    end: 4,
                    start_line: 1,
                    start_column: 2,
                    end_line: 1,
                    end_column: 5,
                }),
                byte_offset: Some(1),
                line: Some(1),
                column: Some(2),
            });

        let report = DiagnosticReport::new(vec![diagnostic.clone()]);
        let encoded = serde_json::to_string(&report).expect("serialize");
        let decoded: DiagnosticReport = serde_json::from_str(&encoded).expect("deserialize");
        assert_eq!(decoded, report);
        assert!(decoded.has_blocking());
        assert_eq!(decoded.primary_message(), Some("bad value"));
    }
}
