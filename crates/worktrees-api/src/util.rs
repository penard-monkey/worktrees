//! Small pure helpers shared across the command layer.

/// epoch → "YYYY-MM-DD HH:MM:SS" UTC (civil-from-days; avoids a chrono dep).
pub fn fmt_utc(epoch: i64) -> String {
    let (days, secs) = (epoch.div_euclid(86_400), epoch.rem_euclid(86_400));
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    // Howard Hinnant's civil_from_days
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mth = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mth <= 2 { y + 1 } else { y };
    format!("{y:04}-{mth:02}-{d:02} {h:02}:{m:02}:{s:02}")
}

#[cfg(test)]
mod tests {
    #[test]
    fn fmt_utc_is_civil_from_days() {
        assert_eq!(super::fmt_utc(0), "1970-01-01 00:00:00");
        // A leap day, which the era arithmetic is the whole reason for.
        assert_eq!(super::fmt_utc(1_709_164_800), "2024-02-29 00:00:00");
    }
}
