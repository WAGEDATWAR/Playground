//! Reason codes (Blueprint §8.8, suggestion S-010).
//!
//! Every planning, failure and acceptance decision stores a [`ReasonCode`]: a stable code, **who made the
//! decision** (the engine or a pack), and a small bag of integer/text/id parameters. The inspector and
//! `pg schedule explain` turn them into plain sentences. Codes are saved with the current day's schedule
//! and dropped at day rollover, except those attached to memories or events.

use crate::canon::{Canon, ToCanon};
use crate::read::{ReadError, Reader};
use std::collections::BTreeMap;

/// Who produced a decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Builtin,
    Pack(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReasonCode {
    pub code: String,
    pub source: Source,
    pub params: BTreeMap<String, Canon>,
}

/// Sentence templates for built-in codes. `{name}` is replaced by the parameter of that name.
const TEMPLATES: &[(&str, &str)] = &[
    // planning
    ("duty_placed", "A required duty was placed at slot {slot} for {slots} slot(s)."),
    ("duty_varied", "The duty's start moved by {shift} slot(s) and its length by -{shrink} (seeded variation), giving slots {start}..{end}."),
    ("duty_dropped", "A duty at slot {slot} was dropped: it overlapped a stronger claim ({by})."),
    ("urgent_need_placed", "A restoring {need} activity was wanted by slot {predicted} and was placed at slot {slot}."),
    ("urgent_need_displaced", "The urgent {need} activity at slot {slot} displaced a lower-priority reservation ({displaced})."),
    ("urgent_need_unplaced", "There was no room before slot {predicted} for the urgent {need} activity, and nothing could be displaced."),
    ("commitment_placed", "An accepted commitment was placed at slot {slot} for {slots} slot(s)."),
    ("commitment_moved", "A commitment was moved from slot {from} to slot {to} because a stronger reservation took its slot."),
    ("commitment_failed", "A commitment at slot {slot} could not be kept: {why}."),
    ("chore_placed", "A chore was placed at slot {slot} for {slots} slot(s) (urgency {urgency})."),
    ("chore_dropped", "A chore (urgency {urgency}) was dropped: no free slot fits it{deadline}."),
    ("leisure_chosen", "Leisure was chosen at slot {slot} for {slots} slot(s) (weight {weight} of {total})."),
    ("moved_later", "A reservation was moved from slot {from} to slot {to} because an urgent need took its slot."),
    ("no_free_slot", "{what} was dropped: no free slot remained today."),
    ("slots_open", "{count} slot(s) were left open."),
    ("replanned", "The rest of the day was replanned from slot {from}: {why}."),
    // task and movement failures (Blueprint §8.7)
    ("blocked_destination", "The destination is blocked or occupied."),
    ("unreachable_or_too_far", "No path was found to the destination."),
    ("path_blocked", "The route stayed blocked by other pawns."),
    ("precondition_failed", "The action's preconditions did not hold: {why}."),
    ("interrupted", "The task was interrupted because the schedule moved on."),
    ("unaffordable", "The pawn could not afford the action."),
    ("unauthorized", "The pawn is not permitted to do this."),
    ("unknown_action", "No action named {action} is registered."),
    ("bad_params", "The action's parameters were invalid: {why}."),
    // commitments (Blueprint §8.6)
    ("commitment_accepted", "The invitee accepted; both schedules reserved slots {start}..{end}."),
    ("commitment_declined", "The invitee declined: {why}."),
    ("commitment_expired", "The proposal expired without an answer."),
    ("commitment_active", "The commitment began."),
    ("commitment_completed", "Both pawns kept the commitment."),
    ("commitment_no_show", "The commitment failed: {why}."),
    ("commitment_cancelled", "The commitment was cancelled: {why}."),
];

fn text_of(v: &Canon) -> String {
    match v {
        Canon::Str(s) => s.clone(),
        Canon::Int(i) => i.to_string(),
        Canon::Bool(b) => b.to_string(),
        Canon::Null => "none".to_owned(),
        other => other.to_canonical_string(),
    }
}

impl ReasonCode {
    /// An engine decision.
    pub fn builtin<'a>(
        code: &str,
        params: impl IntoIterator<Item = (&'a str, Canon)>,
    ) -> ReasonCode {
        ReasonCode {
            code: code.to_owned(),
            source: Source::Builtin,
            params: params.into_iter().map(|(k, v)| (k.to_owned(), v)).collect(),
        }
    }

    /// A decision made by a content pack's script.
    pub fn from_pack<'a>(
        pack: &str,
        code: &str,
        params: impl IntoIterator<Item = (&'a str, Canon)>,
    ) -> ReasonCode {
        ReasonCode {
            code: code.to_owned(),
            source: Source::Pack(pack.to_owned()),
            params: params.into_iter().map(|(k, v)| (k.to_owned(), v)).collect(),
        }
    }

    /// A reason with no parameters.
    pub fn simple(code: &str) -> ReasonCode {
        ReasonCode::builtin(code, [])
    }

    pub fn param(&self, name: &str) -> Option<&Canon> {
        self.params.get(name)
    }

    /// A plain sentence in the built-in English, with the pack named when a pack made the decision.
    pub fn explain(&self) -> String {
        self.explain_with(&|code| {
            TEMPLATES
                .iter()
                .find(|(c, _)| *c == code)
                .map(|(_, t)| (*t).to_owned())
        })
    }

    /// The sentence in `locale` from the content's string tables (falling back to English, then to the
    /// built-in text).
    pub fn explain_in(&self, strings: &pg_content::Strings, locale: &str) -> String {
        self.explain_with(&|code| {
            strings
                .template(locale, &format!("reason.{code}"))
                .or_else(|| {
                    TEMPLATES
                        .iter()
                        .find(|(c, _)| *c == code)
                        .map(|(_, t)| (*t).to_owned())
                })
        })
    }

    /// Like [`ReasonCode::explain`], with sentence templates supplied by `template_for(code)`, normally a
    /// lookup of `reason.<code>` in the active string table (S-026).
    pub fn explain_with(&self, template_for: &dyn Fn(&str) -> Option<String>) -> String {
        let body = match template_for(&self.code) {
            Some(template) if self.source == Source::Builtin => {
                let mut out = template;
                for (name, value) in &self.params {
                    out = out.replace(&format!("{{{name}}}"), &text_of(value));
                }
                // The only optional fragment: a chore's deadline.
                out.replace("{deadline}", "")
            }
            _ => {
                let params = self
                    .params
                    .iter()
                    .map(|(k, v)| format!("{k}={}", text_of(v)))
                    .collect::<Vec<_>>()
                    .join(", ");
                if params.is_empty() {
                    self.code.clone()
                } else {
                    format!("{} ({params})", self.code)
                }
            }
        };
        match &self.source {
            Source::Builtin => body,
            Source::Pack(p) => format!("[{p}] {body}"),
        }
    }

    /// Every built-in code that has a sentence template.
    pub fn known_codes() -> impl Iterator<Item = &'static str> {
        TEMPLATES.iter().map(|(c, _)| *c)
    }
}

