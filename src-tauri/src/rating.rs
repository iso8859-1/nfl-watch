//! "How fun is this game to watch?" -- turns game facts into a 0-100 score.
//!
//! Each score is a sum of independent components ("criteria"). The user can
//! weight the criteria; see [`Model`] for how weighted scores are mapped back
//! onto the default 0-100 scale so that the star tiers keep their meaning.
//!
//! Reasons are reported separately so the UI can keep them hidden: they are
//! mild spoilers ("Comeback" tells you somebody was behind).

use crate::espn::{Game, GameFacts, Line};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::OnceLock;

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

// ---------------------------------------------------------------------------
// Criteria and weights

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Criterion {
    pub key: &'static str,
    pub label: &'static str,
    pub help: &'static str,
}

const fn crit(key: &'static str, label: &'static str, help: &'static str) -> Criterion {
    Criterion { key, label, help }
}

pub const WATCH_N: usize = 9;
pub const HYPE_N: usize = 5;

/// Watchability components, in the order of `watch_parts`.
pub const WATCH_CRITERIA: [Criterion; WATCH_N] = [
    crit("closeness", "Close final score", "One-score finishes score high, blowouts lose points."),
    crit("overtime", "Overtime", "Games that went to overtime."),
    crit("excitement", "Momentum swings", "How much the win probability moved during the game."),
    crit("leadChanges", "Lead changes", "Back-and-forth games with many lead changes and ties."),
    crit("comeback", "Comebacks", "The winner came back from a big deficit."),
    crit("lateDrama", "Late drama", "A go-ahead or tying score in the last five minutes or in overtime."),
    crit("upset", "Upsets", "The underdog won, judged by the betting line."),
    crit("scoring", "Scoring extremes", "Shootouts and low-scoring defensive battles."),
    crit("topMatchup", "Top matchup", "Two teams with winning records."),
];

/// Hype components, in the order of `hype_parts`.
pub const HYPE_CRITERIA: [Criterion; HYPE_N] = [
    crit("line", "Close betting line", "A small spread means the game is expected to be close."),
    crit("total", "Expected points", "A high over/under means a lot of scoring is expected."),
    crit("quality", "Team quality", "The teams' records so far."),
    crit("topMatchup", "Top matchup", "Two teams with winning records."),
    crit("division", "Division rivalry", "Both teams play in the same division."),
];

pub const MAX_WEIGHT: f64 = 3.0;

/// User weights by criterion key. Missing keys mean 1.0 (the default formula).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Weights {
    pub watch: BTreeMap<String, f64>,
    pub hype: BTreeMap<String, f64>,
}

fn weight_array<const N: usize>(criteria: &[Criterion; N], map: &BTreeMap<String, f64>) -> [f64; N] {
    criteria.map(|c| {
        let w = map.get(c.key).copied().unwrap_or(1.0);
        if w.is_finite() { w.clamp(0.0, MAX_WEIGHT) } else { 1.0 }
    })
}

/// Points per component, plus reasons tagged with the component they belong to.
pub struct Parts<const N: usize> {
    pub pts: [f64; N],
    reasons: Vec<(Option<usize>, String)>,
}

