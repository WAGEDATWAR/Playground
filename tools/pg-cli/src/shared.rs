//! Helpers shared by the `pg` subcommands.

use pg_content::{load_pack, ComponentRegistry, ContentSet, DirPack, Limits};
use std::sync::Arc;

/// The shipped base content, relative to the repository root.
pub const DEFAULT_CONTENT_DIR: &str = "data/base";

/// Loads and builds content from pack directories, with no output. Errors list every problem.
pub fn load_content(dirs: &[String]) -> Result<Arc<ContentSet>, String> {
    let mut packs = Vec::new();
    for dir in dirs {
        match load_pack(&DirPack::new(dir), &Limits::default()) {
            Ok(p) => packs.push(p),
            Err(report) => return Err(format!("cannot load pack '{dir}':\n{report}")),
        }
    }
    ContentSet::build(packs, ComponentRegistry::builtin())
        .map(Arc::new)
        .map_err(|report| format!("content did not build:\n{report}"))
}

/// First 8 hex characters of a hash, for compact display.
pub fn short(h: &pg_core::hash::StateHash) -> String {
    h.to_hex().chars().take(8).collect()
}

/// An event filter (`pg sim --events [prefix] --since T --until T`, suggestion S-009).
#[derive(Debug, Clone, Default)]
pub struct EventFilter {
    pub enabled: bool,
    pub prefix: Option<String>,
    pub since: u64,
    pub until: Option<u64>,
}

impl EventFilter {
    pub fn matches(&self, kind: &str, tick: u64) -> bool {
        self.enabled
            && self.prefix.as_deref().is_none_or(|p| kind.starts_with(p))
            && tick >= self.since
            && self.until.is_none_or(|u| tick <= u)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_event_filter_combines_prefix_and_range() {
        let off = EventFilter::default();
        assert!(!off.matches("move.arrived", 5));
        let all = EventFilter {
            enabled: true,
            ..EventFilter::default()
        };
        assert!(all.matches("anything", 0));
        let f = EventFilter {
            enabled: true,
            prefix: Some("move.".into()),
            since: 10,
            until: Some(20),
        };
        assert!(f.matches("move.arrived", 10) && f.matches("move.failed", 20));
        assert!(!f.matches("move.arrived", 9) && !f.matches("move.arrived", 21));
        assert!(!f.matches("map.created", 15));
    }
}
