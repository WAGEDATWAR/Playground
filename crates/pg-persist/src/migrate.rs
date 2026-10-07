//! Save migrations (Blueprint §13.5).
//!
//! `schema` is an integer. A migration is a pure function from the raw canonical value of schema `N` to
//! the raw value of schema `N+1`; running from any older schema applies them in sequence. Saves from a
//! **newer** schema than this build knows are refused, never partially loaded. Migrations are
//! additive-first and never change ids.
//!
//! The world schema is 3 and no older schema ever shipped, so the real chain is empty; the framework is
//! exercised by test chains and by the pinned v3 fixture.

use pg_core::canon::Canon;
use std::fmt;

/// One step: schema `from` -> `from + 1`.
#[derive(Clone)]
pub struct Migration {
    pub from: u32,
    pub describe: &'static str,
    pub apply: fn(Canon) -> Result<Canon, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MigrateError {
    /// The save is from a newer build.
    Newer { found: u32, current: u32 },
    /// No migration path from this schema.
    TooOld { found: u32, oldest: u32 },
    /// A step reported an error, or produced something that is not an object.
    Failed { from: u32, message: String },
}

impl fmt::Display for MigrateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MigrateError::Newer { found, current } => write!(
                f,
                "this save was made by a newer version of Playground (schema {found}; this version reads up to {current})"
            ),
            MigrateError::TooOld { found, oldest } => write!(
                f,
                "this save is too old to open (schema {found}; the oldest supported is {oldest})"
            ),
            MigrateError::Failed { from, message } => {
                write!(f, "migrating from schema {from} failed: {message}")
            }
        }
    }
}

impl std::error::Error for MigrateError {}

#[derive(Clone)]
pub struct Migrations {
    current: u32,
    steps: Vec<Migration>,
}

impl Migrations {
    /// A chain ending at `current`. Steps must be contiguous (`from` ascending by one) and end at
    /// `current - 1`; anything else is a programming error and is reported as such.
    pub fn new(current: u32, steps: Vec<Migration>) -> Result<Migrations, String> {
        for (i, pair) in steps.windows(2).enumerate() {
            if let [a, b] = pair {
                if b.from != a.from + 1 {
                    return Err(format!(
                        "migration {i} ({}) is not followed by {}",
                        a.from,
                        a.from + 1
                    ));
                }
            }
        }
        if let Some(last) = steps.last() {
            if last.from + 1 != current {
                return Err(format!(
                    "the last migration is from {} but the current schema is {current}",
                    last.from
                ));
            }
        }
        Ok(Migrations { current, steps })
    }

    /// The chain this build ships: empty, because schema 3 is the first one that ever existed.
    pub fn builtin() -> Migrations {
        Migrations {
            current: pg_core::world::SCHEMA_VERSION,
            steps: Vec::new(),
        }
    }

    pub fn current(&self) -> u32 {
        self.current
    }

    /// The oldest schema this chain can bring forward.
    pub fn oldest(&self) -> u32 {
        self.steps.first().map_or(self.current, |s| s.from)
    }

    /// Brings `raw` (written at schema `from`) up to the current schema.
    pub fn migrate(&self, mut raw: Canon, from: u32) -> Result<Canon, MigrateError> {
        if from > self.current {
            return Err(MigrateError::Newer {
                found: from,
                current: self.current,
            });
        }
        if from < self.oldest() {
            return Err(MigrateError::TooOld {
                found: from,
                oldest: self.oldest(),
            });
        }
        for step in self.steps.iter().filter(|s| s.from >= from) {
            raw = (step.apply)(raw).map_err(|message| MigrateError::Failed {
                from: step.from,
                message,
            })?;
            match &mut raw {
                Canon::Map(m) => {
                    m.insert("schema".to_owned(), Canon::Int(i128::from(step.from) + 1));
                }
                _ => {
                    return Err(MigrateError::Failed {
                        from: step.from,
                        message: "the migration did not produce an object".to_owned(),
                    })
                }
            }
        }
        Ok(raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add_field(mut c: Canon) -> Result<Canon, String> {
        if let Canon::Map(m) = &mut c {
            m.insert("added_in_2".into(), Canon::Int(7));
        }
        Ok(c)
    }

    fn rename_field(mut c: Canon) -> Result<Canon, String> {
        if let Canon::Map(m) = &mut c {
            let v = m.remove("added_in_2").ok_or("added_in_2 is missing")?;
            m.insert("renamed".into(), v);
        }
        Ok(c)
    }

    fn explode(_: Canon) -> Result<Canon, String> {
        Err("boom".into())
    }

    fn chain() -> Migrations {
        Migrations::new(
            3,
            vec![
                Migration {
                    from: 1,
                    describe: "add a field",
                    apply: add_field,
                },
                Migration {
                    from: 2,
                    describe: "rename it",
                    apply: rename_field,
                },
            ],
        )
        .unwrap()
    }

    fn doc(schema: i128) -> Canon {
        Canon::map([("schema", Canon::Int(schema)), ("name", Canon::str("x"))])
    }

    #[test]
    fn steps_apply_in_sequence_from_any_older_schema() {
        let from1 = chain().migrate(doc(1), 1).unwrap();
        assert_eq!(from1.get("renamed"), Some(&Canon::Int(7)));
        assert_eq!(from1.get("schema"), Some(&Canon::Int(3)));
        // From schema 2 only the second step runs; its input already has the first step's field.
        let mut d2 = doc(2);
        if let Canon::Map(m) = &mut d2 {
            m.insert("added_in_2".into(), Canon::Int(9));
        }
        let from2 = chain().migrate(d2, 2).unwrap();
        assert_eq!(from2.get("renamed"), Some(&Canon::Int(9)));
        // Already current: untouched.
        assert_eq!(chain().migrate(doc(3), 3).unwrap(), doc(3));
    }

    #[test]
    fn newer_and_too_old_saves_are_refused_with_clear_messages() {
        let e = chain().migrate(doc(4), 4).unwrap_err();
        assert_eq!(
            e,
            MigrateError::Newer {
                found: 4,
                current: 3
            }
        );
        assert!(e.to_string().contains("newer version"));
        let e = chain().migrate(doc(0), 0).unwrap_err();
        assert_eq!(
            e,
            MigrateError::TooOld {
                found: 0,
                oldest: 1
            }
        );
        assert!(e.to_string().contains("too old"));
    }

    #[test]
    fn a_failing_step_refuses_the_load() {
        let m = Migrations::new(
            2,
            vec![Migration {
                from: 1,
                describe: "bad",
                apply: explode,
            }],
        )
        .unwrap();
        assert_eq!(
            m.migrate(doc(1), 1).unwrap_err(),
            MigrateError::Failed {
                from: 1,
                message: "boom".into()
            }
        );
    }

    #[test]
    fn chains_must_be_contiguous_and_end_at_the_current_schema() {
        let m = |from| Migration {
            from,
            describe: "",
            apply: add_field,
        };
        assert!(Migrations::new(4, vec![m(1), m(3)]).is_err());
        assert!(Migrations::new(5, vec![m(1), m(2)]).is_err());
        assert!(Migrations::new(3, vec![m(1), m(2)]).is_ok());
        assert!(Migrations::new(3, vec![]).is_ok());
    }

    #[test]
    fn the_shipped_chain_reads_only_the_current_schema() {
        let b = Migrations::builtin();
        assert_eq!(b.oldest(), b.current());
        assert!(b.migrate(doc(2), 2).is_err());
        assert!(b.migrate(doc(i128::from(b.current())), b.current()).is_ok());
    }
}