impl<const N: usize> Parts<N> {
    fn new() -> Self {
        Self { pts: [0.0; N], reasons: Vec::new() }
    }
    fn reason(&mut self, component: usize, text: impl Into<String>) {
        self.reasons.push((Some(component), text.into()));
    }
    /// Reasons of criteria the user switched off are dropped: they didn't count.
    fn reasons_for(&self, weights: &[f64; N]) -> Vec<String> {
        self.reasons
            .iter()
            .filter(|(c, _)| c.map_or(true, |c| weights[c] > 0.0))
            .map(|(_, r)| r.clone())
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Weighted score -> default scale

/// Maps a weighted sum of components back onto the scale of the default
/// formula, so the star cutoffs keep meaning the same share of games.
///
/// Simply multiplying components by weights would shift the whole score
/// distribution: doubling the rare "Upset" bonus barely moves the average,
/// while halving "Close final score" pulls nearly every game down a tier.
/// Normalizing the weights to sum 1 doesn't help either, because the
/// components already have very different ranges.
///
/// Instead, the weighted score is quantile-matched against a reference
/// season (anonymous component vectors of every 2025 game, embedded in the
/// app): a game whose weighted score beats 85% of the reference games gets
/// the default score that beats 85% of them. With all weights at 1 this is
/// exactly the default formula. Only the ratios between weights matter.
pub struct Model<const N: usize> {
    weights: [f64; N],
    /// (weighted score, default score), both sorted ascending, ties merged.
    knots: Vec<(f64, f64)>,
}

fn dot<const N: usize>(w: &[f64; N], x: &[f64; N]) -> f64 {
    w.iter().zip(x).map(|(a, b)| a * b).sum()
}

impl<const N: usize> Model<N> {
    pub fn new(sample: &[[f64; N]], weights: [f64; N]) -> Self {
        let identity = weights.iter().all(|&w| w == 1.0);
        if identity || sample.is_empty() {
            return Self { weights, knots: Vec::new() };
        }
        let mut xs: Vec<f64> = sample.iter().map(|c| dot(&weights, c)).collect();
        let mut ys: Vec<f64> = sample.iter().map(|c| c.iter().sum()).collect();
        xs.sort_by(f64::total_cmp);
        ys.sort_by(f64::total_cmp);
        let mut knots: Vec<(f64, f64)> = Vec::new();
        let mut i = 0;
        while i < xs.len() {
            let mut j = i;
            while j + 1 < xs.len() && xs[j + 1] - xs[i] < 1e-9 {
                j += 1;
            }
            let y = ys[i..=j].iter().sum::<f64>() / (j - i + 1) as f64;
            knots.push((xs[i], y));
            i = j + 1;
        }
        Self { weights, knots }
    }

    pub fn weights(&self) -> &[f64; N] {
        &self.weights
    }

    /// Unrounded score on the default scale.
    pub fn score(&self, pts: &[f64; N]) -> f64 {
        let k = &self.knots;
        if k.is_empty() {
            return pts.iter().sum();
        }
        let x = dot(&self.weights, pts);
        let (first, last) = (k[0], k[k.len() - 1]);
        if k.len() == 1 {
            return first.1;
        }
        if x <= first.0 || x >= last.0 {
            // Beyond the reference season: extend linearly.
            let slope = (last.1 - first.1) / (last.0 - first.0);
            let anchor = if x <= first.0 { first } else { last };
            return anchor.1 + (x - anchor.0) * slope;
        }
        let hi = k.partition_point(|p| p.0 < x);
        let (a, b) = (k[hi - 1], k[hi]);
        a.1 + (x - a.0) / (b.0 - a.0) * (b.1 - a.1)
    }
}

fn parse_sample<const N: usize>(text: &str) -> Vec<[f64; N]> {
    text.lines()
        .skip(1) // header
        .filter_map(|line| {
            let v: Vec<f64> = line.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            v.try_into().ok()
        })
        .collect()
}

/// Reference samples: component points of every 2025 game (watchability) and of
/// regular-season games from week 5 on (hype). Rows are sorted, so they can't be
/// matched to games. Regenerate with `cargo test export_samples -- --ignored`.
fn watch_sample() -> &'static [[f64; WATCH_N]] {
    static S: OnceLock<Vec<[f64; WATCH_N]>> = OnceLock::new();
    S.get_or_init(|| parse_sample(include_str!("../data/watch-sample.csv")))
}

fn hype_sample() -> &'static [[f64; HYPE_N]] {
    static S: OnceLock<Vec<[f64; HYPE_N]>> = OnceLock::new();
    S.get_or_init(|| parse_sample(include_str!("../data/hype-sample.csv")))
}

