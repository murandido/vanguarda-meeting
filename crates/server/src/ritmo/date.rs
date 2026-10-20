//! Datas civis (sem fuso) e o cálculo das semanas do Ritmo.
//!
//! A semana termina no fim do dia da reunião (padrão: terça) e a próxima
//! começa no dia seguinte. Tudo aqui é puro: o "agora" vem de fora.

use std::fmt;

/// Dia civil, guardado como dias desde 1970-01-01.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date(i64);

impl Date {
    /// Data a partir de ano, mês e dia; `None` se não existir.
    #[must_use]
    pub fn from_ymd(y: i64, m: i64, d: i64) -> Option<Self> {
        if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
            return None;
        }
        let date = Self(days_from_civil(y, m, d));
        (date.ymd() == (y, m, d)).then_some(date)
    }

    /// Lê `YYYY-MM-DD` (o formato guardado no banco e usado nas URLs).
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        let b = s.as_bytes();
        if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
            return None;
        }
        let num = |r: std::ops::Range<usize>| -> Option<i64> {
            let part = &s[r];
            part.bytes()
                .all(|c| c.is_ascii_digit())
                .then(|| part.parse().ok())?
        };
        Self::from_ymd(num(0..4)?, num(5..7)?, num(8..10)?)
    }

    /// Dia local de um instante Unix, dado o fuso em minutos (ex.: -180).
    #[must_use]
    pub fn from_unix(secs: i64, offset_minutes: i64) -> Self {
        Self((secs + offset_minutes * 60).div_euclid(86_400))
    }

    #[must_use]
    pub fn ymd(self) -> (i64, i64, i64) {
        civil_from_days(self.0)
    }

    #[must_use]
    pub fn add_days(self, n: i64) -> Self {
        Self(self.0 + n)
    }

    #[must_use]
    pub fn weekday(self) -> Weekday {
        // 1970-01-01 foi uma quinta-feira.
        Weekday::ALL[usize::try_from((self.0 + 3).rem_euclid(7)).expect("0..7")]
    }

    /// `dd/mm`, para a interface.
    #[must_use]
    pub fn short(self) -> String {
        let (_, m, d) = self.ymd();
        format!("{d:02}/{m:02}")
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (y, m, d) = self.ymd();
        write!(f, "{y:04}-{m:02}-{d:02}")
    }
}

/// Hora local `HH:MM` de um instante Unix.
#[must_use]
pub fn local_time(secs: i64, offset_minutes: i64) -> String {
    let mins = (secs + offset_minutes * 60).rem_euclid(86_400) / 60;
    format!("{:02}:{:02}", mins / 60, mins % 60)
}

// Algoritmos de Howard Hinnant ("chrono-compatible low-level date algorithms").
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weekday {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

impl Weekday {
    pub const ALL: [Self; 7] = [
        Self::Monday,
        Self::Tuesday,
        Self::Wednesday,
        Self::Thursday,
        Self::Friday,
        Self::Saturday,
        Self::Sunday,
    ];

    /// Valor guardado em `ritmo_settings` (em inglês).
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Monday => "monday",
            Self::Tuesday => "tuesday",
            Self::Wednesday => "wednesday",
            Self::Thursday => "thursday",
            Self::Friday => "friday",
            Self::Saturday => "saturday",
            Self::Sunday => "sunday",
        }
    }

    #[must_use]
    pub fn from_key(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|d| d.key() == s)
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Monday => "segunda-feira",
            Self::Tuesday => "terça-feira",
            Self::Wednesday => "quarta-feira",
            Self::Thursday => "quinta-feira",
            Self::Friday => "sexta-feira",
            Self::Saturday => "sábado",
            Self::Sunday => "domingo",
        }
    }

    fn index(self) -> i64 {
        match self {
            Self::Monday => 0,
            Self::Tuesday => 1,
            Self::Wednesday => 2,
            Self::Thursday => 3,
            Self::Friday => 4,
            Self::Saturday => 5,
            Self::Sunday => 6,
        }
    }
}

/// Semana "natural" que contém `day`: termina no próximo dia da reunião
/// (o próprio `day`, se for o dia da reunião) e tem sete dias.
#[must_use]
pub fn canonical_week(day: Date, meeting: Weekday) -> (Date, Date) {
    let ahead = (meeting.index() - day.weekday().index()).rem_euclid(7);
    let end = day.add_days(ahead);
    (end.add_days(-6), end)
}

