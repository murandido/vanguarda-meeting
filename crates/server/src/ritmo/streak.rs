//! Streak math (KB-06), kept pure: the inputs are check-in days, pauses and
//! "today", all as local civil dates, so every rule can be unit-tested with
//! fixed dates.
//!
//! Rules, applied day by day from the first check-in of anyone in the pair:
//!
//! * A day with a check-in adds 1 to the streak. Every [`FREEZE_EVERY`]
//!   streak days earns one freeze, up to [`MAX_FREEZES`] banked.
//! * A paused day is neutral: it neither breaks nor extends the streak, even
//!   if there is a check-in on it.
//! * A missed day consumes a banked freeze automatically (neutral as well);
//!   with no freeze left, the streak goes back to 0.
//! * Today without a check-in is still open: it never breaks anything.
//!
//! The pair streak counts the days where everyone checked in. A day where
//! someone was paused or covered by a freeze is neutral for the pair too, and
//! a day where anyone broke their streak breaks the pair's.
//!
//! Nothing here is stored: the server only persists the inputs (check-ins,
//! pauses) and the freeze events derived from them, as history.

use std::collections::BTreeSet;

use super::date::Date;

/// Streak days needed to earn one freeze.
pub const FREEZE_EVERY: u32 = 7;
/// Maximum number of freezes a person can have banked.
pub const MAX_FREEZES: u8 = 2;
/// Longest pause that can be scheduled at once, in days.
pub const MAX_PAUSE_DAYS: i64 = 30;

/// A planned break (travel, sickness), inclusive on both ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pause {
    pub start: Date,
    pub end: Date,
}

impl Pause {
    #[must_use]
    pub fn contains(&self, day: Date) -> bool {
        self.start <= day && day <= self.end
    }
}

/// What a single day meant for someone's streak.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Checked in: the streak grew.
    CheckedIn,
    /// Inside a pause: neutral.
    Paused,
    /// Missed, but a freeze covered it: neutral.
    Frozen,
    /// Missed with no freeze left: the streak broke.
    Missed,
    /// Today, still without a check-in.
    Pending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    FreezeEarned,
    FreezeUsed,
}

impl EventKind {
    /// Value stored in `ritmo_streak_events.kind`.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::FreezeEarned => "freeze_earned",
            Self::FreezeUsed => "freeze_used",
        }
    }
}

/// A freeze being earned or used, on a given day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event {
    pub day: Date,
    pub kind: EventKind,
}

/// Someone's streak as of today.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Streak {
    pub current: u32,
    pub best: u32,
    pub freezes: u8,
    pub checked_in_today: bool,
    pub paused_today: bool,
    pub events: Vec<Event>,
}

/// The pair's streak as of today.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PairStreak {
    pub current: u32,
    pub best: u32,
    pub everyone_today: bool,
}

/// Everything one person contributes to the computation.
#[derive(Debug, Clone, Default)]
pub struct History {
    pub checkins: BTreeSet<Date>,
    pub pauses: Vec<Pause>,
}

/// Day-by-day outcomes for one person, from `from` to `today` inclusive.
fn timeline(history: &History, from: Date, today: Date) -> (Vec<Outcome>, Streak) {
    let mut streak = Streak::default();
    let mut days = Vec::new();
    let mut day = from;
    while day <= today {
        let paused = history.pauses.iter().any(|p| p.contains(day));
        let checked = history.checkins.contains(&day);
        let outcome = if paused {
            Outcome::Paused
        } else if checked {
            streak.current += 1;
            if streak.current.is_multiple_of(FREEZE_EVERY) && streak.freezes < MAX_FREEZES {
                streak.freezes += 1;
                streak.events.push(Event {
                    day,
                    kind: EventKind::FreezeEarned,
                });
            }
            Outcome::CheckedIn
        } else if day == today {
            Outcome::Pending
        } else if streak.freezes > 0 {
            streak.freezes -= 1;
            streak.events.push(Event {
                day,
                kind: EventKind::FreezeUsed,
            });
            Outcome::Frozen
        } else {
            streak.current = 0;
            Outcome::Missed
        };
        streak.best = streak.best.max(streak.current);
        days.push(outcome);
        day = day.add_days(1);
    }
    streak.checked_in_today = history.checkins.contains(&today);
    streak.paused_today = history.pauses.iter().any(|p| p.contains(today));
    (days, streak)
}

/// What a day meant for the pair, given everyone's outcome on it.
fn pair_outcome(outcomes: &[Outcome]) -> Outcome {
    if outcomes.contains(&Outcome::Missed) {
        Outcome::Missed
    } else if outcomes.iter().all(|o| *o == Outcome::CheckedIn) {
        Outcome::CheckedIn
    } else if outcomes.contains(&Outcome::Pending) {
        Outcome::Pending
    } else {
        Outcome::Paused
    }
}

