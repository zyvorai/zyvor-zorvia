// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Support timers: durations counted only inside contract coverage hours,
//! evaluated in the contract's IANA timezone, skipping contract holidays.
//!
//! Pause rules (applied by the case store, using these primitives):
//! the resolution-estimate clock pauses while a case is
//! `waiting_on_customer` and is rebuilt on resume from the *remaining
//! covered seconds* at the moment it paused.

use super::model::CoverageHours;
use chrono::{
    DateTime, Datelike, Duration, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc,
};
use chrono_tz::Tz;

/// Upper bound on how far ahead a timer is searched. Guards against a
/// schedule with no usable days looping forever.
const MAX_DAYS: u32 = 3700;

#[derive(Debug, Clone)]
pub struct Schedule {
    tz: Tz,
    hours: CoverageHours,
    holidays: Vec<NaiveDate>,
}

fn hhmm(s: &str) -> Option<NaiveTime> {
    let (h, m) = s.split_once(':')?;
    NaiveTime::from_hms_opt(h.parse().ok()?, m.parse().ok()?, 0)
}

impl Schedule {
    pub fn new(timezone: &str, hours: &CoverageHours, holidays: &[String]) -> Result<Self, String> {
        let tz: Tz = timezone
            .parse()
            .map_err(|_| format!("unknown timezone '{timezone}'"))?;
        let holidays = holidays
            .iter()
            .map(|h| {
                NaiveDate::parse_from_str(h, "%Y-%m-%d").map_err(|_| format!("bad holiday '{h}'"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            tz,
            hours: hours.clone(),
            holidays,
        })
    }

    /// A schedule that is always on (24x7), used for always-on severities.
    pub fn always(timezone: &str) -> Result<Self, String> {
        Self::new(
            timezone,
            &CoverageHours {
                always: true,
                ..Default::default()
            },
            &[],
        )
    }

    fn to_utc(&self, local: NaiveDateTime) -> Option<DateTime<Utc>> {
        match self.tz.from_local_datetime(&local) {
            LocalResult::Single(t) | LocalResult::Ambiguous(t, _) => Some(t.with_timezone(&Utc)),
            // Nonexistent local time (DST gap): use the same wall time an
            // hour later, which exists.
            LocalResult::None => self
                .tz
                .from_local_datetime(&(local + Duration::hours(1)))
                .earliest()
                .map(|t| t.with_timezone(&Utc)),
        }
    }

    /// Covered window for one local calendar date, as UTC instants.
    fn window(&self, date: NaiveDate) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
        if self.hours.always {
            let start = self.to_utc(date.and_hms_opt(0, 0, 0)?)?;
            let end = self.to_utc(date.succ_opt()?.and_hms_opt(0, 0, 0)?)?;
            return Some((start, end));
        }
        let weekday = u8::try_from(date.weekday().number_from_monday()).ok()?;
        if !self.hours.days.contains(&weekday) || self.holidays.contains(&date) {
            return None;
        }
        let start = self.to_utc(date.and_time(hhmm(&self.hours.start)?))?;
        let end = self.to_utc(date.and_time(hhmm(&self.hours.end)?))?;
        (start < end).then_some((start, end))
    }

    fn local_date(&self, t: DateTime<Utc>) -> NaiveDate {
        t.with_timezone(&self.tz).date_naive()
    }

    /// The instant at which `seconds` of covered time have elapsed after
    /// `start`. `None` when the schedule never provides that much time.
    pub fn add_covered_seconds(&self, start: DateTime<Utc>, seconds: i64) -> Option<DateTime<Utc>> {
        if seconds <= 0 {
            return Some(start);
        }
        let mut remaining = seconds;
        let mut date = self.local_date(start);
        for _ in 0..MAX_DAYS {
            if let Some((ws, we)) = self.window(date) {
                let from = ws.max(start);
                if from < we {
                    let available = (we - from).num_seconds();
                    if remaining <= available {
                        return Some(from + Duration::seconds(remaining));
                    }
                    remaining -= available;
                }
            }
            date = date.succ_opt()?;
        }
        None
    }

    /// Covered seconds in `[from, to]`.
    pub fn covered_seconds_between(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> i64 {
        if to <= from {
            return 0;
        }
        let mut total = 0;
        let mut date = self.local_date(from);
        let last = self.local_date(to);
        for _ in 0..MAX_DAYS {
            if date > last {
                break;
            }
            if let Some((ws, we)) = self.window(date) {
                let s = ws.max(from);
                let e = we.min(to);
                if s < e {
                    total += (e - s).num_seconds();
                }
            }
            match date.succ_opt() {
                Some(d) => date = d,
                None => break,
            }
        }
        total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn business(tz: &str, holidays: &[&str]) -> Schedule {
        Schedule::new(
            tz,
            &CoverageHours {
                always: false,
                days: vec![1, 2, 3, 4, 5],
                start: "09:00".into(),
                end: "17:00".into(),
            },
            &holidays.iter().map(|h| h.to_string()).collect::<Vec<_>>(),
        )
        .unwrap()
    }

    #[test]
    fn friday_afternoon_rolls_to_monday() {
        // Fri 2026-01-09 16:00 Berlin (15:00Z) + 2 covered hours:
        // 1h left on Friday, 1h on Monday from 09:00 -> Mon 10:00 Berlin.
        let s = business("Europe/Berlin", &[]);
        let due = s
            .add_covered_seconds(utc("2026-01-09T15:00:00Z"), 7200)
            .unwrap();
        assert_eq!(due, utc("2026-01-12T09:00:00Z"));
    }

    #[test]
    fn holiday_is_skipped() {
        let s = business("Europe/Berlin", &["2026-01-12"]);
        let due = s
            .add_covered_seconds(utc("2026-01-09T15:00:00Z"), 7200)
            .unwrap();
        assert_eq!(due, utc("2026-01-13T09:00:00Z"));
    }

    #[test]
    fn timezone_changes_the_answer() {
        let ber = business("Europe/Berlin", &[]);
        let nyc = business("America/New_York", &[]);
        let t = utc("2026-01-12T07:30:00Z"); // 08:30 Berlin (before open), 02:30 New York
        let b = ber.add_covered_seconds(t, 3600).unwrap();
        let n = nyc.add_covered_seconds(t, 3600).unwrap();
        assert_eq!(b, utc("2026-01-12T09:00:00Z")); // opens 09:00 Berlin, +1h = 10:00
        assert_eq!(n, utc("2026-01-12T15:00:00Z")); // opens 09:00 NY (14:00Z), +1h
        assert_ne!(b, n);
    }

    #[test]
    fn weekend_start_waits_for_monday() {
        let s = business("Europe/Berlin", &[]);
        let due = s
            .add_covered_seconds(utc("2026-01-10T12:00:00Z"), 3600)
            .unwrap();
        assert_eq!(due, utc("2026-01-12T09:00:00Z"));
    }

    #[test]
    fn always_on_ignores_hours_and_holidays() {
        let s = Schedule::always("Europe/Berlin").unwrap();
        let due = s
            .add_covered_seconds(utc("2026-01-10T12:00:00Z"), 3600)
            .unwrap();
        assert_eq!(due, utc("2026-01-10T13:00:00Z"));
    }

    #[test]
    fn covered_between_counts_only_covered_time() {
        let s = business("Europe/Berlin", &[]);
        // Fri 16:00 Berlin -> Mon 10:00 Berlin: 1h Friday + 1h Monday.
        assert_eq!(
            s.covered_seconds_between(utc("2026-01-09T15:00:00Z"), utc("2026-01-12T09:00:00Z")),
            7200
        );
        assert_eq!(
            s.covered_seconds_between(utc("2026-01-12T09:00:00Z"), utc("2026-01-09T15:00:00Z")),
            0
        );
    }

    #[test]
    fn pause_and_resume_preserves_remaining_time() {
        let s = business("Europe/Berlin", &[]);
        let start = utc("2026-01-12T08:00:00Z"); // Mon 09:00 Berlin
        let due = s.add_covered_seconds(start, 4 * 3600).unwrap();
        let pause_at = utc("2026-01-12T09:00:00Z"); // 1h used
        let remaining = s.covered_seconds_between(pause_at, due);
        assert_eq!(remaining, 3 * 3600);
        // Customer answers Wed 10:00 Berlin; 3 covered hours remain.
        let resume = utc("2026-01-14T09:00:00Z");
        assert_eq!(
            s.add_covered_seconds(resume, remaining).unwrap(),
            utc("2026-01-14T12:00:00Z")
        );
    }

    #[test]
    fn dst_spring_forward_uses_real_local_hours() {
        // US DST began Sun 2026-03-08. Mon 09:00 New York is 13:00Z (EDT).
        let s = business("America/New_York", &[]);
        let due = s
            .add_covered_seconds(utc("2026-03-09T13:00:00Z"), 3600)
            .unwrap();
        assert_eq!(due, utc("2026-03-09T14:00:00Z"));
    }

    #[test]
    fn empty_schedule_terminates() {
        let s = Schedule::new(
            "UTC",
            &CoverageHours {
                always: false,
                days: vec![],
                start: "09:00".into(),
                end: "17:00".into(),
            },
            &[],
        )
        .unwrap();
        assert!(s.add_covered_seconds(Utc::now(), 60).is_none());
    }

    #[test]
    fn bad_inputs_rejected() {
        assert!(Schedule::always("Mars/Base").is_err());
        assert!(Schedule::new("UTC", &CoverageHours::default(), &["nope".into()]).is_err());
    }
}