/// Semana a criar para `day`, ajustada para não encostar nas vizinhas que já
/// existem (`prev_end`: fim da última semana antes de `day`; `next_start`:
/// início da primeira depois). Só acontece quando o dia da reunião muda.
#[must_use]
pub fn plan_week(
    day: Date,
    meeting: Weekday,
    prev_end: Option<Date>,
    next_start: Option<Date>,
) -> (Date, Date) {
    let (mut start, mut end) = canonical_week(day, meeting);
    if let Some(prev) = prev_end
        && prev >= start
    {
        start = prev.add_days(1);
    }
    if let Some(next) = next_start
        && next <= end
    {
        end = next.add_days(-1);
    }
    (start, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Date {
        Date::parse(s).unwrap()
    }

    #[test]
    fn parses_and_formats() {
        assert_eq!(d("2026-10-20").to_string(), "2026-10-20");
        assert_eq!(d("1970-01-01"), Date(0));
        assert_eq!(d("2024-02-29").ymd(), (2024, 2, 29));
        assert_eq!(d("1969-12-31"), Date(-1));
        assert_eq!(d("2000-03-01").add_days(-1), d("2000-02-29"));
        assert_eq!(d("2026-01-05").short(), "05/01");
        for bad in [
            "2026-02-29",
            "2026-13-01",
            "2026-00-10",
            "2026-04-31",
            "2026-1-01",
            "20261020",
            "2026-10-2x",
            "+026-10-20",
            "",
        ] {
            assert_eq!(Date::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn round_trips_many_days() {
        for n in -800_000..800_000 {
            let (y, m, dd) = Date(n).ymd();
            assert_eq!(Date::from_ymd(y, m, dd), Some(Date(n)));
        }
    }

    #[test]
    fn weekdays() {
        assert_eq!(d("1970-01-01").weekday(), Weekday::Thursday);
        assert_eq!(d("2026-10-20").weekday(), Weekday::Tuesday);
        assert_eq!(d("2026-10-25").weekday(), Weekday::Sunday);
        assert_eq!(d("1969-12-29").weekday(), Weekday::Monday);
    }

    #[test]
    fn weekday_keys_and_labels() {
        for day in Weekday::ALL {
            assert_eq!(Weekday::from_key(day.key()), Some(day));
            assert_ne!(day.label(), "");
        }
        assert_eq!(Weekday::from_key("terca"), None);
        assert_eq!(Weekday::Tuesday.label(), "terça-feira");
    }

    #[test]
    fn local_day_and_time_follow_offset() {
        // 2026-10-21 02:30 UTC = 2026-10-20 23:30 em Brasília (-03:00).
        let secs = 1_792_549_800;
        assert_eq!(Date::from_unix(secs, 0), d("2026-10-21"));
        assert_eq!(Date::from_unix(secs, -180), d("2026-10-20"));
        assert_eq!(local_time(secs, 0), "02:30");
        assert_eq!(local_time(secs, -180), "23:30");
        assert_eq!(Date::from_unix(-1, 0), d("1969-12-31"));
    }

    #[test]
    fn week_ends_at_end_of_meeting_day() {
        let tue = Weekday::Tuesday;
        // Terça (dia da reunião) ainda é da semana que termina nela.
        assert_eq!(
            canonical_week(d("2026-10-20"), tue),
            (d("2026-10-14"), d("2026-10-20"))
        );
        // Quarta já é a semana nova.
        assert_eq!(
            canonical_week(d("2026-10-21"), tue),
            (d("2026-10-21"), d("2026-10-27"))
        );
        // Segunda: último dia antes da reunião.
        assert_eq!(
            canonical_week(d("2026-10-26"), tue),
            (d("2026-10-21"), d("2026-10-27"))
        );
        // Outro dia de reunião: domingo.
        assert_eq!(
            canonical_week(d("2026-10-21"), Weekday::Sunday),
            (d("2026-10-19"), d("2026-10-25"))
        );
        // Virada de ano.
        assert_eq!(
            canonical_week(d("2026-12-31"), tue),
            (d("2026-12-30"), d("2027-01-05"))
        );
    }

    #[test]
    fn plan_week_avoids_overlap_after_meeting_day_change() {
        // Semana existente terminou numa terça; reunião passou para quinta.
        let (s, e) = plan_week(
            d("2026-10-21"),
            Weekday::Thursday,
            Some(d("2026-10-20")),
            None,
        );
        assert_eq!((s, e), (d("2026-10-21"), d("2026-10-22")));

        // Semana seguinte já existe e começa antes do fim natural.
        let (s, e) = plan_week(
            d("2026-10-21"),
            Weekday::Tuesday,
            None,
            Some(d("2026-10-25")),
        );
        assert_eq!((s, e), (d("2026-10-21"), d("2026-10-24")));

        // Vizinhas distantes não mudam nada.
        assert_eq!(
            plan_week(
                d("2026-10-22"),
                Weekday::Tuesday,
                Some(d("2026-10-06")),
                Some(d("2026-11-11"))
            ),
            canonical_week(d("2026-10-22"), Weekday::Tuesday)
        );
    }
}
