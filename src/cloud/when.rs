//! How cloud commands show a server timestamp (DEC-AU6, I-219).
//!
//! Four copies of these lived in `session`, `device` and `token`, and `org
//! invites` had none, so it printed `2026-10-20T09:00:00.123456Z`.

/// An RFC 3339 timestamp as `YYYY-MM-DD HH:MM UTC`, as the server sends it.
///
/// ⚠️ The zone is said (RM-85, F5): a production server in KST read `16:21`
/// as local time, nine hours off. A date alone stays a date.
pub(crate) fn stamp(ts: &str) -> String {
    let short: String = ts.replace('T', " ").chars().take(16).collect();
    if ts.contains('T') {
        format!("{short} UTC")
    } else {
        short
    }
}

/// An RFC 3339 timestamp's date, `YYYY-MM-DD`.
pub(crate) fn date(ts: &str) -> String {
    ts.split('T').next().unwrap_or(ts).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One table for every shape the server sends.
    #[test]
    fn t_i219_timestamps_read_as_people_write_them() {
        for (raw, s, d) in [
            (
                "2026-10-20T09:00:00.123456Z",
                "2026-10-20 09:00 UTC",
                "2026-10-20",
            ),
            ("2026-10-20T09:00:00Z", "2026-10-20 09:00 UTC", "2026-10-20"),
            (
                "2026-10-20T09:00:00+00:00",
                "2026-10-20 09:00 UTC",
                "2026-10-20",
            ),
            ("2026-10-20", "2026-10-20", "2026-10-20"),
            ("", "", ""),
        ] {
            assert_eq!(stamp(raw), s, "{raw:?}");
            assert_eq!(date(raw), d, "{raw:?}");
        }
    }
}
