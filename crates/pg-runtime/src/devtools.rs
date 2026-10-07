//! The developer-tool registry (Blueprint §20, milestone 0.8 skeleton).
//!
//! Every dev tool is listed once with where it lives: a `pg` subcommand, an overlay panel (milestone 0.10),
//! or both. `pg tools` prints this list, and the overlay builds its menu from it, so a tool added to the
//! registry shows up in both places and an overlay panel cannot silently lack a command-line twin.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DevTool {
    pub id: &'static str,
    pub title: &'static str,
    pub category: &'static str,
    pub summary: &'static str,
    /// The command-line form, if there is one.
    pub cli: Option<&'static str>,
    /// Whether the overlay gets a panel for it (built in 0.10).
    pub overlay: bool,
}

const fn tool(
    id: &'static str,
    title: &'static str,
    category: &'static str,
    summary: &'static str,
    cli: Option<&'static str>,
    overlay: bool,
) -> DevTool {
    DevTool {
        id,
        title,
        category,
        summary,
        cli,
        overlay,
    }
}

/// All developer tools, in a stable order.
pub fn registry() -> Vec<DevTool> {
    vec![
        tool(
            "events",
            "Event viewer",
            "explain",
            "Events from the typed catalog, filtered by kind prefix and tick range.",
            Some("pg events list"),
            true,
        ),
        tool(
            "reasons",
            "Reason explorer",
            "explain",
            "Why each pawn's day looks as it does, with the deciding system named.",
            Some("pg schedule"),
            true,
        ),
        tool(
            "hashes",
            "State hashes",
            "determinism",
            "Per-table and per-row hashes; where two runs first differ.",
            Some("pg replay"),
            true,
        ),
        tool(
            "profiler",
            "Tick profiler",
            "performance",
            "Time per system per tick.",
            Some("pg profile"),
            true,
        ),
        tool(
            "keyframes",
            "Time scrub",
            "determinism",
            "Step back to an earlier keyframe and run forward again.",
            Some("pg run --scrub"),
            true,
        ),
        tool(
            "shadow",
            "Shadow verification",
            "determinism",
            "Re-simulate spans with another thread count and compare.",
            Some("pg run --shadow"),
            true,
        ),
        tool(
            "saves",
            "Save inspector",
            "persistence",
            "Generations, summaries and damage in a save slot.",
            Some("pg save list"),
            true,
        ),
        tool(
            "bundles",
            "Bug bundles",
            "persistence",
            "Cut, open and replay bundles; crash bundles appear here.",
            Some("pg bugbundle"),
            true,
        ),
        tool(
            "settings",
            "Settings registry",
            "configuration",
            "Every setting with its type, range and default.",
            Some("pg settings list"),
            true,
        ),
        tool(
            "strings",
            "String tables",
            "content",
            "Missing, orphaned and mismatched strings; pseudo-locale.",
            Some("pg strings lint"),
            false,
        ),
        tool(
            "packs",
            "Pack status",
            "content",
            "Loaded packs, hashes, warnings and quarantine.",
            Some("pg content lint"),
            true,
        ),
        tool(
            "ai",
            "AI status",
            "ai",
            "Provider, key status, breaker state and the exact request.",
            Some("pg ai"),
            true,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_every_tool_is_reachable() {
        let tools = registry();
        let mut ids: Vec<&str> = tools.iter().map(|t| t.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), tools.len());
        for t in &tools {
            assert!(
                t.cli.is_some() || t.overlay,
                "{} has no way to be used",
                t.id
            );
            assert!(!t.title.is_empty() && !t.summary.is_empty());
        }
        assert!(tools.len() >= 10);
    }
}
