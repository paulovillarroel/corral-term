//! Corral's agent skill is text only: Markdown an agent reads, nothing it runs when it
//! loads the skill. Agent Skills can carry commands that run as the skill loads, before
//! the model reads it and without a permission prompt (in Claude Code, `!` followed by a
//! command in backquotes, or a code block fenced with "```!"), and scripts of their own.
//! Corral's has neither: what an agent does with Corral, it does by running `corral`
//! commands it can see, which go through its permissions and Corral's harness. CI keeps
//! it that way here.

use std::path::{Path, PathBuf};

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files(&path, out);
        } else {
            out.push(path);
        }
    }
}

#[test]
fn the_skill_runs_nothing_when_it_loads() {
    let skill = Path::new(env!("CARGO_MANIFEST_DIR")).join("../skills/corral");
    let mut all = Vec::new();
    files(&skill, &mut all);
    assert!(all.iter().any(|p| p.ends_with("SKILL.md")), "no SKILL.md in {}", skill.display());
    for path in &all {
        // Markdown only: no scripts, nothing executable.
        assert!(path.extension().is_some_and(|e| e == "md"), "{} isn't Markdown", path.display());
        let mode = std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(path).unwrap().permissions());
        assert_eq!(mode & 0o111, 0, "{} is executable", path.display());
        let text = std::fs::read_to_string(path).unwrap();
        for (n, line) in text.lines().enumerate() {
            let at = format!("{}:{}", path.display(), n + 1);
            assert!(!line.contains("!`"), "{at} runs a command when the skill loads: {line}");
            assert!(!line.trim_start().starts_with("```!"), "{at} opens a block that runs when the skill loads: {line}");
        }
    }
}
