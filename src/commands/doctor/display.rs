//! Display and reporting functions for the doctor command.

use super::{DoctorReport, IssueSeverity};

/// Print the doctor report.
pub fn print_report(report: &DoctorReport, repair_mode: bool) {
    if !report.has_issues() && report.repairs.is_empty() {
        println!("Workflow is healthy. No issues detected.");
        return;
    }

    // Print issues
    if !report.issues.is_empty() {
        println!("Issues detected ({}):", report.issues.len());
        println!();

        for (i, issue) in report.issues.iter().enumerate() {
            println!(
                "  {}. [{}] {} - {}",
                i + 1,
                issue.severity,
                issue.category,
                issue.description
            );

            if let Some(path) = &issue.path {
                println!("     Path: {}", path);
            }

            if let Some(remediation) = &issue.remediation {
                println!(
                    "     Fix:  {}",
                    remediation.lines().next().unwrap_or(remediation)
                );
                for line in remediation.lines().skip(1) {
                    println!("           {}", line);
                }
            }

            if issue.repairable && !repair_mode {
                println!("     (auto-repairable with --repair --force)");
            }

            println!();
        }
    }

    // Print repairs
    if !report.repairs.is_empty() {
        println!("Repairs applied ({}):", report.repairs.len());
        println!();

        for repair in &report.repairs {
            println!("  - {}", repair);
        }

        println!();
    }

    // Print summary
    let error_count = report
        .issues
        .iter()
        .filter(|i| i.severity == IssueSeverity::Error)
        .count();
    let warning_count = report
        .issues
        .iter()
        .filter(|i| i.severity == IssueSeverity::Warning)
        .count();
    let repairable_count = report.issues.iter().filter(|i| i.repairable).count();

    if repair_mode {
        let remaining = report.issues.iter().filter(|i| !i.repairable).count();
        if remaining > 0 {
            println!(
                "Summary: {} issue(s) remain that cannot be auto-repaired ({} errors, {} warnings).",
                remaining, error_count, warning_count
            );
        } else if !report.repairs.is_empty() {
            println!("All repairable issues have been fixed.");
        }
    } else {
        println!(
            "Summary: {} errors, {} warnings, {} auto-repairable.",
            error_count, warning_count, repairable_count
        );

        if repairable_count > 0 {
            println!();
            println!("Run `burl doctor --repair --force` to apply safe repairs.");
        }
    }
}
