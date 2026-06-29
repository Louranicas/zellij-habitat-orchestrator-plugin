//! Catalog assembly.
//!
//! Enumerates the factory's callable surface: workflows under `.claude/workflows`,
//! agents under `.claude/agents`, and `just` recipes (`just --list`). Filesystem
//! scans use [`std::fs`]; recipe listing uses the injected runner.

use std::fs;
use std::path::Path;

use crate::error::PerceiveError;
use crate::exec::CommandRunner;
use crate::manifest::CatalogObservation;
use crate::Result;

/// Filesystem roots scanned to build the catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanRoots {
    /// Directory containing workflow definitions.
    pub workflows_dir: String,
    /// Directory containing agent definitions.
    pub agents_dir: String,
    /// Working directory from which `just --list` is run.
    pub just_dir: String,
}

/// Assembles the catalog observation from the given roots.
///
/// The catalog is assembled from three sources:
///
/// 1. **Workflows** — every file directly inside `roots.workflows_dir` whose
///    name does not start with `_` and ends with `.js`, `.ts`, `.md`,
///    `.yaml`/`.yml`, or `.json`.  Entries are returned as bare file stems
///    (no extension, no directory prefix), sorted.
///
/// 2. **Agents** — every file directly inside `roots.agents_dir` that ends
///    with `.md`.  Entries are returned as bare file stems, sorted.
///
/// 3. **Just recipes** — output of `just --list` parsed to extract recipe
///    names.  If `just` is not installed or the working directory has no
///    `justfile`, an empty list is returned without propagating an error.
///
/// # Errors
/// Returns [`PerceiveError::Subprocess`] if a directory exists but cannot
/// be read (permission error).  A directory that does not exist is treated
/// as empty, which is not an error.
pub fn assemble(runner: &dyn CommandRunner, roots: &ScanRoots) -> Result<CatalogObservation> {
    let workflows = scan_dir(&roots.workflows_dir, is_workflow_file)?;
    let agents = scan_dir(&roots.agents_dir, is_agent_file)?;
    let just_recipes = just_list(runner, &roots.just_dir);
    let source = format!(
        "just --list + fs-scan:{}:{}",
        roots.workflows_dir, roots.agents_dir
    );

    Ok(CatalogObservation {
        workflows,
        agents,
        just_recipes,
        source,
    })
}

// ---------------------------------------------------------------------------
// Filesystem helpers
// ---------------------------------------------------------------------------

fn scan_dir<F>(dir: &str, accept: F) -> Result<Vec<String>>
where
    F: Fn(&str) -> bool,
{
    let path = Path::new(dir);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let entries = fs::read_dir(path).map_err(|err| PerceiveError::Subprocess {
        command: format!("read-dir:{dir}"),
        detail: err.to_string(),
    })?;

    let mut names: Vec<String> = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();
            if accept(name.as_ref()) {
                stem(name.as_ref())
            } else {
                None
            }
        })
        .collect();

    names.sort_unstable();
    Ok(names)
}

fn stem(file_name: &str) -> Option<String> {
    Path::new(file_name)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
}