/// Streaks of each person (same order as `people`) and of the pair.
#[must_use]
pub fn compute(people: &[History], today: Date) -> (Vec<Streak>, PairStreak) {
    let Some(from) = people
        .iter()
        .filter_map(|h| h.checkins.first().copied())
        .min()
        .filter(|first| *first <= today)
    else {
        return (vec![Streak::default(); people.len()], PairStreak::default());
    };

    let (timelines, streaks): (Vec<_>, Vec<_>) =
        people.iter().map(|h| timeline(h, from, today)).unzip();

    let mut pair = PairStreak::default();
    if !people.is_empty() {
        for i in 0..timelines[0].len() {
            let day: Vec<Outcome> = timelines.iter().map(|t| t[i]).collect();
            match pair_outcome(&day) {
                Outcome::CheckedIn => pair.current += 1,
                Outcome::Missed => pair.current = 0,
                Outcome::Paused | Outcome::Frozen | Outcome::Pending => {}
            }
            pair.best = pair.best.max(pair.current);
        }
        pair.everyone_today = streaks.iter().all(|s| s.checked_in_today);
    }
    (streaks, pair)
}

/// Why a pause cannot be scheduled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseError {
    /// Pauses are planned ahead: they must start after today.
    NotAhead,
    /// The end comes before the start.
    Backwards,
    /// Longer than [`MAX_PAUSE_DAYS`].
    TooLong,
    /// Overlaps another pause of the same person.
    Overlaps,
}