#[derive(Serialize)]
pub struct CriterionInfo {
    #[serde(flatten)]
    criterion: Criterion,
    /// Typical spread (standard deviation) of this component's points in the
    /// reference season. The UI uses it to show each criterion's influence.
    spread: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CriteriaInfo {
    pub watch: Vec<CriterionInfo>,
    pub hype: Vec<CriterionInfo>,
    pub max_weight: f64,
}

fn infos<const N: usize>(criteria: &[Criterion; N], sample: &[[f64; N]]) -> Vec<CriterionInfo> {
    let n = sample.len().max(1) as f64;
    (0..N)
        .map(|i| {
            let mean = sample.iter().map(|c| c[i]).sum::<f64>() / n;
            let var = sample.iter().map(|c| (c[i] - mean).powi(2)).sum::<f64>() / n;
            CriterionInfo { criterion: criteria[i], spread: if sample.is_empty() { 1.0 } else { var.sqrt() } }
        })
        .collect()
}

pub fn criteria_info() -> CriteriaInfo {
    CriteriaInfo {
        watch: infos(&WATCH_CRITERIA, watch_sample()),
        hype: infos(&HYPE_CRITERIA, hype_sample()),
        max_weight: MAX_WEIGHT,
    }
}

/// Rates games with a given set of weights. Build once per request.
pub struct Scorer {
    watch: Model<WATCH_N>,
    hype: Model<HYPE_N>,
}

impl Default for Scorer {
    fn default() -> Self {
        Self::new(&Weights::default())
    }
}

impl Scorer {
    pub fn new(w: &Weights) -> Self {
        Self {
            watch: Model::new(watch_sample(), weight_array(&WATCH_CRITERIA, &w.watch)),
            hype: Model::new(hype_sample(), weight_array(&HYPE_CRITERIA, &w.hype)),
        }
    }

    pub fn rate(&self, game: &Game, facts: Option<&GameFacts>, home_rec: Record, away_rec: Record) -> Rating {
        let parts = watch_parts(game, facts, home_rec, away_rec);
        let score = self.watch.score(&parts.pts).round().clamp(0.0, 100.0) as u8;
        // Thresholds calibrated on the 2025 season: roughly the top ~15% of games
        // are "Must watch", ~30% are lopsided enough to skip.
        let stars = match score {
            75.. => 5,
            58..=74 => 4,
            40..=57 => 3,
            15..=39 => 2,
            _ => 1,
        };
        let mut reasons = parts.reasons_for(self.watch.weights());
        if reasons.is_empty() {
            reasons.push("Not much drama".into());
        }
        Rating { score, stars, tier: tier_for(stars).into(), reasons }
    }

