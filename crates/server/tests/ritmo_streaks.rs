//! Ritmo streaks (KB-06) and celebrations (KB-08), end to end through the
//! HTTP routes, with the server clock fixed in Brasília time (UTC-3).

mod common;

use axum::http::StatusCode;
use common::{
    body_text,
    ritmo::{DAY, Dupla, HOUR, TUE_NOON, WED_NOON, dupla},
};

/// The streak number shown for `login` on the board (`dupla` for the pair).
fn streak_of(html: &str, login: &str) -> u32 {
    let marker = if login == "dupla" {
        r#"<p class="streak dupla""#.to_owned()
    } else {
        format!(r#"<p class="streak" data-login="{login}""#)
    };
    let start = html
        .find(&marker)
        .unwrap_or_else(|| panic!("no streak for {login}"));
    let rest = &html[start..];
    let from = rest.find(r#"<strong class="dias">"#).unwrap() + r#"<strong class="dias">"#.len();
    rest[from..].split('<').next().unwrap().parse().unwrap()
}

/// The `data-status` of `login`'s streak (`feito`, `pausa` or `pendente`).
fn streak_status(html: &str, login: &str) -> String {
    let marker = format!(r#"<p class="streak" data-login="{login}" data-status=""#);
    let start = html.find(&marker).unwrap() + marker.len();
    html[start..].split('"').next().unwrap().to_owned()
}

impl Dupla {
    /// Check-in with no tasks: it still counts for the streak.
    async fn checkin(&self, who: &str) {
        let res = self.post(who, "/ritmo/checkin", "").await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers()["hx-redirect"], "/ritmo#celebrar");
    }

    /// Moves the clock to noon, `n` days after Wednesday 21/10.
    fn day(&self, n: i64) {
        self.clock.set(WED_NOON + n * DAY);
    }
}

#[tokio::test]
async fn checkin_starts_and_grows_the_streak() {
    let d = dupla(WED_NOON).await;
    let body = d.page(&d.ana, "/ritmo").await;
    assert_eq!(streak_of(&body, "ana"), 0);
    assert_eq!(streak_status(&body, "ana"), "pendente");
    assert!(body.contains("comece hoje com um check-in"));

    // The check-in page works even without tasks.
    let body = d.page(&d.ana, "/ritmo/checkin").await;
    assert!(body.contains("Salvar check-in"));
    d.checkin(&d.ana).await;
    let body = d.page(&d.ana, "/ritmo").await;
    assert_eq!(streak_of(&body, "ana"), 1);
    assert_eq!(streak_status(&body, "ana"), "feito");
    assert!(body.contains("1</strong> dia\n"));

    // Next day: still 1 until the check-in, then 2.
    d.day(1);
    let body = d.page(&d.ana, "/ritmo").await;
    assert_eq!(streak_of(&body, "ana"), 1);
    assert!(body.contains("falta o check-in de hoje"));
    d.checkin(&d.ana).await;
    assert_eq!(streak_of(&d.page(&d.ana, "/ritmo").await, "ana"), 2);

    // Skipping a whole day with no freeze breaks it.
    d.day(3);
    assert_eq!(streak_of(&d.page(&d.ana, "/ritmo").await, "ana"), 0);
}

#[tokio::test]
async fn updating_todays_checkin_does_not_count_twice() {
    let d = dupla(WED_NOON).await;
    d.checkin(&d.ana).await;
    d.clock.set(WED_NOON + 2 * HOUR);
    d.checkin(&d.ana).await;
    assert_eq!(streak_of(&d.page(&d.ana, "/ritmo").await, "ana"), 1);
}

#[tokio::test]
async fn day_boundary_follows_brasilia_time() {
    // Tuesday 23:59 in Brasília (Wednesday 02:59 UTC), then Wednesday 00:01
    // (03:01 UTC): two different local days, so a streak of 2.
    let d = dupla(TUE_NOON + 11 * HOUR + 59 * 60).await;
    d.checkin(&d.ana).await;
    d.clock.set(TUE_NOON + 12 * HOUR + 60);
    let body = d.page(&d.ana, "/ritmo").await;
    // A new day: yesterday's check-in still holds the streak.
    assert_eq!(streak_of(&body, "ana"), 1);
    assert_eq!(streak_status(&body, "ana"), "pendente");
    d.checkin(&d.ana).await;
    assert_eq!(streak_of(&d.page(&d.ana, "/ritmo").await, "ana"), 2);
    let days: Vec<String> = sqlx::query_scalar("SELECT day FROM ritmo_checkins ORDER BY day")
        .fetch_all(&d.pool)
        .await
        .unwrap();
    assert_eq!(days, ["2026-10-20", "2026-10-21"]);
}

#[tokio::test]
async fn freezes_are_earned_used_and_recorded() {
    let d = dupla(WED_NOON).await;
    for n in 0..7 {
        d.day(n);
        d.checkin(&d.ana).await;
    }
    let body = d.page(&d.ana, "/ritmo").await;
    assert_eq!(streak_of(&body, "ana"), 7);
    assert!(body.contains("🧊 1/2"));
    assert_eq!(
        d.count("SELECT COUNT(*) FROM ritmo_streak_events WHERE kind = 'freeze_earned'")
            .await,
        1
    );

    // Miss day 7, come back on day 8: the freeze covered the gap.
    d.day(8);
    d.checkin(&d.ana).await;
    let body = d.page(&d.ana, "/ritmo").await;
    assert_eq!(streak_of(&body, "ana"), 8);
    assert!(body.contains("🧊 0/2"));
    let used: Vec<(String, String)> =
        sqlx::query_as("SELECT login, day FROM ritmo_streak_events WHERE kind = 'freeze_used'")
            .fetch_all(&d.pool)
            .await
            .unwrap();
    assert_eq!(used, [("ana".to_owned(), "2026-10-28".to_owned())]);

    // Events are recorded once, however many times the board is seen.
    d.page(&d.bia, "/ritmo").await;
    d.page(&d.ana, "/").await;
    assert_eq!(d.count("SELECT COUNT(*) FROM ritmo_streak_events").await, 2);
}

#[tokio::test]
async fn pair_streak_counts_days_where_both_checked_in() {
    let d = dupla(WED_NOON).await;
    d.checkin(&d.ana).await;
    d.checkin(&d.bia).await;
    d.day(1);
    d.checkin(&d.ana).await;
    let body = d.page(&d.bia, "/ritmo").await;
    assert_eq!(streak_of(&body, "dupla"), 1);
    assert!(body.contains("conta quando os dois fazem check-in"));
    d.checkin(&d.bia).await;
    let body = d.page(&d.bia, "/ritmo").await;
    assert_eq!(streak_of(&body, "dupla"), 2);
    assert!(body.contains("os dois fizeram check-in hoje"));

    // Ana alone on day 2; Bia misses it, which breaks hers and the pair's.
    d.day(2);
    d.checkin(&d.ana).await;
    d.day(3);
    let body = d.page(&d.ana, "/ritmo").await;
    assert_eq!(streak_of(&body, "ana"), 3);
    assert_eq!(streak_of(&body, "bia"), 0);
    assert_eq!(streak_of(&body, "dupla"), 0);
}

#[tokio::test]
async fn home_card_shows_the_streaks() {
    let d = dupla(WED_NOON).await;
    d.checkin(&d.ana).await;
    d.checkin(&d.bia).await;
    let body = d.page(&d.ana, "/").await;
    assert!(body.contains(r#"<span class="streak" data-login="você">"#));
    assert!(body.contains(r#"<span class="streak" data-login="bia">"#));
    assert!(body.contains("<strong>1</strong> dupla"));
}

#[tokio::test]
async fn pauses_neither_break_nor_extend() {
    let d = dupla(WED_NOON).await;
    d.checkin(&d.ana).await;
    // Pause Thursday to Saturday, set ahead of time.
    let res = d
        .post(
            &d.ana,
            "/ritmo/pausas",
            "inicio=2026-10-22&fim=2026-10-24&motivo=Viagem",
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()["hx-refresh"], "true");
    let body = d.page(&d.bia, "/ritmo").await;
    assert!(body.contains("ana: 22/10 – 24/10 — Viagem"));
    // Only the owner gets the cancel button.
    assert!(!body.contains("/ritmo/pausas/1/cancelar"));

    d.day(2);
    let body = d.page(&d.ana, "/ritmo").await;
    assert_eq!(streak_status(&body, "ana"), "pausa");
    assert!(body.contains("em pausa até 24/10"));
    // A check-in during the pause is fine, but does not count.
    d.checkin(&d.ana).await;
    assert_eq!(streak_of(&d.page(&d.ana, "/ritmo").await, "ana"), 1);

    d.day(4);
    d.checkin(&d.ana).await;
    let body = d.page(&d.ana, "/ritmo").await;
    assert_eq!(streak_of(&body, "ana"), 2);
    assert_eq!(d.count("SELECT COUNT(*) FROM ritmo_streak_events").await, 0);
}

#[tokio::test]
async fn pauses_are_validated() {
    let d = dupla(WED_NOON).await;
    d.post(&d.ana, "/ritmo/pausas", "inicio=2026-10-25&fim=2026-10-27")
        .await;
    let cases = [
        ("inicio=2026-10-21&fim=2026-10-22", "a partir de amanhã"),
        ("inicio=2026-10-23&fim=2026-10-22", "vem antes do início"),
        ("inicio=2026-10-22&fim=2026-11-30", "no máximo 30 dias"),
        ("inicio=2026-10-27&fim=2026-10-28", "já tem uma pausa"),
        ("inicio=amanha&fim=2026-10-22", "Data de início inválida"),
        ("inicio=2026-10-22&fim=", "Data de fim inválida"),
    ];
    for (form, message) in cases {
        let res = d.post(&d.ana, "/ritmo/pausas", form).await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST, "{form}");
        assert!(body_text(res).await.contains(message), "{form}");
    }
    let long = format!("inicio=2026-11-01&fim=2026-11-02&motivo={}", "x".repeat(61));
    let res = d.post(&d.ana, "/ritmo/pausas", &long).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    // Bia's pauses don't overlap with Ana's.
    let res = d
        .post(&d.bia, "/ritmo/pausas", "inicio=2026-10-25&fim=2026-10-27")
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(d.count("SELECT COUNT(*) FROM ritmo_pauses").await, 2);
}

#[tokio::test]
async fn pauses_can_be_cancelled_by_the_owner_before_they_start() {
    let d = dupla(WED_NOON).await;
    d.post(&d.ana, "/ritmo/pausas", "inicio=2026-10-23&fim=2026-10-24")
        .await;
    let id: i64 = d.count("SELECT id FROM ritmo_pauses").await;
    let cancel = format!("/ritmo/pausas/{id}/cancelar");
    assert!(d.page(&d.ana, "/ritmo").await.contains(&cancel));

    let res = d.post(&d.bia, &cancel, "").await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let res = d.post(&d.ana, "/ritmo/pausas/999/cancelar", "").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // Once it starts, it stays: past days must keep their meaning.
    d.day(2);
    assert!(!d.page(&d.ana, "/ritmo").await.contains(&cancel));
    let res = d.post(&d.ana, &cancel, "").await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert_eq!(d.count("SELECT COUNT(*) FROM ritmo_pauses").await, 1);

    // A future one can go.
    d.post(&d.ana, "/ritmo/pausas", "inicio=2026-11-01&fim=2026-11-01")
        .await;
    let id: i64 = d.count("SELECT MAX(id) FROM ritmo_pauses").await;
    let res = d
        .post(&d.ana, &format!("/ritmo/pausas/{id}/cancelar"), "")
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(d.count("SELECT COUNT(*) FROM ritmo_pauses").await, 1);

    // Pauses that ended are no longer listed.
    d.day(5);
    assert!(!d.page(&d.ana, "/ritmo").await.contains("23/10 – 24/10"));
}

#[tokio::test]
async fn moving_to_done_asks_for_a_celebration() {
    let d = dupla(WED_NOON).await;
    let p = d.project(&d.ana, "Livro").await;
    let id = d.task(&d.ana, "2026-10-21", p, "Escrever").await;
    let mover = format!("/ritmo/tarefas/{id}/mover");

    let res = d.post(&d.ana, &mover, "para=doing").await;
    assert_eq!(res.headers()["hx-trigger"], "ritmo-changed");
    let res = d.post(&d.ana, &mover, "para=done").await;
    assert_eq!(
        res.headers()["hx-trigger"],
        "ritmo-changed, ritmo-celebrate"
    );
    // Already done: nothing new to celebrate.
    let res = d.post(&d.ana, &mover, "para=done").await;
    assert_eq!(res.headers()["hx-trigger"], "ritmo-changed");

    // The page has the live region for the message and the sound toggle.
    let body = d.page(&d.ana, "/ritmo").await;
    assert!(body.contains(r#"id="comemoracao""#));
    assert!(body.contains(r#"id="som""#));
}

#[test]
fn dev_clock_lands_on_the_requested_day() {
    use server::{Clock, clock_on, ritmo::date::Date};

    let now = Clock::default().now();
    for day in ["2026-10-20", "2031-01-01", "2001-09-30"] {
        let clock = clock_on(day, -180, now).unwrap();
        assert_eq!(Date::from_unix(clock.now(), -180).to_string(), day);
    }
    assert!(clock_on("terça", -180, now).is_none());
}

#[tokio::test]
async fn pages_have_no_inline_styles_or_handlers() {
    // The CSP forbids them; project colors go through `data-cor`.
    let d = dupla(WED_NOON).await;
    let p = d.project(&d.ana, "Livro").await;
    d.task(&d.ana, "2026-10-21", p, "Escrever").await;
    for path in ["/ritmo", "/ritmo/checkin", "/ritmo/semana/fechar"] {
        let body = d.page(&d.ana, path).await;
        assert!(!body.contains("style="), "{path}");
        assert!(!body.contains("hx-on"), "{path}");
        assert!(!body.contains("<script>"), "{path}");
    }
}
