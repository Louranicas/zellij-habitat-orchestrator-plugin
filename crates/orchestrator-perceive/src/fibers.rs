//! Fiber and campaign observation.
//!
//! Reads `hopf-anchor fibers` and records the active campaigns and their loops.
//!
//! ## Output format
//!
//! `hopf-anchor fibers` prints either JSON or one-per-line text.  We accept
//! both:
//!
//! **JSON array** (preferred):
//! ```json
//! [{"campaign":"factory.my-campaign","root":"/home/…","loops":["loop-1","loop-2"]}]
//! ```
//!
//! **Text** (one line per campaign, `|`-separated):
//! ```text
//! factory.my-campaign|/home/…|loop-1,loop-2
//! ```
//!
//! If `hopf-anchor` is not installed or returns no fibers the function returns
//! an empty `Vec` without propagating an error.

use crate::exec::CommandRunner;
use crate::manifest::FiberObservation;
use crate::Result;

/// Path to the `hopf-anchor` binary (absolute, per D11 rule).
const HOPF_ANCHOR_PATH: &str = "/home/louranicas/.local/bin/hopf-anchor";

/// Reads the active fiber / campaign topology.
///
/// The runner is used to invoke `hopf-anchor fibers`.  If the command is
/// unavailable, returns a non-zero status, or its output cannot be parsed,
/// an empty vector is returned — never an error — so the assembler can
/// complete a partial snapshot from the other sources.
///
/// # Errors
/// This function never returns `Err`; it always yields `Ok(Vec<…>)`.
pub fn observe(runner: &dyn CommandRunner) -> Result<Vec<FiberObservation>> {
    let out = match runner.run(&[
        HOPF_ANCHOR_PATH.to_string(),
        "fibers".to_string(),
    ]) {
        Ok(o) if o.status == 0 && !o.stdout.trim().is_empty() => o,
        _ => return Ok(Vec::new()),
    };

    Ok(parse_fibers_output(&out.stdout))
}

fn parse_fibers_output(text: &str) -> Vec<FiberObservation> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    // Try JSON array first
    if trimmed.starts_with('[') {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(arr) = v.as_array() {
                return arr.iter().filter_map(parse_fiber_object).collect();
            }
        }
    }

    // Fallback: text lines  campaign|root|loop1,loop2
    parse_fiber_lines(trimmed)
}

fn parse_fiber_object(v: &serde_json::Value) -> Option<FiberObservation> {
    let campaign = v.get("campaign")?.as_str()?.to_string();
    let root = v
        .get("root")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string();
    let loops: Vec<String> = v
        .get("loops")
        .and_then(serde_json::Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    Some(FiberObservation {
        campaign,
        root,
        loops,
    })
}

fn parse_fiber_lines(text: &str) -> Vec<FiberObservation> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(parse_fiber_line)
        .collect()
}