fn has_extension(name: &str, ext: &str) -> bool {
    std::path::Path::new(name)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

fn is_workflow_file(name: &str) -> bool {
    if name.starts_with('_') {
        return false;
    }
    has_extension(name, "js")
        || has_extension(name, "ts")
        || has_extension(name, "md")
        || has_extension(name, "yaml")
        || has_extension(name, "yml")
        || has_extension(name, "json")
}

fn is_agent_file(name: &str) -> bool {
    has_extension(name, "md")
}

// ---------------------------------------------------------------------------
// `just --list` runner
// ---------------------------------------------------------------------------

/// Path to the `just` binary (absolute, per D11 rule).
const JUST_PATH: &str = "/home/louranicas/.local/bin/just";

fn just_list(runner: &dyn CommandRunner, dir: &str) -> Vec<String> {
    let justfile = format!("{dir}/justfile");

    // Attempt 1: explicit justfile path + no-prefix
    let out = runner.run(&[
        JUST_PATH.to_string(),
        "--list".to_string(),
        "--list-prefix".to_string(),
        String::new(),
        "--color".to_string(),
        "never".to_string(),
        "--justfile".to_string(),
        justfile.clone(),
    ]);

    match out {
        Ok(o) if o.status == 0 => return parse_just_list(&o.stdout),
        _ => {}
    }

    // Attempt 2: without --list-prefix (older just) or when explicit path fails
    match runner.run(&[
        JUST_PATH.to_string(),
        "--list".to_string(),
        "--justfile".to_string(),
        justfile,
    ]) {
        Ok(o) if o.status == 0 => parse_just_list(&o.stdout),
        _ => Vec::new(),
    }
}

/// Parses `just --list` output.
///
/// The format produced by recent `just` versions is:
/// ```text
/// Available recipes:
///     recipe-name        # comment
///     other-recipe args  # comment
/// ```
///
/// We skip the header line and blank lines, strip leading whitespace, and
/// take the first whitespace-separated token as the recipe name, discarding
/// tokens that start with `#`.
fn parse_just_list(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter(|l| !l.trim().starts_with("Available"))
        .map(|l| {
            l.split_whitespace()
                .next()
                .unwrap_or("")
                .to_string()
        })
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::exec::{CommandOutput, CommandRunner};
    use std::io::Write;

    // -----------------------------------------------------------------------
    // Fake runner for just --list
    // -----------------------------------------------------------------------

    struct FakeJust {
        output: String,
        status: i32,
    }

    impl CommandRunner for FakeJust {
        fn run(&self, _argv: &[String]) -> crate::Result<CommandOutput> {
            Ok(CommandOutput {
                status: self.status,
                stdout: self.output.clone(),
                stderr: String::new(),
            })
        }
    }

    fn ok_just(output: &str) -> FakeJust {
        FakeJust {
            output: output.to_string(),
            status: 0,
        }
    }

    fn fail_just() -> FakeJust {
        FakeJust {
            output: String::new(),
            status: 1,
        }
    }

    // -----------------------------------------------------------------------
    // Temporary directory helpers (no external crate needed)
    // -----------------------------------------------------------------------

    fn make_temp_dir(suffix: &str) -> std::path::PathBuf {
        let base = std::env::temp_dir().join(format!(
            "orchestrator-perceive-catalog-test-{suffix}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.subsec_nanos())
        ));
        fs::create_dir_all(&base).unwrap();
        base
    }

    fn touch(dir: &Path, name: &str) {
        let mut f = fs::File::create(dir.join(name)).unwrap();
        f.write_all(b"").unwrap();
    }

    fn cleanup(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }

    fn temp_roots(tag: &str) -> (std::path::PathBuf, ScanRoots) {
        let base = make_temp_dir(tag);
        let wf = base.join("workflows");
        let ag = base.join("agents");
        fs::create_dir_all(&wf).unwrap();
        fs::create_dir_all(&ag).unwrap();
        let roots = ScanRoots {
            workflows_dir: wf.to_string_lossy().to_string(),
            agents_dir: ag.to_string_lossy().to_string(),
            just_dir: base.to_string_lossy().to_string(),
        };
        (base, roots)
    }

    // -----------------------------------------------------------------------
    // parse_just_list tests
    // -----------------------------------------------------------------------

    #[test]
    fn parse_just_list_empty_string() {
        assert!(parse_just_list("").is_empty());
    }

    #[test]
    fn parse_just_list_skips_available_header() {
        let text = "Available recipes:\n    gate\n    test\n";
        let recipes = parse_just_list(text);
        assert_eq!(recipes, vec!["gate", "test"]);
    }

    #[test]
    fn parse_just_list_strips_comments_after_name() {
        let text = "Available recipes:\n    gate     # run quality gate\n";
        let recipes = parse_just_list(text);
        assert_eq!(recipes, vec!["gate"]);
    }

    #[test]
    fn parse_just_list_handles_arguments_after_name() {
        let text = "    deploy SERVICE PORT  # deploy\n";
        let recipes = parse_just_list(text);
        assert_eq!(recipes, vec!["deploy"]);
    }

    #[test]
    fn parse_just_list_skips_blank_lines() {
        let text = "\n    gate\n\n    test\n";
        let recipes = parse_just_list(text);
        assert_eq!(recipes, vec!["gate", "test"]);
    }

    #[test]
    fn parse_just_list_multiple_recipes() {
        let text =
            "Available recipes:\n    gate\n    test\n    deploy\n    cci\n    cci-all\n";
        let recipes = parse_just_list(text);
        assert_eq!(recipes.len(), 5);
        assert!(recipes.contains(&"cci-all".to_string()));
    }

    // -----------------------------------------------------------------------
    // is_workflow_file tests
    // -----------------------------------------------------------------------

    #[test]
    fn workflow_accepts_js_extension() {
        assert!(is_workflow_file("my-workflow.js"));
    }

    #[test]
    fn workflow_accepts_ts_extension() {
        assert!(is_workflow_file("my-workflow.ts"));
    }

    #[test]
    fn workflow_accepts_md_extension() {
        assert!(is_workflow_file("deploy.md"));
    }

    #[test]
    fn workflow_accepts_yaml_extension() {
        assert!(is_workflow_file("ci.yaml"));
    }

    #[test]
    fn workflow_accepts_yml_extension() {
        assert!(is_workflow_file("ci.yml"));
    }

    #[test]
    fn workflow_accepts_json_extension() {
        assert!(is_workflow_file("schema.json"));
    }

    #[test]
    fn workflow_rejects_underscore_prefix() {
        assert!(!is_workflow_file("_perceive_probe.js"));
    }

    #[test]
    fn workflow_rejects_unknown_extension() {
        assert!(!is_workflow_file("script.sh"));
    }

    #[test]
    fn workflow_rejects_rs_extension() {
        assert!(!is_workflow_file("build.rs"));
    }

    #[test]
    fn agent_accepts_md() {
        assert!(is_agent_file("orchestrator.md"));
    }

    #[test]
    fn agent_rejects_non_md() {
        assert!(!is_agent_file("agent.js"));
    }

    // -----------------------------------------------------------------------
    // scan_dir tests
    // -----------------------------------------------------------------------

    #[test]
    fn scan_dir_nonexistent_returns_empty() {
        let result = scan_dir("/nonexistent/dir/xyz-perceive-test", |_| true).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn scan_dir_lists_matching_workflow_files() {
        let dir = make_temp_dir("scan1");
        touch(&dir, "workflow-a.js");
        touch(&dir, "workflow-b.ts");
        touch(&dir, "README.txt");

        let result = scan_dir(&dir.to_string_lossy(), is_workflow_file).unwrap();
        assert!(result.contains(&"workflow-a".to_string()));
        assert!(result.contains(&"workflow-b".to_string()));
        assert!(!result.iter().any(|s| s.contains("README")));
        cleanup(&dir);
    }

    #[test]
    fn scan_dir_returns_sorted_output() {
        let dir = make_temp_dir("scan2");
        touch(&dir, "z.md");
        touch(&dir, "a.md");
        touch(&dir, "m.md");

        let result = scan_dir(&dir.to_string_lossy(), is_agent_file).unwrap();
        assert_eq!(result, vec!["a", "m", "z"]);
        cleanup(&dir);
    }

    #[test]
    fn scan_dir_excludes_underscore_workflows() {
        let dir = make_temp_dir("scan3");
        touch(&dir, "_private.js");
        touch(&dir, "public.js");

        let result = scan_dir(&dir.to_string_lossy(), is_workflow_file).unwrap();
        assert_eq!(result, vec!["public"]);
        cleanup(&dir);
    }

    // -----------------------------------------------------------------------
    // assemble() integration tests
    // -----------------------------------------------------------------------

    #[test]
    fn assemble_empty_dirs_and_no_just_returns_empty_catalog() {
        let (base, roots) = temp_roots("asm1");
        let runner = fail_just();
        let cat = assemble(&runner, &roots).unwrap();
        assert!(cat.workflows.is_empty());
        assert!(cat.agents.is_empty());
        assert!(cat.just_recipes.is_empty());
        cleanup(&base);
    }

    #[test]
    fn assemble_picks_up_workflow_files() {
        let (base, roots) = temp_roots("asm2");
        let wf_path = Path::new(&roots.workflows_dir);
        touch(wf_path, "deploy.md");
        touch(wf_path, "test-suite.js");

        let runner = fail_just();
        let cat = assemble(&runner, &roots).unwrap();
        assert!(cat.workflows.contains(&"deploy".to_string()));
        assert!(cat.workflows.contains(&"test-suite".to_string()));
        cleanup(&base);
    }

    #[test]
    fn assemble_picks_up_agent_files() {
        let (base, roots) = temp_roots("asm3");
        let ag_path = Path::new(&roots.agents_dir);
        touch(ag_path, "orchestrator.md");
        touch(ag_path, "forge.md");

        let runner = fail_just();
        let cat = assemble(&runner, &roots).unwrap();
        assert!(cat.agents.contains(&"orchestrator".to_string()));
        assert!(cat.agents.contains(&"forge".to_string()));
        cleanup(&base);
    }

    #[test]
    fn assemble_includes_just_recipes() {
        let (base, roots) = temp_roots("asm4");
        let runner = ok_just("Available recipes:\n    gate\n    test\n    deploy\n");
        let cat = assemble(&runner, &roots).unwrap();
        assert!(cat.just_recipes.contains(&"gate".to_string()));
        assert!(cat.just_recipes.contains(&"test".to_string()));
        assert!(cat.just_recipes.contains(&"deploy".to_string()));
        cleanup(&base);
    }

    #[test]
    fn assemble_source_field_contains_scan_prefix() {
        let (base, roots) = temp_roots("asm5");
        let runner = fail_just();
        let cat = assemble(&runner, &roots).unwrap();
        assert!(cat.source.starts_with("just --list + fs-scan:"));
        cleanup(&base);
    }

    #[test]
    fn assemble_ignores_underscore_workflow() {
        let (base, roots) = temp_roots("asm6");
        let wf_path = Path::new(&roots.workflows_dir);
        touch(wf_path, "_perceive_probe.js");
        touch(wf_path, "real-workflow.js");

        let runner = fail_just();
        let cat = assemble(&runner, &roots).unwrap();
        assert!(!cat.workflows.iter().any(|w| w.starts_with('_')));
        assert!(cat.workflows.contains(&"real-workflow".to_string()));
        cleanup(&base);
    }

    #[test]
    fn assemble_planted_workflow_appears_in_next_observation() {
        // Simulates the G2 acceptance test: a file planted after the first
        // call is visible in the second call with no code change.
        let (base, roots) = temp_roots("asm7");
        let wf_path = Path::new(&roots.workflows_dir);
        let runner = fail_just();

        let cat1 = assemble(&runner, &roots).unwrap();
        assert!(!cat1.workflows.contains(&"planted".to_string()));

        touch(wf_path, "planted.md");
        let cat2 = assemble(&runner, &roots).unwrap();
        assert!(cat2.workflows.contains(&"planted".to_string()));
        cleanup(&base);
    }
}
