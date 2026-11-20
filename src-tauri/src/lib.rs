//! NFL Watch: spoiler-free suggestions of which NFL games are worth watching.
//!
//! SPOILER POLICY: the commands below are the only thing the UI can talk to.
//! None of the view models carries a score, a post-game record or any other
//! result information. The single exception is `reveal_result`, which only
//! answers for games the user has explicitly marked as watched.

mod espn;
mod rating;
mod store;

use espn::{Espn, Game, Line, Team, WeekInfo, POST, REGULAR};
use rating::{CriteriaInfo, Rating, Record, Scorer};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;
use store::{Settings, Store, UserData};
use tauri::{Manager, State};

struct App {
    espn: Espn,
    store: Mutex<Store>,
}

type Res<T> = Result<T, String>;

// ---------------------------------------------------------------------- view models

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Overview {
    current_season: i32,
    season: i32,
    seasons: Vec<i32>,
    weeks: Vec<WeekInfo>,
    default_index: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TeamCard {
    id: String,
    abbr: String,
    name: String,
    short: String,
    logo: String,
    color: String,
    /// Record *before* this game, computed from earlier weeks.
    record: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GameCard {
    id: String,
    kickoff: String,
    /// "final", "live" or "upcoming"
    status: String,
    venue: String,
    broadcast: String,
    note: String,
    home: TeamCard,
    away: TeamCard,
    favorite: bool,
    watched: bool,
    top_matchup: bool,
    /// Played abroad (London, Munich, ...): often at European-friendly times.
    international: bool,
    /// Pre-game betting line, e.g. "KC -3.5 · O/U 47.5".
    line: Option<String>,
    rating: Option<Rating>,
    /// Pre-game hype for games that have not finished yet.
    hype: Option<Rating>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ResultView {
    home_abbr: String,
    away_abbr: String,
    home_score: i32,
    away_score: i32,
}

fn team_card(team: &Team, rec: Option<Record>) -> TeamCard {
    TeamCard {
        id: team.id.clone(),
        abbr: team.abbr.clone(),
        name: team.name.clone(),
        short: team.short.clone(),
        logo: team.logo.clone(),
        color: team.color.clone(),
        record: rec.filter(|r| r.games() > 0).map(|r| r.text()),
    }
}

fn line_text(game: &Game, line: Option<&Line>) -> Option<String> {
    let line = line?;
    let fav = line.favorite.as_ref().and_then(|id| {
        [&game.home.team, &game.away.team]
            .into_iter()
            .find(|t| &t.id == id)
            .map(|t| t.abbr.clone())
    });
    let mut parts = Vec::new();
    match fav {
        Some(abbr) if line.spread > 0.0 => parts.push(format!("{abbr} -{}", line.spread)),
        _ => parts.push("Pick'em".into()),
    }
    if let Some(ou) = line.over_under {
        parts.push(format!("O/U {ou}"));
    }
    Some(parts.join(" · "))
}

// ---------------------------------------------------------------------- helpers

/// Team records from all completed games strictly before the given week.
async fn records_before(app: &App, season: i32, stype: u8, week: u8) -> HashMap<String, Record> {
    let weeks = app.espn.season_weeks(season).await.unwrap_or_default();
    let keys: Vec<(u8, u8)> = weeks
        .iter()
        .filter(|w| w.season_type == REGULAR && (stype == POST || w.week < week))
        .map(|w| (w.season_type, w.week))
        .collect();
    let mut records: HashMap<String, Record> = HashMap::new();
    for games in app.espn.many_weeks(season, &keys).await {
        for g in games.iter().filter(|g| g.completed) {
            let (h, a) = (g.home.score, g.away.score);
            let home = records.entry(g.home.team.id.clone()).or_default();
            match h.cmp(&a) {
                std::cmp::Ordering::Greater => home.w += 1,
                std::cmp::Ordering::Less => home.l += 1,
                std::cmp::Ordering::Equal => home.t += 1,
            }
            let away = records.entry(g.away.team.id.clone()).or_default();
            match a.cmp(&h) {
                std::cmp::Ordering::Greater => away.w += 1,
                std::cmp::Ordering::Less => away.l += 1,
                std::cmp::Ordering::Equal => away.t += 1,
            }
        }
    }
    records
}

fn user_data(app: &App) -> UserData {
    app.store.lock().unwrap().data.clone()
}

fn update_store(app: &App, f: impl FnOnce(&mut UserData)) -> Res<UserData> {
    let mut store = app.store.lock().unwrap();
    f(&mut store.data);
    store.save()?;
    Ok(store.data.clone())
}

// ---------------------------------------------------------------------- commands

#[tauri::command]
async fn get_overview(app: State<'_, App>, season: Option<i32>) -> Res<Overview> {
    let current = app.espn.current_season().await?;
    let season = season.unwrap_or(current);
    let weeks = app.espn.season_weeks(season).await?;
    // Default to the latest week in which at least one game has finished.
    let mut default_index = weeks.len().saturating_sub(1);
    for (i, w) in weeks.iter().enumerate().rev() {
        let games = app.espn.week_games(season, w.season_type, w.week).await?;
        if games.iter().any(|g| g.completed) {
            default_index = i;
            break;
        }
    }
    Ok(Overview {
        current_season: current,
        season,
        seasons: (0..5).map(|i| current - i).collect(),
        weeks,
        default_index,
    })
}

#[tauri::command]
async fn get_week(app: State<'_, App>, season: i32, season_type: u8, week: u8) -> Res<Vec<GameCard>> {
    let games = app.espn.week_games(season, season_type, week).await?;
    let finished: Vec<String> = games.iter().filter(|g| g.completed).map(|g| g.id.clone()).collect();
    let (facts, records) = futures::join!(
        app.espn.many_facts(&finished),
        records_before(&app, season, season_type, week)
    );
    let data = user_data(&app);
    let scorer = Scorer::new(&data.settings.weights);

    let cards = games
        .iter()
        .map(|g| {
            let hr = records.get(&g.home.team.id).copied().unwrap_or_default();
            let ar = records.get(&g.away.team.id).copied().unwrap_or_default();
            let f = facts.get(&g.id);
            let line = f.and_then(|f| f.line.as_ref()).or(g.line.as_ref());
            let international = g.neutral && season_type == REGULAR;
            let status = if g.completed {
                "final"
            } else if g.state == "in" {
                "live"
            } else {
                "upcoming"
            };
            GameCard {
                id: g.id.clone(),
                kickoff: g.date.clone(),
                status: status.into(),
                venue: g.venue.clone(),
                broadcast: g.broadcast.clone(),
                note: g.note.clone(),
                home: team_card(&g.home.team, Some(hr)),
                away: team_card(&g.away.team, Some(ar)),
                favorite: data.favorites.contains(&g.home.team.id)
                    || data.favorites.contains(&g.away.team.id),
                watched: data.watched.contains(&g.id),
                top_matchup: rating::top_matchup(hr, ar),
                international,
                line: line_text(g, line),
                rating: g.completed.then(|| scorer.rate(g, f, hr, ar)),
                hype: (!g.completed).then(|| {
                    scorer.hype(&g.home.team.abbr, &g.away.team.abbr, line, hr, ar, international)
                }),
            }
        })
        .collect();
    Ok(cards)
}

#[tauri::command]
async fn get_teams(app: State<'_, App>) -> Res<Vec<Team>> {
    app.espn.teams().await
}

/// The rating criteria the user can weight, for the settings screen.
#[tauri::command]
fn get_criteria() -> CriteriaInfo {
    rating::criteria_info()
}

#[tauri::command]
fn get_user_data(app: State<'_, App>) -> UserData {
    user_data(&app)
}

#[tauri::command]
fn set_favorites(app: State<'_, App>, team_ids: Vec<String>) -> Res<UserData> {
    update_store(&app, |d| d.favorites = team_ids.into_iter().collect())
}

#[tauri::command]
fn set_watched(app: State<'_, App>, game_id: String, watched: bool) -> Res<UserData> {
    update_store(&app, |d| {
        if watched {
            d.watched.insert(game_id);
        } else {
            d.watched.remove(&game_id);
        }
    })
}

#[tauri::command]
fn set_settings(app: State<'_, App>, settings: Settings) -> Res<UserData> {
    update_store(&app, |d| d.settings = settings)
}

/// The final score -- only for games the user has marked as watched.
#[tauri::command]
fn reveal_result(app: State<'_, App>, game_id: String) -> Res<ResultView> {
    if !user_data(&app).watched.contains(&game_id) {
        return Err("Mark the game as watched first.".into());
    }
    let g = app.espn.seen_game(&game_id).ok_or("Unknown game")?;
    if !g.completed {
        return Err("Game has not finished yet.".into());
    }
    Ok(ResultView {
        home_abbr: g.home.team.abbr,
        away_abbr: g.away.team.abbr,
        home_score: g.home.score,
        away_score: g.away.score,
    })
}

#[tauri::command]
fn refresh(app: State<'_, App>) {
    app.espn.clear_live();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            app.manage(App {
                espn: Espn::new(dir.join("cache")),
                store: Mutex::new(Store::load(dir.join("user-data.json"))),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_overview,
            get_week,
            get_teams,
            get_criteria,
            get_user_data,
            set_favorites,
            set_watched,
            set_settings,
            reveal_result,
            refresh
        ])
        .run(tauri::generate_context!())
        .expect("error while running NFL Watch");
}

#[cfg(test)]
mod calibration {
    use super::*;

    /// Prints only the distribution of ratings of a full season (no teams, no scores).
    /// Run with: cargo test calibration -- --ignored --nocapture
    #[test]
    #[ignore]
    fn season_distribution() {
        let season: i32 = std::env::var("SEASON").ok().and_then(|s| s.parse().ok()).unwrap_or(2025);
        let dir = std::env::temp_dir().join("nfl-watch-calibration");
        let espn = Espn::new(dir);
        tauri::async_runtime::block_on(async {
            let weeks = espn.season_weeks(season).await.unwrap();
            let mut stars = [0usize; 6];
            let mut scores = Vec::new();
            let mut reasons: HashMap<String, usize> = HashMap::new();
            for w in &weeks {
                let games = espn.week_games(season, w.season_type, w.week).await.unwrap();
                let ids: Vec<String> = games.iter().filter(|g| g.completed).map(|g| g.id.clone()).collect();
                let facts = espn.many_facts(&ids).await;
                let mut per_week = [0usize; 6];
                for g in games.iter().filter(|g| g.completed) {
                    let r = Scorer::default().rate(g, facts.get(&g.id), Record::default(), Record::default());
                    stars[r.stars as usize] += 1;
                    per_week[r.stars as usize] += 1;
                    scores.push(r.score);
                    for x in r.reasons {
                        let key = x.split(" (").next().unwrap().to_string();
                        *reasons.entry(key).or_default() += 1;
                    }
                }
                println!("{:>5}: 1*={} 2*={} 3*={} 4*={} 5*={}", w.label, per_week[1], per_week[2], per_week[3], per_week[4], per_week[5]);
            }
            scores.sort();
            let n = scores.len();
            println!("games={n} stars 1..5 = {:?}", &stars[1..]);
            println!("score percentiles p10={} p25={} p50={} p75={} p90={}", scores[n / 10], scores[n / 4], scores[n / 2], scores[3 * n / 4], scores[9 * n / 10]);
            let mut r: Vec<_> = reasons.into_iter().collect();
            r.sort_by(|a, b| b.1.cmp(&a.1));
            println!("reasons: {r:?}");
        });
    }

    /// Does pre-game hype predict post-game watchability? Aggregates only.
    /// Run with: cargo test hype_vs_rating -- --ignored --nocapture
    #[test]
    #[ignore]
    fn hype_vs_rating() {
        let season: i32 = std::env::var("SEASON").ok().and_then(|s| s.parse().ok()).unwrap_or(2025);
        let espn = Espn::new(std::env::temp_dir().join("nfl-watch-calibration"));
        let scorer = Scorer::default();
        tauri::async_runtime::block_on(async {
            let weeks = espn.season_weeks(season).await.unwrap();
            let mut records: HashMap<String, Record> = HashMap::new();
            let mut pairs: Vec<(f64, f64)> = Vec::new();
            let mut by_star = [(0usize, 0f64); 6];
            for w in weeks.iter().filter(|w| w.season_type == REGULAR) {
                let games = espn.week_games(season, w.season_type, w.week).await.unwrap();
                let ids: Vec<String> = games.iter().filter(|g| g.completed).map(|g| g.id.clone()).collect();
                let facts = espn.many_facts(&ids).await;
                for g in games.iter().filter(|g| g.completed) {
                    let hr = records.get(&g.home.team.id).copied().unwrap_or_default();
                    let ar = records.get(&g.away.team.id).copied().unwrap_or_default();
                    let f = facts.get(&g.id);
                    let h = scorer.hype(&g.home.team.abbr, &g.away.team.abbr, f.and_then(|f| f.line.as_ref()), hr, ar, g.neutral);
                    let r = scorer.rate(g, f, hr, ar);
                    if w.week < 5 { continue; } // records mean little early on
                    pairs.push((h.score as f64, r.score as f64));
                    by_star[h.stars as usize].0 += 1;
                    by_star[h.stars as usize].1 += r.score as f64;
                }
                for g in games.iter().filter(|g| g.completed) {
                    let (h, a) = (g.home.score, g.away.score);
                    let e = records.entry(g.home.team.id.clone()).or_default();
                    if h > a { e.w += 1 } else if h < a { e.l += 1 } else { e.t += 1 }
                    let e = records.entry(g.away.team.id.clone()).or_default();
                    if a > h { e.w += 1 } else if a < h { e.l += 1 } else { e.t += 1 }
                }
            }
            let n = pairs.len() as f64;
            let (mx, my) = (pairs.iter().map(|p| p.0).sum::<f64>() / n, pairs.iter().map(|p| p.1).sum::<f64>() / n);
            let cov: f64 = pairs.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
            let vx: f64 = pairs.iter().map(|p| (p.0 - mx).powi(2)).sum();
            let vy: f64 = pairs.iter().map(|p| (p.1 - my).powi(2)).sum();
            let mut hs: Vec<f64> = pairs.iter().map(|p| p.0).collect(); hs.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let q = |f: f64| hs[((hs.len() as f64 - 1.0) * f) as usize];
            println!("hype percentiles p20={} p45={} p70={} p88={}", q(0.2), q(0.45), q(0.7), q(0.88));
            println!("games={n} correlation(hype, watchability)={:.3}", cov / (vx * vy).sqrt());
            for s in 1..=5 {
                let (c, sum) = by_star[s];
                if c > 0 { println!("hype {s}*: n={c:>3} avg watchability={:.1}", sum / c as f64); }
            }
        });
    }

    /// Writes the anonymous reference samples used to calibrate weighted scores
    /// (`data/*-sample.csv`): component points only, rows sorted, no game ids.
    /// Run with: SEASON=2025 cargo test export_samples -- --ignored
    #[test]
    #[ignore]
    fn export_samples() {
        let season: i32 = std::env::var("SEASON").ok().and_then(|s| s.parse().ok()).unwrap_or(2025);
        let espn = Espn::new(std::env::temp_dir().join("nfl-watch-calibration"));
        tauri::async_runtime::block_on(async {
            let weeks = espn.season_weeks(season).await.unwrap();
            let mut records: HashMap<String, Record> = HashMap::new();
            let mut watch: Vec<Vec<f64>> = Vec::new();
            let mut hype: Vec<Vec<f64>> = Vec::new();
            // Regular season first, so playoff games see the full regular-season records.
            let mut ordered: Vec<_> = weeks.iter().filter(|w| w.season_type == REGULAR).collect();
            ordered.extend(weeks.iter().filter(|w| w.season_type == POST));
            for w in ordered {
                let games = espn.week_games(season, w.season_type, w.week).await.unwrap();
                let ids: Vec<String> = games.iter().filter(|g| g.completed).map(|g| g.id.clone()).collect();
                let facts = espn.many_facts(&ids).await;
                for g in games.iter().filter(|g| g.completed) {
                    let hr = records.get(&g.home.team.id).copied().unwrap_or_default();
                    let ar = records.get(&g.away.team.id).copied().unwrap_or_default();
                    let f = facts.get(&g.id);
                    watch.push(rating::watch_parts(g, f, hr, ar).pts.to_vec());
                    if w.season_type == REGULAR && w.week >= 5 {
                        let line = f.and_then(|f| f.line.as_ref());
                        let neutral = g.neutral;
                        hype.push(rating::hype_parts(&g.home.team.abbr, &g.away.team.abbr, line, hr, ar, neutral).pts.to_vec());
                    }
                }
                if w.season_type != REGULAR {
                    continue;
                }
                for g in games.iter().filter(|g| g.completed) {
                    let (h, a) = (g.home.score, g.away.score);
                    let e = records.entry(g.home.team.id.clone()).or_default();
                    if h > a { e.w += 1 } else if h < a { e.l += 1 } else { e.t += 1 }
                    let e = records.entry(g.away.team.id.clone()).or_default();
                    if a > h { e.w += 1 } else if a < h { e.l += 1 } else { e.t += 1 }
                }
            }
            let write = |name: &str, keys: Vec<&str>, mut rows: Vec<Vec<f64>>| {
                rows.sort_by(|a, b| a.iter().zip(b).map(|(x, y)| x.total_cmp(y)).find(|o| o.is_ne()).unwrap_or(std::cmp::Ordering::Equal));
                let mut text = keys.join(",") + "\n";
                for r in &rows {
                    let cells: Vec<String> = r.iter().map(|x| format!("{}", (x * 100.0).round() / 100.0)).collect();
                    text += &(cells.join(",") + "\n");
                }
                let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data").join(name);
                std::fs::write(&path, text).unwrap();
                println!("{name}: {} rows", rows.len());
            };
            write("watch-sample.csv", rating::WATCH_CRITERIA.iter().map(|c| c.key).collect(), watch);
            write("hype-sample.csv", rating::HYPE_CRITERIA.iter().map(|c| c.key).collect(), hype);
        });
    }
}