impl ToCanon for ReasonCode {
    fn to_canon(&self) -> Canon {
        Canon::map([
            ("code", Canon::str(self.code.clone())),
            (
                "source",
                match &self.source {
                    Source::Builtin => Canon::str("builtin"),
                    Source::Pack(p) => Canon::str(format!("pack:{p}")),
                },
            ),
            ("params", Canon::Map(self.params.clone())),
        ])
    }
}

impl ReasonCode {
    /// Decodes the canonical form written by [`ToCanon`].
    pub fn from_reader(r: Reader<'_>) -> Result<ReasonCode, ReadError> {
        r.only(&["code", "source", "params"])?;
        let code = r.child("code")?.reader().str()?.to_owned();
        let source_child = r.child("source")?;
        let text = source_child.reader().str()?;
        let source = if text == "builtin" {
            Source::Builtin
        } else if let Some(pack) = text.strip_prefix("pack:") {
            Source::Pack(pack.to_owned())
        } else {
            return Err(source_child
                .reader()
                .err("expected 'builtin' or 'pack:<id>'"));
        };
        let params_child = r.child("params")?;
        let params = match params_child.reader().value() {
            Canon::Map(m) => m.clone(),
            _ => return Err(params_child.reader().err("expected an object")),
        };
        Ok(ReasonCode {
            code,
            source,
            params,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_reasons_render_their_sentence_with_parameters() {
        let r = ReasonCode::builtin(
            "duty_placed",
            [("slot", Canon::Int(18)), ("slots", Canon::Int(16))],
        );
        assert_eq!(
            r.explain(),
            "A required duty was placed at slot 18 for 16 slot(s)."
        );
        let r = ReasonCode::builtin(
            "urgent_need_placed",
            [
                ("need", Canon::str("hunger")),
                ("predicted", Canon::Int(30)),
                ("slot", Canon::Int(28)),
            ],
        );
        assert_eq!(
            r.explain(),
            "A restoring hunger activity was wanted by slot 30 and was placed at slot 28."
        );
    }

    #[test]
    fn missing_parameters_leave_their_placeholder_visible_rather_than_panicking() {
        let r = ReasonCode::builtin("duty_placed", [("slot", Canon::Int(1))]);
        assert!(r.explain().contains("{slots}"));
    }

    #[test]
    fn codes_without_parameters_and_unknown_codes_are_still_explained() {
        assert_eq!(
            ReasonCode::simple("path_blocked").explain(),
            "The route stayed blocked by other pawns."
        );
        assert_eq!(
            ReasonCode::simple("something_new").explain(),
            "something_new"
        );
        let r = ReasonCode::builtin(
            "something_new",
            [("a", Canon::Int(1)), ("b", Canon::str("x"))],
        );
        assert_eq!(r.explain(), "something_new (a=1, b=x)");
    }

    #[test]
    fn pack_decisions_name_their_pack() {
        let r = ReasonCode::from_pack("coffee_shop", "caffeine_rush", [("level", Canon::Int(7))]);
        assert_eq!(r.explain(), "[coffee_shop] caffeine_rush (level=7)");
        assert_eq!(
            r.to_canon().get("source").and_then(Canon::as_str),
            Some("pack:coffee_shop")
        );
        // A pack cannot impersonate a built-in code to borrow its sentence.
        let spoof = ReasonCode::from_pack("evil", "duty_placed", []);
        assert_eq!(spoof.explain(), "[evil] duty_placed");
    }

    #[test]
    fn every_template_is_unique_and_balanced() {
        let mut seen = std::collections::BTreeSet::new();
        for (code, template) in TEMPLATES {
            assert!(seen.insert(*code), "duplicate code {code}");
            assert_eq!(
                template.matches('{').count(),
                template.matches('}').count(),
                "{code}"
            );
        }
        assert!(ReasonCode::known_codes().count() >= 25);
    }

    #[test]
    fn canonical_form_is_stable() {
        let r = ReasonCode::builtin(
            "chore_placed",
            [
                ("slot", Canon::Int(3)),
                ("slots", Canon::Int(2)),
                ("urgency", Canon::Int(1)),
            ],
        );
        assert_eq!(
            r.to_canon().to_canonical_string(),
            r#"{"code":"chore_placed","params":{"slot":3,"slots":2,"urgency":1},"source":"builtin"}"#
        );
    }
    /// S-031: every built-in reason code used in the source has a sentence, and every sentence is used
    /// (apart from codes reserved for systems that do not exist yet).
    #[test]
    fn every_code_in_the_source_has_a_template_and_every_template_is_used() {
        const RESERVED: [&str; 3] = ["precondition_failed", "unaffordable", "unauthorized"];
        let mut used = std::collections::BTreeSet::new();
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![src];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if name == "reason.rs" || name == "tests.rs" || name.ends_with("_tests.rs") {
                        continue;
                    }
                    let full = std::fs::read_to_string(&path).unwrap();
                    // Only shipped code counts, not the test fixtures below it.
                    let text = full.split("#[cfg(test)]").next().unwrap_or("").to_owned();
                    for marker in ["ReasonCode::builtin(", "ReasonCode::simple("] {
                        for part in text.split(marker).skip(1) {
                            let rest = part.trim_start();
                            if let Some(lit) = rest.strip_prefix('"') {
                                if let Some(end) = lit.find('"') {
                                    used.insert(lit[..end].to_owned());
                                }
                            }
                        }
                    }
                    // Codes passed through helpers and variables still appear as string literals somewhere.
                    for known in ReasonCode::known_codes() {
                        if text.contains(&format!("\"{known}\"")) {
                            used.insert(known.to_owned());
                        }
                    }
                }
            }
        }
        let known: std::collections::BTreeSet<String> =
            ReasonCode::known_codes().map(str::to_owned).collect();
        let missing: Vec<&String> = used.iter().filter(|c| !known.contains(*c)).collect();
        assert!(
            missing.is_empty(),
            "codes used in the source but without a sentence: {missing:?}"
        );
        let unused: Vec<&String> = known
            .iter()
            .filter(|c| !used.contains(*c) && !RESERVED.contains(&c.as_str()))
            .collect();
        assert!(
            unused.is_empty(),
            "sentences with no code that uses them: {unused:?}"
        );
    }

