//! "How fun is this game to watch?" -- turns game facts into a 0-100 score.
//!
//! The score combines several independent signals of entertainment value.
//! Reasons are reported separately so the UI can keep them hidden: they are
//! mild spoilers ("Comeback" tells you somebody was behind).

use crate::espn::{Game, GameFacts, Line};
use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Record {
    pub w: u32,
    pub l: u32,
    pub t: u32,
}

impl Record {
    pub fn games(&self) -> u32 {
        self.w + self.l + self.t
    }
    pub fn pct(&self) -> f64 {
        if self.games() == 0 {
            0.5
        } else {
            (self.w as f64 + 0.5 * self.t as f64) / self.games() as f64
        }
    }
    pub fn text(&self) -> String {
        if self.t > 0 {
            format!("{}-{}-{}", self.w, self.l, self.t)
        } else {
            format!("{}-{}", self.w, self.l)
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Rating {
    pub score: u8,
    /// 1..=5
    pub stars: u8,
    pub tier: String,
    /// Spoiler-ish explanations, shown only on demand.
    pub reasons: Vec<String>,
}

pub fn tier_for(stars: u8) -> &'static str {
    match stars {
        5 => "Must watch",
        4 => "Worth it",
        3 => "Solid",
        2 => "Only if bored",
        _ => "Skip it",
    }
}

/// Is this a matchup of two good teams (judged on pre-game records only)?
pub fn top_matchup(home: Record, away: Record) -> bool {
    let (h, a) = (home.pct(), away.pct());
    home.games() >= 3 && away.games() >= 3 && h.min(a) >= 0.6 && (h + a) / 2.0 >= 0.65
}

pub fn rate(game: &Game, facts: Option<&GameFacts>, home_rec: Record, away_rec: Record) -> Rating {
    let hs = game.home.score;
    let aw = game.away.score;
    let margin = (hs - aw).abs();
    let total = hs + aw;
    let mut pts: f64 = 0.0;
    let mut reasons: Vec<String> = Vec::new();

    // --- closeness of the final result
    pts += match margin {
        0 => 22.0,
        1..=3 => 25.0,
        4..=7 => 18.0,
        8..=10 => 10.0,
        11..=14 => 4.0,
        _ => 0.0,
    };
    if margin <= 3 {
        reasons.push("Nail-biter to the end".into());
    } else if margin <= 8 {
        reasons.push("One-score game".into());
    }
    if margin >= 21 {
        pts -= 10.0;
    }

    let plays = facts.map(|f| f.plays.as_slice()).unwrap_or(&[]);

    // --- overtime
    let overtime = facts.map_or(false, |f| f.periods > 4) || plays.iter().any(|p| p.period > 4);
    if overtime {
        pts += 10.0;
        reasons.push("Overtime".into());
    }

    // --- win-probability swings (sum of absolute changes, a classic excitement index)
    let wp = facts.map(|f| f.wp.as_slice()).unwrap_or(&[]);
    if wp.len() > 10 {
        let excitement: f64 = wp.windows(2).map(|w| (w[1] - w[0]).abs()).sum();
        pts += ((excitement - 1.5) * 5.0).clamp(0.0, 25.0);
        if excitement >= 5.0 {
            reasons.push("Wild momentum swings".into());
        }
    } else {
        pts += if margin <= 8 { 12.0 } else { 4.0 };
    }

    // --- lead changes, ties, comebacks, late drama from the scoring timeline
    let winner_home = hs > aw;
    let mut lead_changes = 0;
    let mut ties = 0;
    let mut last_leader = 0i32; // sign of (home - away) of the last team that led
    let mut worst_winner_deficit = 0;
    let mut late_drama = 0.0f64;
    let (mut prev_h, mut prev_a) = (0, 0);
    for p in plays {
        let diff = p.home - p.away;
        let sign = diff.signum();
        if sign != 0 && last_leader != 0 && sign != last_leader {
            lead_changes += 1;
        }
        if sign == 0 && p.home + p.away > 0 && prev_h != prev_a {
            ties += 1;
        }
        if sign != 0 {
            last_leader = sign;
        }
        if margin > 0 {
            let winner_diff = if winner_home { diff } else { -diff };
            worst_winner_deficit = worst_winner_deficit.min(winner_diff);
        }
        // Go-ahead or game-tying score late in the 4th quarter or in overtime.
        let prev_diff = prev_h - prev_a;
        let home_scored = p.home > prev_h;
        let scorer_before = if home_scored { prev_diff } else { -prev_diff };
        let scorer_after = if home_scored { diff } else { -diff };
        let late = (p.period == 4 && p.clock <= 300.0) || p.period > 4;
        if late && scorer_before <= 0 && scorer_after >= 0 {
            let value = if p.period > 4 || p.clock <= 120.0 { 12.0 } else { 9.0 };
            late_drama = late_drama.max(value);
        }
        prev_h = p.home;
        prev_a = p.away;
    }
    pts += (lead_changes as f64 * 3.0 + ties as f64).min(15.0);
    if lead_changes >= 3 {
        reasons.push(format!("Back-and-forth ({lead_changes} lead changes)"));
    }
    let deficit = -worst_winner_deficit;
    pts += match deficit {
        d if d >= 17 => 14.0,
        d if d >= 14 => 12.0,
        d if d >= 10 => 8.0,
        d if d >= 7 => 5.0,
        _ => 0.0,
    };
    if deficit >= 10 {
        reasons.push("Big comeback".into());
    } else if deficit >= 7 {
        reasons.push("Comeback".into());
    }
    if late_drama > 0.0 {
        pts += late_drama;
        reasons.push("Late go-ahead or tying score".into());
    }

    // --- upset relative to the betting line
    if let Some(line) = facts.and_then(|f| f.line.as_ref()) {
        if let Some(fav) = &line.favorite {
            let winner_id = if winner_home { &game.home.team.id } else { &game.away.team.id };
            if margin > 0 && winner_id != fav && line.spread >= 3.0 {
                pts += if line.spread >= 7.0 { 10.0 } else { 6.0 };
                reasons.push("Underdog upset".into());
            }
        }
    }

    // --- scoring extremes
    if total >= 65 {
        pts += 12.0;
        reasons.push("Shootout".into());
    } else if total >= 55 {
        pts += 8.0;
        reasons.push("High-scoring".into());
    } else if total >= 48 && margin <= 10 {
        pts += 4.0;
    } else if total <= 20 {
        pts += if margin <= 7 { 10.0 } else { 5.0 };
        reasons.push("Defensive battle".into());
    }

    // --- quality of the matchup (pre-game knowledge, not a spoiler)
    if top_matchup(home_rec, away_rec) {
        pts += 6.0;
    }

    let score = pts.round().clamp(0.0, 100.0) as u8;
    // Thresholds calibrated on the 2025 season: roughly the top ~15% of games
    // are "Must watch", ~30% are lopsided enough to skip.
    let stars = match score {
        75.. => 5,
        58..=74 => 4,
        40..=57 => 3,
        15..=39 => 2,
        _ => 1,
    };
    if reasons.is_empty() {
        reasons.push("Not much drama".into());
    }
    Rating { score, stars, tier: tier_for(stars).into(), reasons }
}

// ---------------------------------------------------------------------------
// Pre-game hype: how promising an *upcoming* game looks. Uses only information
// available before kickoff, so it can never spoil anything.

const DIVISIONS: [[&str; 4]; 8] = [
    ["BUF", "MIA", "NE", "NYJ"],
    ["BAL", "CIN", "CLE", "PIT"],
    ["HOU", "IND", "JAX", "TEN"],
    ["DEN", "KC", "LV", "LAC"],
    ["DAL", "NYG", "PHI", "WSH"],
    ["CHI", "DET", "GB", "MIN"],
    ["ATL", "CAR", "NO", "TB"],
    ["ARI", "LAR", "SF", "SEA"],
];

pub fn division_game(a: &str, b: &str) -> bool {
    DIVISIONS.iter().any(|d| d.contains(&a) && d.contains(&b))
}

pub fn hype_tier(stars: u8) -> &'static str {
    match stars {
        5 => "Can't miss",
        4 => "Promising",
        3 => "Decent",
        2 => "Meh",
        _ => "Low hype",
    }
}

fn fmt_num(x: f64) -> String {
    if x.fract() == 0.0 { format!("{x:.0}") } else { format!("{x:.1}") }
}

pub fn hype(
    home_abbr: &str,
    away_abbr: &str,
    line: Option<&Line>,
    home_rec: Record,
    away_rec: Record,
    international: bool,
) -> Rating {
    let mut pts = 0.0;
    let mut reasons = Vec::new();

    // Expected closeness from the betting line.
    match line.map(|l| l.spread) {
        Some(s) => {
            pts += match s {
                s if s <= 1.5 => 30.0,
                s if s <= 3.5 => 25.0,
                s if s <= 5.5 => 17.0,
                s if s <= 7.5 => 10.0,
                s if s <= 10.0 => 4.0,
                _ => 0.0,
            };
            if s <= 1.5 {
                reasons.push("Coin-flip line".to_string());
            } else if s <= 3.5 {
                reasons.push(format!("Tight line ({})", fmt_num(s)));
            } else if s >= 10.0 {
                reasons.push(format!("Big favorite ({})", fmt_num(s)));
            }
        }
        None => pts += 12.0,
    }

    // Expected points.
    if let Some(ou) = line.and_then(|l| l.over_under) {
        pts += match ou {
            x if x >= 52.0 => 15.0,
            x if x >= 48.0 => 11.0,
            x if x >= 45.0 => 7.0,
            x if x >= 42.0 => 3.0,
            _ => 0.0,
        };
        if ou >= 50.0 {
            reasons.push(format!("Shootout potential (O/U {})", fmt_num(ou)));
        } else if ou <= 38.0 {
            reasons.push(format!("Defensive slugfest (O/U {})", fmt_num(ou)));
        }
    }

    // Team quality.
    if home_rec.games() >= 2 && away_rec.games() >= 2 {
        pts += (home_rec.pct() + away_rec.pct()) / 2.0 * 30.0;
        if home_rec.pct() < 0.35 && away_rec.pct() < 0.35 && home_rec.games() >= 3 {
            pts -= 8.0;
            reasons.push("Two struggling teams".into());
        }
    } else {
        pts += 15.0;
    }
    if top_matchup(home_rec, away_rec) {
        pts += 10.0;
        reasons.push("Top matchup".into());
    }
    if division_game(home_abbr, away_abbr) {
        pts += 6.0;
        reasons.push("Division rivals".into());
    }
    if international {
        reasons.push("International game".into());
    }

    let score = (pts as f64).round().clamp(0.0, 100.0) as u8;
    // Thresholds calibrated on the 2025 regular season, weeks 5+ (~10% "Can't miss").
    let stars = match score {
        50.. => 5,
        39..=49 => 4,
        31..=38 => 3,
        24..=30 => 2,
        _ => 1,
    };
    Rating { score, stars, tier: hype_tier(stars).into(), reasons }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::espn::{ScoringPlay, Side, Team};

    fn team(id: &str) -> Team {
        Team {
            id: id.into(),
            abbr: id.into(),
            name: id.into(),
            short: id.into(),
            logo: String::new(),
            color: String::new(),
        }
    }

    fn game(home: i32, away: i32) -> Game {
        Game {
            id: "1".into(),
            date: String::new(),
            state: "post".into(),
            completed: true,
            venue: String::new(),
            broadcast: String::new(),
            note: String::new(),
            home: Side { team: team("H"), score: home },
            away: Side { team: team("A"), score: away },
            neutral: false,
            line: None,
        }
    }

    fn play(home: i32, away: i32, period: u32, clock: f64) -> ScoringPlay {
        ScoringPlay { home, away, period, clock }
    }

    #[test]
    fn thriller_beats_blowout() {
        let thriller = GameFacts {
            plays: vec![
                play(0, 7, 1, 600.0),
                play(7, 7, 2, 300.0),
                play(7, 14, 3, 500.0),
                play(14, 14, 4, 600.0),
                play(14, 17, 4, 200.0),
                play(20, 17, 4, 30.0),
            ],
            wp: (0..60).map(|i| if i % 2 == 0 { 0.3 } else { 0.7 }).collect(),
            line: None,
            periods: 4,
        };
        let blowout = GameFacts {
            plays: vec![play(7, 0, 1, 600.0), play(14, 0, 2, 600.0), play(31, 3, 3, 100.0)],
            wp: (0..60).map(|i| 0.6 + i as f64 * 0.006).collect(),
            line: None,
            periods: 4,
        };
        let r1 = rate(&game(20, 17), Some(&thriller), Record::default(), Record::default());
        let r2 = rate(&game(38, 3), Some(&blowout), Record::default(), Record::default());
        assert!(r1.stars >= 4, "{r1:?}");
        assert_eq!(r2.stars, 1, "{r2:?}");
        assert!(r1.reasons.iter().any(|r| r.contains("Late")));
    }

    #[test]
    fn upset_is_detected() {
        let facts = GameFacts {
            plays: vec![play(0, 10, 2, 0.0), play(0, 13, 3, 0.0)],
            wp: vec![],
            line: Some(crate::espn::Line { favorite: Some("H".into()), spread: 9.5, over_under: None }),
            periods: 4,
        };
        let r = rate(&game(0, 13), Some(&facts), Record::default(), Record::default());
        assert!(r.reasons.iter().any(|x| x.contains("upset")), "{r:?}");
        assert!(r.reasons.iter().any(|x| x.contains("Defensive")), "{r:?}");
    }

    #[test]
    fn hype_prefers_close_good_matchups() {
        let good = Record { w: 7, l: 2, t: 0 };
        let bad = Record { w: 2, l: 7, t: 0 };
        let close = Line { favorite: None, spread: 1.5, over_under: Some(51.5) };
        let lopsided = Line { favorite: None, spread: 13.5, over_under: Some(39.0) };
        let a = hype("KC", "DEN", Some(&close), good, good, false);
        let b = hype("MIA", "SF", Some(&lopsided), bad, good, false);
        assert!(a.stars >= 4 && a.reasons.iter().any(|r| r.contains("Division")), "{a:?}");
        assert!(b.stars <= 2, "{b:?}");
    }
}