/// Checks a new pause against today and the person's existing pauses.
pub fn validate_pause(new: Pause, today: Date, existing: &[Pause]) -> Result<(), PauseError> {
    if new.start <= today {
        return Err(PauseError::NotAhead);
    }
    if new.end < new.start {
        return Err(PauseError::Backwards);
    }
    if new.start.add_days(MAX_PAUSE_DAYS - 1) < new.end {
        return Err(PauseError::TooLong);
    }
    if existing
        .iter()
        .any(|p| p.start <= new.end && new.start <= p.end)
    {
        return Err(PauseError::Overlaps);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Date {
        Date::parse(s).unwrap()
    }

    /// Consecutive days starting at `start`.
    fn run(start: &str, n: i64) -> impl Iterator<Item = Date> {
        let start = d(start);
        (0..n).map(move |i| start.add_days(i))
    }

    fn history(days: impl IntoIterator<Item = Date>) -> History {
        History {
            checkins: days.into_iter().collect(),
            pauses: Vec::new(),
        }
    }

    fn pause(start: &str, end: &str) -> Pause {
        Pause {
            start: d(start),
            end: d(end),
        }
    }

    fn solo(h: &History, today: &str) -> Streak {
        compute(std::slice::from_ref(h), d(today)).0.remove(0)
    }

    #[test]
    fn nobody_checked_in_yet() {
        let (streaks, pair) = compute(&[History::default(), History::default()], d("2026-10-20"));
        assert_eq!(streaks, vec![Streak::default(); 2]);
        assert_eq!(pair, PairStreak::default());
        assert_eq!(compute(&[], d("2026-10-20")).1, PairStreak::default());
    }

    #[test]
    fn consecutive_days_count() {
        let h = history(run("2026-10-14", 3));
        let s = solo(&h, "2026-10-16");
        assert_eq!((s.current, s.best, s.freezes), (3, 3, 0));
        assert!(s.checked_in_today);
    }

    #[test]
    fn today_without_checkin_is_still_open() {
        let h = history(run("2026-10-14", 3));
        let s = solo(&h, "2026-10-17");
        assert_eq!(s.current, 3);
        assert!(!s.checked_in_today);
        // The next day, the missed 17th breaks it.
        assert_eq!(solo(&h, "2026-10-18").current, 0);
        assert_eq!(solo(&h, "2026-10-18").best, 3);
    }

    #[test]
    fn a_gap_resets_the_streak() {
        let h = history(run("2026-10-01", 4).chain(run("2026-10-06", 2)));
        let s = solo(&h, "2026-10-07");
        assert_eq!((s.current, s.best), (2, 4));
        assert_eq!(s.events, Vec::new());
    }

    #[test]
    fn checkins_in_the_future_are_ignored() {
        let h = history([d("2026-10-25")]);
        assert_eq!(solo(&h, "2026-10-20"), Streak::default());
    }

    #[test]
    fn seven_days_earn_a_freeze() {
        let h = history(run("2026-10-01", 7));
        let s = solo(&h, "2026-10-07");
        assert_eq!((s.current, s.freezes), (7, 1));
        assert_eq!(
            s.events,
            vec![Event {
                day: d("2026-10-07"),
                kind: EventKind::FreezeEarned
            }]
        );
        // Six days are not enough.
        assert_eq!(
            solo(&history(run("2026-10-01", 6)), "2026-10-06").freezes,
            0
        );
    }

    #[test]
    fn a_missed_day_uses_a_freeze_instead_of_breaking() {
        // 7 days, miss the 8th, check in on the 9th.
        let h = history(run("2026-10-01", 7).chain([d("2026-10-09")]));
        let s = solo(&h, "2026-10-09");
        assert_eq!((s.current, s.freezes), (8, 0));
        assert_eq!(
            s.events[1],
            Event {
                day: d("2026-10-08"),
                kind: EventKind::FreezeUsed
            }
        );
    }

    #[test]
    fn freezes_run_out() {
        // 7 days (1 freeze), then two missed days: first frozen, second breaks.
        let h = history(run("2026-10-01", 7).chain([d("2026-10-10")]));
        let s = solo(&h, "2026-10-10");
        assert_eq!((s.current, s.best, s.freezes), (1, 7, 0));
        assert_eq!(s.events.len(), 2);
    }

    #[test]
    fn freezes_are_capped() {
        // 28 days would earn 4, but only 2 can be banked.
        let h = history(run("2026-10-01", 28));
        let s = solo(&h, "2026-10-28");
        assert_eq!((s.current, s.freezes), (28, MAX_FREEZES));
        assert_eq!(s.events.len(), 2);
        // Two missed days are covered; the third breaks.
        let s = solo(&h, "2026-10-31");
        assert_eq!((s.current, s.freezes), (28, 0));
        let s = solo(&h, "2026-11-01");
        assert_eq!((s.current, s.best), (0, 28));
    }

    #[test]
    fn freeze_days_do_not_count_toward_the_next_freeze() {
        // 7 days, 1 frozen, then 6 more: streak 13, no new freeze yet.
        let h = history(run("2026-10-01", 7).chain(run("2026-10-09", 6)));
        let s = solo(&h, "2026-10-14");
        assert_eq!((s.current, s.freezes), (13, 0));
        // The 14th streak day earns the next one.
        let h = history(run("2026-10-01", 7).chain(run("2026-10-09", 7)));
        assert_eq!(solo(&h, "2026-10-15").freezes, 1);
    }

    #[test]
    fn a_used_freeze_can_be_earned_again_after_the_cap() {
        // 14 days (2 freezes), miss one, 7 more days earn it back.
        let h = history(run("2026-10-01", 14).chain(run("2026-10-16", 7)));
        let s = solo(&h, "2026-10-22");
        assert_eq!((s.current, s.freezes), (21, 2));
    }

    #[test]
    fn pauses_neither_break_nor_extend() {
        let mut h = history(run("2026-10-01", 3).chain(run("2026-10-08", 2)));
        h.pauses.push(pause("2026-10-04", "2026-10-07"));
        let s = solo(&h, "2026-10-09");
        assert_eq!((s.current, s.freezes), (5, 0));
        assert_eq!(s.events, Vec::new());

        // A check-in on a paused day does not count either.
        h.checkins.insert(d("2026-10-05"));
        assert_eq!(solo(&h, "2026-10-09").current, 5);
    }

    #[test]
    fn pauses_keep_freezes_banked() {
        let mut h = history(run("2026-10-01", 7).chain([d("2026-10-11")]));
        h.pauses.push(pause("2026-10-08", "2026-10-10"));
        let s = solo(&h, "2026-10-11");
        assert_eq!((s.current, s.freezes), (8, 1));
        assert_eq!(s.events.len(), 1);
    }

    #[test]
    fn paused_today_is_reported() {
        let mut h = history(run("2026-10-01", 2));
        h.pauses.push(pause("2026-10-03", "2026-10-05"));
        let s = solo(&h, "2026-10-04");
        assert!(s.paused_today);
        assert_eq!(s.current, 2);
        // After the pause, missing the first day breaks it.
        assert_eq!(solo(&h, "2026-10-07").current, 0);
    }

    #[test]
    fn pair_counts_days_where_both_checked_in() {
        let ana = history(run("2026-10-01", 5));
        let bia = history(run("2026-10-03", 3));
        let (s, pair) = compute(&[ana, bia], d("2026-10-05"));
        assert_eq!((s[0].current, s[1].current), (5, 3));
        assert_eq!((pair.current, pair.best), (3, 3));
        assert!(pair.everyone_today);
    }

    #[test]
    fn pair_waits_for_both_today() {
        let ana = history(run("2026-10-01", 3));
        let bia = history(run("2026-10-01", 2));
        let (_, pair) = compute(&[ana.clone(), bia.clone()], d("2026-10-03"));
        assert_eq!(pair.current, 2);
        assert!(!pair.everyone_today);
        // The next day, Bia's missed 3rd broke both her streak and the pair's.
        let (s, pair) = compute(&[ana, bia], d("2026-10-04"));
        assert_eq!((s[0].current, s[1].current, pair.current), (3, 0, 0));
        assert_eq!(pair.best, 2);
    }

    #[test]
    fn pair_survives_a_freeze_or_a_pause_of_one_person() {
        let ana = history(run("2026-10-01", 10));
        // Bia: 7 days, misses the 8th (freeze), checks in 9th and 10th.
        let bia = history(run("2026-10-01", 7).chain(run("2026-10-09", 2)));
        let (_, pair) = compute(&[ana.clone(), bia], d("2026-10-10"));
        assert_eq!(pair.current, 9);

        // Bia paused the 8th instead.
        let mut bia = history(run("2026-10-01", 7).chain(run("2026-10-09", 2)));
        bia.pauses.push(pause("2026-10-08", "2026-10-08"));
        let (s, pair) = compute(&[ana, bia], d("2026-10-10"));
        assert_eq!(pair.current, 9);
        assert_eq!(s[1].freezes, 1);
    }

    #[test]
    fn pair_breaks_when_anyone_breaks() {
        let ana = history(run("2026-10-01", 6));
        let bia = history(run("2026-10-01", 3).chain(run("2026-10-05", 2)));
        let (_, pair) = compute(&[ana, bia], d("2026-10-06"));
        assert_eq!((pair.current, pair.best), (2, 3));
    }

    #[test]
    fn days_before_someones_first_checkin_break_the_pair() {
        // Bia never checked in: the pair has nothing.
        let (_, pair) = compute(
            &[history(run("2026-10-01", 4)), History::default()],
            d("2026-10-04"),
        );
        assert_eq!(pair, PairStreak::default());
    }

    #[test]
    fn local_day_boundary_at_minus_three_hours() {
        // Brasília is UTC-3 all year (no DST since 2019), so a check-in at
        // 23:59 local on the 20th is 02:59 UTC on the 21st and still belongs
        // to the 20th; 00:00 local on the 21st is exactly 03:00 UTC.
        const OFFSET: i64 = -180;
        let late_20th = 1_792_551_540; // 2026-10-21 02:59 UTC
        let midnight_21st = 1_792_551_600; // 2026-10-21 03:00 UTC
        assert_eq!(Date::from_unix(late_20th, OFFSET), d("2026-10-20"));
        assert_eq!(Date::from_unix(midnight_21st, OFFSET), d("2026-10-21"));
        assert_eq!(Date::from_unix(midnight_21st - 1, OFFSET), d("2026-10-20"));

        let h = history([
            d("2026-10-19"),
            Date::from_unix(late_20th, OFFSET),
            Date::from_unix(midnight_21st, OFFSET),
        ]);
        let today = Date::from_unix(midnight_21st + 60, OFFSET);
        let s = solo(&h, &today.to_string());
        assert_eq!(s.current, 3);
        assert!(s.checked_in_today);

        // In UTC the same instants would have collapsed into the 21st and
        // left a hole on the 20th.
        let utc = history([
            d("2026-10-19"),
            Date::from_unix(late_20th, 0),
            Date::from_unix(midnight_21st, 0),
        ]);
        assert_eq!(solo(&utc, "2026-10-21").current, 1);
    }

    #[test]
    fn pause_validation() {
        let today = d("2026-10-20");
        let ok = pause("2026-10-21", "2026-10-25");
        assert_eq!(validate_pause(ok, today, &[]), Ok(()));
        assert_eq!(
            validate_pause(pause("2026-10-20", "2026-10-25"), today, &[]),
            Err(PauseError::NotAhead)
        );
        assert_eq!(
            validate_pause(pause("2026-10-23", "2026-10-22"), today, &[]),
            Err(PauseError::Backwards)
        );
        assert_eq!(
            validate_pause(pause("2026-10-21", "2026-11-19"), today, &[]),
            Ok(())
        );
        assert_eq!(
            validate_pause(pause("2026-10-21", "2026-11-20"), today, &[]),
            Err(PauseError::TooLong)
        );
        let existing = [pause("2026-10-25", "2026-10-27")];
        assert_eq!(
            validate_pause(pause("2026-10-27", "2026-10-28"), today, &existing),
            Err(PauseError::Overlaps)
        );
        assert_eq!(
            validate_pause(pause("2026-10-21", "2026-10-25"), today, &existing),
            Err(PauseError::Overlaps)
        );
        assert_eq!(
            validate_pause(pause("2026-10-28", "2026-10-30"), today, &existing),
            Ok(())
        );
    }

    #[test]
    fn event_keys() {
        assert_eq!(EventKind::FreezeEarned.key(), "freeze_earned");
        assert_eq!(EventKind::FreezeUsed.key(), "freeze_used");
    }
}