    pub fn hype(
        &self,
        home_abbr: &str,
        away_abbr: &str,
        line: Option<&Line>,
        home_rec: Record,
        away_rec: Record,
        international: bool,
    ) -> Rating {
        let parts = hype_parts(home_abbr, away_abbr, line, home_rec, away_rec, international);
        let score = self.hype.score(&parts.pts).round().clamp(0.0, 100.0) as u8;
        // Thresholds calibrated on the 2025 regular season, weeks 5+ (~10% "Can't miss").
        let stars = match score {
            50.. => 5,
            39..=49 => 4,
            31..=38 => 3,
            24..=30 => 2,
            _ => 1,
        };
        let reasons = parts.reasons_for(self.hype.weights());
        Rating { score, stars, tier: hype_tier(stars).into(), reasons }
    }
}

// ---------------------------------------------------------------------------
// Watchability components

pub fn watch_parts(game: &Game, facts: Option<&GameFacts>, home_rec: Record, away_rec: Record) -> Parts<WATCH_N> {
    const CLOSE: usize = 0;
    const OT: usize = 1;
    const EXCITE: usize = 2;
    const LEADS: usize = 3;
    const COMEBACK: usize = 4;
    const LATE: usize = 5;
    const UPSET: usize = 6;
    const SCORING: usize = 7;
    const TOP: usize = 8;

    let hs = game.home.score;
    let aw = game.away.score;
    let margin = (hs - aw).abs();
    let total = hs + aw;
    let mut p = Parts::new();

    // --- closeness of the final result
    p.pts[CLOSE] = match margin {
        0 => 22.0,
        1..=3 => 25.0,
        4..=7 => 18.0,
        8..=10 => 10.0,
        11..=14 => 4.0,
        15..=20 => 0.0,
        _ => -10.0, // blowout penalty
    };
    if margin <= 3 {
        p.reason(CLOSE, "Nail-biter to the end");
    } else if margin <= 8 {
        p.reason(CLOSE, "One-score game");
    }

    let plays = facts.map(|f| f.plays.as_slice()).unwrap_or(&[]);

    // --- overtime
    let overtime = facts.map_or(false, |f| f.periods > 4) || plays.iter().any(|p| p.period > 4);
    if overtime {
        p.pts[OT] = 10.0;
        p.reason(OT, "Overtime");
    }

    // --- win-probability swings (sum of absolute changes, a classic excitement index)
    let wp = facts.map(|f| f.wp.as_slice()).unwrap_or(&[]);
    if wp.len() > 10 {
        let excitement: f64 = wp.windows(2).map(|w| (w[1] - w[0]).abs()).sum();
        p.pts[EXCITE] = ((excitement - 1.5) * 5.0).clamp(0.0, 25.0);
        if excitement >= 5.0 {
            p.reason(EXCITE, "Wild momentum swings");
        }
    } else {
        p.pts[EXCITE] = if margin <= 8 { 12.0 } else { 4.0 };
    }

    // --- lead changes, ties, comebacks, late drama from the scoring timeline
    let winner_home = hs > aw;
    let mut lead_changes = 0;
    let mut ties = 0;
    let mut last_leader = 0i32; // sign of (home - away) of the last team that led
    let mut worst_winner_deficit = 0;
    let mut late_drama = 0.0f64;
    let (mut prev_h, mut prev_a) = (0, 0);
    for s in plays {
        let diff = s.home - s.away;
        let sign = diff.signum();
        if sign != 0 && last_leader != 0 && sign != last_leader {
            lead_changes += 1;
        }
        if sign == 0 && s.home + s.away > 0 && prev_h != prev_a {
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
        let home_scored = s.home > prev_h;
        let scorer_before = if home_scored { prev_diff } else { -prev_diff };
        let scorer_after = if home_scored { diff } else { -diff };
        let late = (s.period == 4 && s.clock <= 300.0) || s.period > 4;
        if late && scorer_before <= 0 && scorer_after >= 0 {
            let value = if s.period > 4 || s.clock <= 120.0 { 12.0 } else { 9.0 };
            late_drama = late_drama.max(value);
        }
        prev_h = s.home;
        prev_a = s.away;
    }
    p.pts[LEADS] = (lead_changes as f64 * 3.0 + ties as f64).min(15.0);
    if lead_changes >= 3 {
        p.reason(LEADS, format!("Back-and-forth ({lead_changes} lead changes)"));
    }
    let deficit = -worst_winner_deficit;
    p.pts[COMEBACK] = match deficit {
        d if d >= 17 => 14.0,
        d if d >= 14 => 12.0,
        d if d >= 10 => 8.0,
        d if d >= 7 => 5.0,
        _ => 0.0,
    };
    if deficit >= 10 {
        p.reason(COMEBACK, "Big comeback");
    } else if deficit >= 7 {
        p.reason(COMEBACK, "Comeback");
    }
    if late_drama > 0.0 {
        p.pts[LATE] = late_drama;
        p.reason(LATE, "Late go-ahead or tying score");
    }

    // --- upset relative to the betting line
    if let Some(line) = facts.and_then(|f| f.line.as_ref()) {
        if let Some(fav) = &line.favorite {
            let winner_id = if winner_home { &game.home.team.id } else { &game.away.team.id };
            if margin > 0 && winner_id != fav && line.spread >= 3.0 {
                p.pts[UPSET] = if line.spread >= 7.0 { 10.0 } else { 6.0 };
                p.reason(UPSET, "Underdog upset");
            }
        }
    }

    // --- scoring extremes
    if total >= 65 {
        p.pts[SCORING] = 12.0;
        p.reason(SCORING, "Shootout");
    } else if total >= 55 {
        p.pts[SCORING] = 8.0;
        p.reason(SCORING, "High-scoring");
    } else if total >= 48 && margin <= 10 {
        p.pts[SCORING] = 4.0;
    } else if total <= 20 {
        p.pts[SCORING] = if margin <= 7 { 10.0 } else { 5.0 };
        p.reason(SCORING, "Defensive battle");
    }

    // --- quality of the matchup (pre-game knowledge, not a spoiler)
    if top_matchup(home_rec, away_rec) {
        p.pts[TOP] = 6.0;
    }
    p
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

pub fn hype_parts(
    home_abbr: &str,
    away_abbr: &str,
    line: Option<&Line>,
    home_rec: Record,
    away_rec: Record,
    international: bool,
) -> Parts<HYPE_N> {
    const LINE: usize = 0;
    const TOTAL: usize = 1;
    const QUALITY: usize = 2;
    const TOP: usize = 3;
    const DIVISION: usize = 4;
    let mut p = Parts::new();

    // Expected closeness from the betting line.
    match line.map(|l| l.spread) {
        Some(s) => {
            p.pts[LINE] = match s {
                s if s <= 1.5 => 30.0,
                s if s <= 3.5 => 25.0,
                s if s <= 5.5 => 17.0,
                s if s <= 7.5 => 10.0,
                s if s <= 10.0 => 4.0,
                _ => 0.0,
            };
            if s <= 1.5 {
                p.reason(LINE, "Coin-flip line");
            } else if s <= 3.5 {
                p.reason(LINE, format!("Tight line ({})", fmt_num(s)));
            } else if s >= 10.0 {
                p.reason(LINE, format!("Big favorite ({})", fmt_num(s)));
            }
        }
        None => p.pts[LINE] = 12.0,
    }

    // Expected points.
    if let Some(ou) = line.and_then(|l| l.over_under) {
        p.pts[TOTAL] = match ou {
            x if x >= 52.0 => 15.0,
            x if x >= 48.0 => 11.0,
            x if x >= 45.0 => 7.0,
            x if x >= 42.0 => 3.0,
            _ => 0.0,
        };
        if ou >= 50.0 {
            p.reason(TOTAL, format!("Shootout potential (O/U {})", fmt_num(ou)));
        } else if ou <= 38.0 {
            p.reason(TOTAL, format!("Defensive slugfest (O/U {})", fmt_num(ou)));
        }
    }

    // Team quality.
    if home_rec.games() >= 2 && away_rec.games() >= 2 {
        p.pts[QUALITY] = (home_rec.pct() + away_rec.pct()) / 2.0 * 30.0;
        if home_rec.pct() < 0.35 && away_rec.pct() < 0.35 && home_rec.games() >= 3 {
            p.pts[QUALITY] -= 8.0;
            p.reason(QUALITY, "Two struggling teams");
        }
    } else {
        p.pts[QUALITY] = 15.0;
    }
    if top_matchup(home_rec, away_rec) {
        p.pts[TOP] = 10.0;
        p.reason(TOP, "Top matchup");
    }
    if division_game(home_abbr, away_abbr) {
        p.pts[DIVISION] = 6.0;
        p.reason(DIVISION, "Division rivals");
    }
    if international {
        p.reasons.push((None, "International game".into()));
    }
    p
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

    fn thriller() -> GameFacts {
        GameFacts {
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
        }
    }

    #[test]
    fn thriller_beats_blowout() {
        let blowout = GameFacts {
            plays: vec![play(7, 0, 1, 600.0), play(14, 0, 2, 600.0), play(31, 3, 3, 100.0)],
            wp: (0..60).map(|i| 0.6 + i as f64 * 0.006).collect(),
            line: None,
            periods: 4,
        };
        let s = Scorer::default();
        let r1 = s.rate(&game(20, 17), Some(&thriller()), Record::default(), Record::default());
        let r2 = s.rate(&game(38, 3), Some(&blowout), Record::default(), Record::default());
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
        let r = Scorer::default().rate(&game(0, 13), Some(&facts), Record::default(), Record::default());
        assert!(r.reasons.iter().any(|x| x.contains("upset")), "{r:?}");
        assert!(r.reasons.iter().any(|x| x.contains("Defensive")), "{r:?}");
    }

    #[test]
    fn hype_prefers_close_good_matchups() {
        let good = Record { w: 7, l: 2, t: 0 };
        let bad = Record { w: 2, l: 7, t: 0 };
        let close = Line { favorite: None, spread: 1.5, over_under: Some(51.5) };
        let lopsided = Line { favorite: None, spread: 13.5, over_under: Some(39.0) };
        let s = Scorer::default();
        let a = s.hype("KC", "DEN", Some(&close), good, good, false);
        let b = s.hype("MIA", "SF", Some(&lopsided), bad, good, false);
        assert!(a.stars >= 4 && a.reasons.iter().any(|r| r.contains("Division")), "{a:?}");
        assert!(b.stars <= 2, "{b:?}");
    }

    #[test]
    fn default_weights_match_the_plain_sum() {
        let parts = watch_parts(&game(20, 17), Some(&thriller()), Record::default(), Record::default());
        let m = Model::new(watch_sample(), [1.0; WATCH_N]);
        assert_eq!(m.score(&parts.pts), parts.pts.iter().sum::<f64>());
    }

    #[test]
    fn only_weight_ratios_matter() {
        let parts = watch_parts(&game(20, 17), Some(&thriller()), Record::default(), Record::default());
        let mut w = [1.0; WATCH_N];
        w[1] = 2.0;
        w[6] = 0.0;
        let a = Model::new(watch_sample(), w).score(&parts.pts);
        let b = Model::new(watch_sample(), w.map(|x| x * 1.5)).score(&parts.pts);
        assert!((a - b).abs() < 1e-6, "{a} vs {b}");
    }

    #[test]
    fn weighting_keeps_the_star_shares() {
        let sample = watch_sample();
        if sample.is_empty() {
            return;
        }
        let stars = |w: [f64; WATCH_N]| {
            let m = Model::new(sample, w);
            let mut n = [0usize; 5];
            for c in sample {
                let s = m.score(c).round();
                n[match s as i32 { 75.. => 4, 58..=74 => 3, 40..=57 => 2, 15..=39 => 1, _ => 0 }] += 1;
            }
            n
        };
        let base = stars([1.0; WATCH_N]);
        let mut w = [1.0; WATCH_N];
        w[0] = 0.3; // care much less about the final margin
        w[2] = 2.5; // and much more about momentum swings
        let skewed = stars(w);
        for (a, b) in base.iter().zip(&skewed) {
            assert!(a.abs_diff(*b) <= sample.len() / 20, "{base:?} vs {skewed:?}");
        }
    }

    #[test]
    fn switched_off_criteria_give_no_reasons() {
        let mut w = Weights::default();
        w.watch.insert("lateDrama".into(), 0.0);
        let r = Scorer::new(&w).rate(&game(20, 17), Some(&thriller()), Record::default(), Record::default());
        assert!(!r.reasons.iter().any(|x| x.contains("Late")), "{r:?}");
    }
}