    /// The base pack's English string table is the shipped copy of the sentence table; they must not drift.
    #[test]
    fn the_base_pack_english_strings_match_the_built_in_sentences() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../data/base/data/strings/en.json");
        let value = crate::canon::json::parse(&std::fs::read_to_string(path).unwrap()).unwrap();
        let Canon::Map(file) = value else {
            panic!("en.json is an object")
        };
        let built_in: BTreeMap<String, String> = TEMPLATES
            .iter()
            .map(|(c, t)| (format!("reason.{c}"), (*t).to_owned()))
            .collect();
        let shipped: BTreeMap<String, String> = file
            .iter()
            .filter(|(k, _)| k.starts_with("reason."))
            .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned()))
            .collect();
        assert_eq!(shipped, built_in);
    }

    #[test]
    fn sentences_come_from_the_active_string_table() {
        let mut strings = pg_content::Strings::new();
        let mut fr = BTreeMap::new();
        fr.insert(
            "reason.slots_open".to_owned(),
            "{count} créneau(x) restent libres.".to_owned(),
        );
        strings.add("fr", &fr);
        let r = ReasonCode::builtin("slots_open", [("count", Canon::Int(3))]);
        assert_eq!(r.explain_in(&strings, "fr"), "3 créneau(x) restent libres.");
        // A code the table lacks falls back to the built-in English.
        assert_eq!(r.explain_in(&strings, "de"), "3 slot(s) were left open.");
        // Pseudo needs English in the table; without it the built-in sentence still shows.
        assert_eq!(
            r.explain_in(&pg_content::Strings::new(), "pseudo"),
            "3 slot(s) were left open."
        );
        assert_eq!(r.explain(), "3 slot(s) were left open.");
    }
}
