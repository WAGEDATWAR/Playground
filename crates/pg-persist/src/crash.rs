//! Local crash reports (Blueprint §17, §24): a plain-text file written next to the saves, with every secret
//! removed. A report never leaves the machine on its own; the player may attach it to a bug report.
//!
//! The runtime's panic hook (0.8) calls [`write_report`]. Everything that goes into the report, including the
//! panic message (which can contain whatever a buggy `format!` put in it), the backtrace and the recent log
//! lines, passes through `redact` as one piece after assembly, so no field can be forgotten.

use pg_host::{redact, Secret, Storage};

pub struct CrashInfo<'a> {
    pub message: &'a str,
    pub location: Option<&'a str>,
    pub backtrace: Option<&'a str>,
    pub app_version: &'a str,
    pub os: &'a str,
    /// From the `Clock` service (wall clock is allowed for metadata).
    pub time_iso: &'a str,
    /// The last few log lines, oldest first.
    pub recent_log: &'a [String],
    /// Extra facts such as the world id, tick and content refs (never keys, but redacted regardless).
    pub context: &'a [(&'a str, &'a str)],
}

/// Longest report kept (a runaway backtrace should not fill the disk).
const MAX_REPORT_BYTES: usize = 256 * 1024;

pub fn build_report(info: &CrashInfo<'_>, secrets: &[&Secret]) -> String {
    let mut out = String::new();
    out.push_str("Playground crash report\n");
    out.push_str(&format!(
        "version: {}\nos: {}\ntime: {}\n",
        info.app_version, info.os, info.time_iso
    ));
    out.push_str(&format!("message: {}\n", info.message));
    if let Some(l) = info.location {
        out.push_str(&format!("location: {l}\n"));
    }
    for (k, v) in info.context {
        out.push_str(&format!("{k}: {v}\n"));
    }
    if let Some(b) = info.backtrace {
        out.push_str("\nbacktrace:\n");
        out.push_str(b);
        out.push('\n');
    }
    if !info.recent_log.is_empty() {
        out.push_str("\nrecent log:\n");
        for line in info.recent_log {
            out.push_str(line);
            out.push('\n');
        }
    }
    let mut clean = redact(&out, secrets);
    if clean.len() > MAX_REPORT_BYTES {
        let mut cut = MAX_REPORT_BYTES;
        while !clean.is_char_boundary(cut) {
            cut -= 1;
        }
        clean.truncate(cut);
        clean.push_str("\n[report truncated]\n");
    }
    clean
}

/// A file-name-safe version of a timestamp.
fn file_stamp(iso: &str) -> String {
    iso.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Builds the report and stores it as `crash/<time>.txt`. Returns the blob name.
pub fn write_report(
    storage: &dyn Storage,
    info: &CrashInfo<'_>,
    secrets: &[&Secret],
) -> Result<String, String> {
    let name = format!("crash/{}.txt", file_stamp(info.time_iso));
    storage
        .write_atomic(&name, build_report(info, secrets).as_bytes())
        .map_err(|e| e.to_string())?;
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_host::MemStorage;

    const KEY: &str = "sk-SENTINEL-0123456789abcdef";

    fn info<'a>(msg: &'a str, log: &'a [String], ctx: &'a [(&'a str, &'a str)]) -> CrashInfo<'a> {
        CrashInfo {
            message: msg,
            location: Some("crates/pg-core/src/x.rs:10:5"),
            backtrace: Some("0: foo\n1: bar"),
            app_version: "0.0.1",
            os: "windows",
            time_iso: "2026-10-07T01:02:03Z",
            recent_log: log,
            context: ctx,
        }
    }

    #[test]
    fn a_report_has_the_facts_a_bug_report_needs() {
        let r = build_report(
            &info(
                "index out of bounds",
                &["tick 5 ok".to_owned()],
                &[("world", "town")],
            ),
            &[],
        );
        for needle in [
            "version: 0.0.1",
            "os: windows",
            "time: 2026-10-07T01:02:03Z",
            "message: index out of bounds",
            "location: crates/pg-core/src/x.rs:10:5",
            "world: town",
            "0: foo",
            "tick 5 ok",
        ] {
            assert!(r.contains(needle), "{needle} missing from:\n{r}");
        }
    }

    #[test]
    fn keys_in_any_field_are_removed() {
        let secret = Secret::new("hunter2-not-key-shaped");
        let log = vec![
            format!("request failed: {KEY}"),
            "token=abcdef123456".to_owned(),
        ];
        let ctx = [("note", "the user pasted hunter2-not-key-shaped here")];
        let msg = format!("panic while sending Authorization: Bearer {KEY}");
        let r = build_report(&info(&msg, &log, &ctx), &[&secret]);
        for needle in ["SENTINEL", "hunter2", "abcdef123456"] {
            assert!(!r.contains(needle), "{needle} survived:\n{r}");
        }
        assert!(r.contains("[redacted]"));
    }

    #[test]
    fn reports_are_written_under_crash_with_a_safe_name_and_are_bounded() {
        let mem = MemStorage::new();
        let name = write_report(&mem, &info("boom", &[], &[]), &[]).unwrap();
        assert_eq!(name, "crash/2026-10-07T01-02-03Z.txt");
        assert!(String::from_utf8(mem.get_raw(&name).unwrap())
            .unwrap()
            .contains("message: boom"));
        let huge = "é".repeat(300_000);
        let r = build_report(&info(&huge, &[], &[]), &[]);
        assert!(r.len() <= MAX_REPORT_BYTES + 64 && r.ends_with("[report truncated]\n"));
        assert!(write_report(&mem, &info("x", &[], &[]), &[]).is_ok());
        let weird = CrashInfo {
            time_iso: "../../etc/passwd",
            ..info("x", &[], &[])
        };
        let n = write_report(&mem, &weird, &[]).unwrap();
        assert!(n.starts_with("crash/") && !n.contains(".."), "{n}");
    }
}