fn parse_fiber_line(line: &str) -> Option<FiberObservation> {
    let mut parts = line.splitn(3, '|');
    let campaign = parts.next()?.trim().to_string();
    if campaign.is_empty() {
        return None;
    }
    let root = parts.next().unwrap_or("").trim().to_string();
    let loops_str = parts.next().unwrap_or("").trim();
    let loops: Vec<String> = if loops_str.is_empty() {
        Vec::new()
    } else {
        loops_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };

    Some(FiberObservation {
        campaign,
        root,
        loops,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::exec::{CommandOutput, CommandRunner};

    struct FakeAnchor {
        status: i32,
        stdout: String,
    }

    impl CommandRunner for FakeAnchor {
        fn run(&self, _argv: &[String]) -> crate::Result<CommandOutput> {
            Ok(CommandOutput {
                status: self.status,
                stdout: self.stdout.clone(),
                stderr: String::new(),
            })
        }
    }

    fn ok_anchor(text: &str) -> FakeAnchor {
        FakeAnchor {
            status: 0,
            stdout: text.to_string(),
        }
    }

    fn fail_anchor() -> FakeAnchor {
        FakeAnchor {
            status: 1,
            stdout: String::new(),
        }
    }

    // -----------------------------------------------------------------------
    // parse_fibers_output tests (JSON path)
    // -----------------------------------------------------------------------

    #[test]
    fn parse_fibers_empty_string() {
        assert!(parse_fibers_output("").is_empty());
    }

    #[test]
    fn parse_fibers_empty_json_array() {
        assert!(parse_fibers_output("[]").is_empty());
    }

    #[test]
    fn parse_fibers_json_single_entry() {
        let json = r#"[{"campaign":"factory.my","root":"/home","loops":["l1","l2"]}]"#;
        let fibers = parse_fibers_output(json);
        assert_eq!(fibers.len(), 1);
        assert_eq!(fibers[0].campaign, "factory.my");
        assert_eq!(fibers[0].root, "/home");
        assert_eq!(fibers[0].loops, vec!["l1", "l2"]);
    }

    #[test]
    fn parse_fibers_json_multiple_entries() {
        let json = r#"[
            {"campaign":"camp-a","root":"/a","loops":["la"]},
            {"campaign":"camp-b","root":"/b","loops":[]}
        ]"#;
        let fibers = parse_fibers_output(json);
        assert_eq!(fibers.len(), 2);
    }

    #[test]
    fn parse_fibers_json_missing_loops_defaults_empty() {
        let json = r#"[{"campaign":"x","root":"/r"}]"#;
        let fibers = parse_fibers_output(json);
        assert_eq!(fibers.len(), 1);
        assert!(fibers[0].loops.is_empty());
    }

    #[test]
    fn parse_fibers_json_missing_root_defaults_empty_string() {
        let json = r#"[{"campaign":"x"}]"#;
        let fibers = parse_fibers_output(json);
        assert_eq!(fibers.len(), 1);
        assert_eq!(fibers[0].root, "");
    }

    #[test]
    fn parse_fibers_json_entry_missing_campaign_skipped() {
        let json = r#"[{"root":"/r","loops":[]},{"campaign":"valid","root":"/v","loops":[]}]"#;
        let fibers = parse_fibers_output(json);
        assert_eq!(fibers.len(), 1);
        assert_eq!(fibers[0].campaign, "valid");
    }

    // -----------------------------------------------------------------------
    // parse_fibers_output tests (text path)
    // -----------------------------------------------------------------------

    #[test]
    fn parse_fibers_text_single_line() {
        let text = "factory.camp|/home/user|loop-1,loop-2\n";
        let fibers = parse_fibers_output(text);
        assert_eq!(fibers.len(), 1);
        assert_eq!(fibers[0].campaign, "factory.camp");
        assert_eq!(fibers[0].root, "/home/user");
        assert_eq!(fibers[0].loops, vec!["loop-1", "loop-2"]);
    }

    #[test]
    fn parse_fibers_text_no_loops_column() {
        let text = "factory.camp|/home/user\n";
        let fibers = parse_fibers_output(text);
        assert_eq!(fibers.len(), 1);
        assert!(fibers[0].loops.is_empty());
    }

    #[test]
    fn parse_fibers_text_empty_loops() {
        let text = "factory.camp|/home|\n";
        let fibers = parse_fibers_output(text);
        assert_eq!(fibers.len(), 1);
        assert!(fibers[0].loops.is_empty());
    }

    #[test]
    fn parse_fibers_text_multiple_lines() {
        let text = "camp-a|/a|l1\ncamp-b|/b|l2,l3\n";
        let fibers = parse_fibers_output(text);
        assert_eq!(fibers.len(), 2);
        assert_eq!(fibers[0].loops.len(), 1);
        assert_eq!(fibers[1].loops.len(), 2);
    }

    #[test]
    fn parse_fibers_text_skips_blank_lines() {
        let text = "camp-a|/a|l1\n\ncamp-b|/b|l2\n";
        let fibers = parse_fibers_output(text);
        assert_eq!(fibers.len(), 2);
    }

    // -----------------------------------------------------------------------
    // observe() tests
    // -----------------------------------------------------------------------

    #[test]
    fn observe_hopf_unavailable_returns_empty() {
        let result = observe(&fail_anchor()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn observe_empty_output_returns_empty() {
        let result = observe(&ok_anchor("")).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn observe_json_output_parsed() {
        let json = r#"[{"campaign":"c1","root":"/r1","loops":["la","lb"]}]"#;
        let result = observe(&ok_anchor(json)).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].campaign, "c1");
    }

    #[test]
    fn observe_text_output_parsed() {
        let text = "c1|/r1|la,lb\nc2|/r2|lc\n";
        let result = observe(&ok_anchor(text)).unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn observe_never_returns_error() {
        struct AlwaysFails;
        impl CommandRunner for AlwaysFails {
            fn run(&self, _: &[String]) -> crate::Result<CommandOutput> {
                Err(crate::error::PerceiveError::Subprocess {
                    command: "hopf-anchor".to_string(),
                    detail: "not installed".to_string(),
                })
            }
        }
        assert!(observe(&AlwaysFails).is_ok());
    }
}
