use serde::Serialize;

/// Aggregated report from all tools.
#[derive(Debug, Serialize)]
pub struct AggregatedReport {
    pub tool: String,
    pub version: String,
    pub total_findings: usize,
    pub by_severity: SeverityCounts,
    pub by_tool: ToolCounts,
    pub findings: Vec<ai_tools_core::finding::Finding>,
}

#[derive(Debug, Default, Serialize)]
pub struct SeverityCounts {
    pub critical: usize,
    pub high: usize,
    pub medium: usize,
    pub low: usize,
}

#[derive(Debug, Default, Serialize)]
pub struct ToolCounts {
    pub diff_risk: usize,
    pub cargo_impact: usize,
    pub spec_drift: usize,
}

impl AggregatedReport {
    pub fn new(
        tool: &str,
        findings: &[ai_tools_core::finding::Finding],
    ) -> Self {
        let mut counts = SeverityCounts::default();
        let mut tool_counts = ToolCounts::default();

        for f in findings {
            match f.severity {
                ai_tools_core::finding::Severity::Critical => counts.critical += 1,
                ai_tools_core::finding::Severity::High => counts.high += 1,
                ai_tools_core::finding::Severity::Medium => counts.medium += 1,
                ai_tools_core::finding::Severity::Low => counts.low += 1,
            }
            match f.source_tool.as_str() {
                "diff-risk" => tool_counts.diff_risk += 1,
                "cargo-impact" => tool_counts.cargo_impact += 1,
                "spec-drift" => tool_counts.spec_drift += 1,
                _ => {}
            }
        }

        Self {
            tool: tool.to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            total_findings: findings.len(),
            by_severity: counts,
            by_tool: tool_counts,
            findings: findings.to_vec(),
        }
    }
}
