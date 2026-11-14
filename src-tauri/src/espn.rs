//! Fetching and caching NFL data from ESPN's public site API.
//!
//! Raw ESPN payloads contain spoilers (scores, records, headlines). Only the
//! facts needed for rating are extracted here, and those types are never sent
//! to the UI directly -- see `main.rs` for the spoiler-free view models.

use chrono::{DateTime, NaiveDateTime, Utc};
use futures::future::join_all;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const BASE: &str = "https://site.api.espn.com/apis/site/v2/sports/football/nfl";
/// How long data of weeks with unfinished games is reused before refetching.
const LIVE_TTL: Duration = Duration::from_secs(300);

pub const REGULAR: u8 = 2;
pub const POST: u8 = 3;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Team {
    pub id: String,
    pub abbr: String,
    pub name: String,
    pub short: String,
    pub logo: String,
    pub color: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Side {
    pub team: Team,
    pub score: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Game {
    pub id: String,
    pub date: String,
    /// "pre", "in" or "post"
    pub state: String,
    pub completed: bool,
    pub venue: String,
    pub broadcast: String,
    pub note: String,
    pub home: Side,
    pub away: Side,
    /// Neutral site (international games, Super Bowl).
    #[serde(default)]
    pub neutral: bool,
    /// Pre-game betting line (available for upcoming games).
    #[serde(default)]
    pub line: Option<Line>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WeekInfo {
    pub season_type: u8,
    pub week: u8,
    pub label: String,
    pub long_label: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScoringPlay {
    pub home: i32,
    pub away: i32,
    pub period: u32,
    /// Seconds remaining in the period.
    pub clock: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Line {
    /// Team id of the pre-game favorite.
    pub favorite: Option<String>,
    pub spread: f64,
    pub over_under: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GameFacts {
    pub plays: Vec<ScoringPlay>,
    /// Home win probability after each play.
    pub wp: Vec<f64>,
    pub line: Option<Line>,
    pub periods: u32,
}

type Timed<T> = Mutex<HashMap<String, (Instant, T)>>;

pub struct Espn {
    http: reqwest::Client,
    dir: PathBuf,
    weeks: Timed<Vec<Game>>,
    calendars: Timed<Vec<WeekInfo>>,
    current: Timed<i32>,
    /// Every game seen so far, by id (used to reveal results of watched games).
    seen: Mutex<HashMap<String, Game>>,
}

fn str_of(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn num_of(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

pub fn parse_date(s: &str) -> Option<DateTime<Utc>> {
    if let Ok(d) = DateTime::parse_from_rfc3339(s) {
        return Some(d.with_timezone(&Utc));
    }
    NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%MZ")
        .ok()
        .map(|n| n.and_utc())
}

impl Espn {
    pub fn new(dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(dir.join("weeks"));
        let _ = std::fs::create_dir_all(dir.join("games"));
        let http = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (compatible; nfl-watch/0.1)")
            .timeout(Duration::from_secs(20))
            .build()
            .expect("http client");
        Self {
            http,
            dir,
            weeks: Mutex::default(),
            calendars: Mutex::default(),
            current: Mutex::default(),
            seen: Mutex::default(),
        }
    }

    // ------------------------------------------------------------------ helpers

    async fn get_json(&self, url: &str) -> Result<Value, String> {
        let mut last = String::new();
        for attempt in 0..3u64 {
            match self.http.get(url).send().await {
                Ok(resp) if resp.status().is_success() => match resp.json::<Value>().await {
                    Ok(v) => return Ok(v),
                    Err(e) => last = e.to_string(),
                },
                Ok(resp) => last = format!("HTTP {}", resp.status()),
                Err(e) => last = e.to_string(),
            }
            tokio_sleep(Duration::from_millis(400 * (attempt + 1))).await;
        }
        Err(format!("Could not reach ESPN ({last})"))
    }

    fn disk_get<T: DeserializeOwned>(&self, rel: &str) -> Option<T> {
        let text = std::fs::read_to_string(self.dir.join(rel)).ok()?;
        serde_json::from_str(&text).ok()
    }

    fn disk_put<T: Serialize>(&self, rel: &str, value: &T) {
        let path = self.dir.join(rel);
        let tmp = path.with_extension("tmp");
        if let Ok(text) = serde_json::to_string(value) {
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }

    fn mem_get<T: Clone>(map: &Timed<T>, key: &str) -> Option<T> {
        let map = map.lock().unwrap();
        map.get(key)
            .filter(|(t, _)| t.elapsed() < LIVE_TTL)
            .map(|(_, v)| v.clone())
    }

    fn mem_put<T>(map: &Timed<T>, key: String, value: T) {
        map.lock().unwrap().insert(key, (Instant::now(), value));
    }

    /// Forget everything that might still change (unfinished weeks, calendars).
    pub fn clear_live(&self) {
        self.weeks.lock().unwrap().clear();
        self.calendars.lock().unwrap().clear();
        self.current.lock().unwrap().clear();
    }

    pub fn seen_game(&self, id: &str) -> Option<Game> {
        self.seen.lock().unwrap().get(id).cloned()
    }

    // ------------------------------------------------------------------ calendar

    pub async fn current_season(&self) -> Result<i32, String> {
        if let Some(s) = Self::mem_get(&self.current, "season") {
            return Ok(s);
        }
        let data = self.get_json(&format!("{BASE}/scoreboard")).await?;
        let season = data["season"]["year"].as_i64().ok_or("Unexpected ESPN data")? as i32;
        Self::mem_put(&self.current, "season".into(), season);
        Ok(season)
    }

    /// Weeks (regular season + playoffs) of a season that have already started.
    pub async fn season_weeks(&self, season: i32) -> Result<Vec<WeekInfo>, String> {
        let key = season.to_string();
        if let Some(w) = Self::mem_get(&self.calendars, &key) {
            return Ok(w);
        }
        let data = self
            .get_json(&format!("{BASE}/scoreboard?dates={season}&seasontype=2&week=1"))
            .await?;
        let now = Utc::now();
        let mut weeks = Vec::new();
        let empty = vec![];
        let calendar = data["leagues"][0]["calendar"].as_array().unwrap_or(&empty);
        for block in calendar {
            let stype: u8 = str_of(block, "value").parse().unwrap_or(0);
            if stype != REGULAR && stype != POST {
                continue;
            }
            for e in block["entries"].as_array().unwrap_or(&empty) {
                let label = str_of(e, "label");
                if label.to_lowercase().contains("pro bowl") {
                    continue;
                }
                // Include weeks starting within the next days for the gameday plan.
                let horizon = now + chrono::Duration::days(4);
                let started = parse_date(&str_of(e, "startDate")).map_or(false, |d| d <= horizon);
                if !started {
                    continue;
                }
                let short = if stype == REGULAR {
                    label.replace("Week ", "W")
                } else {
                    match label.as_str() {
                        "Wild Card" => "WC".into(),
                        "Divisional Round" => "DIV".into(),
                        "Conference Championship" => "CONF".into(),
                        "Super Bowl" => "SB".into(),
                        other => other.to_string(),
                    }
                };
                weeks.push(WeekInfo {
                    season_type: stype,
                    week: str_of(e, "value").parse().unwrap_or(0),
                    label: short,
                    long_label: label,
                });
            }
        }
        Self::mem_put(&self.calendars, key, weeks.clone());
        Ok(weeks)
    }

    // ------------------------------------------------------------------ teams

    pub async fn teams(&self) -> Result<Vec<Team>, String> {
        if let Some(t) = self.disk_get::<Vec<Team>>("teams.json") {
            return Ok(t);
        }
        let data = self.get_json(&format!("{BASE}/teams")).await?;
        let mut teams = Vec::new();
        if let Some(list) = data["sports"][0]["leagues"][0]["teams"].as_array() {
            for t in list {
                let t = &t["team"];
                teams.push(Team {
                    id: str_of(t, "id"),
                    abbr: str_of(t, "abbreviation"),
                    name: str_of(t, "displayName"),
                    short: str_of(t, "shortDisplayName"),
                    logo: t["logos"][0]["href"].as_str().unwrap_or("").to_string(),
                    color: str_of(t, "color"),
                });
            }
        }
        teams.sort_by(|a, b| a.name.cmp(&b.name));
        if !teams.is_empty() {
            self.disk_put("teams.json", &teams);
        }
        Ok(teams)
    }

    // ------------------------------------------------------------------ weeks

    pub async fn week_games(&self, season: i32, stype: u8, week: u8) -> Result<Vec<Game>, String> {
        let key = format!("{season}-{stype}-{week}");
        let rel = format!("weeks/{key}.json");
        let cached = self
            .disk_get::<Vec<Game>>(&rel)
            .or_else(|| Self::mem_get(&self.weeks, &key));
        let games = match cached {
            Some(g) => g,
            None => {
                let games = self.fetch_week(season, stype, week).await?;
                if !games.is_empty() && games.iter().all(|g| g.completed) {
                    self.disk_put(&rel, &games);
                } else {
                    Self::mem_put(&self.weeks, key, games.clone());
                }
                games
            }
        };
        let mut seen = self.seen.lock().unwrap();
        for g in &games {
            seen.insert(g.id.clone(), g.clone());
        }
        Ok(games)
    }

    async fn fetch_week(&self, season: i32, stype: u8, week: u8) -> Result<Vec<Game>, String> {
        let data = self
            .get_json(&format!(
                "{BASE}/scoreboard?dates={season}&seasontype={stype}&week={week}"
            ))
            .await?;
        let mut games = Vec::new();
        let empty = vec![];
        for ev in data["events"].as_array().unwrap_or(&empty) {
            let comp = &ev["competitions"][0];
            let status = &comp["status"]["type"];
            let mut home = None;
            let mut away = None;
            for c in comp["competitors"].as_array().unwrap_or(&empty) {
                let t = &c["team"];
                let side = Side {
                    team: Team {
                        id: str_of(t, "id"),
                        abbr: str_of(t, "abbreviation"),
                        name: str_of(t, "displayName"),
                        short: str_of(t, "shortDisplayName"),
                        logo: str_of(t, "logo"),
                        color: str_of(t, "color"),
                    },
                    score: num_of(&c["score"]).unwrap_or(0.0) as i32,
                };
                match c["homeAway"].as_str() {
                    Some("home") => home = Some(side),
                    _ => away = Some(side),
                }
            }
            let (Some(home), Some(away)) = (home, away) else { continue };
            let broadcast = comp["broadcasts"]
                .as_array()
                .unwrap_or(&empty)
                .iter()
                .flat_map(|b| b["names"].as_array().cloned().unwrap_or_default())
                .filter_map(|n| n.as_str().map(str::to_string))
                .collect::<Vec<_>>()
                .join(", ");
            let note = comp["notes"]
                .as_array()
                .unwrap_or(&empty)
                .iter()
                .map(|n| str_of(n, "headline"))
                .collect::<Vec<_>>()
                .join(" ");
            let odds = &comp["odds"][0];
            let line = num_of(&odds["spread"]).map(|spread| Line {
                favorite: ["homeTeamOdds", "awayTeamOdds"]
                    .iter()
                    .find(|k| odds[**k]["favorite"].as_bool() == Some(true))
                    .map(|k| if *k == "homeTeamOdds" { home.team.id.clone() } else { away.team.id.clone() }),
                spread: spread.abs(),
                over_under: num_of(&odds["overUnder"]),
            });
            games.push(Game {
                neutral: comp["neutralSite"].as_bool().unwrap_or(false),
                line,
                id: str_of(ev, "id"),
                date: str_of(ev, "date"),
                state: str_of(status, "state"),
                completed: status["completed"].as_bool().unwrap_or(false),
                venue: str_of(&comp["venue"], "fullName"),
                broadcast,
                note,
                home,
                away,
            });
        }
        games.sort_by(|a, b| a.date.cmp(&b.date));
        Ok(games)
    }

    pub async fn many_weeks(&self, season: i32, keys: &[(u8, u8)]) -> Vec<Vec<Game>> {
        join_all(keys.iter().map(|(t, w)| self.week_games(season, *t, *w)))
            .await
            .into_iter()
            .map(|r| r.unwrap_or_default())
            .collect()
    }

    // ------------------------------------------------------------------ game details

    /// Condensed facts of a finished game (cached on disk forever).
    pub async fn game_facts(&self, id: &str) -> Result<GameFacts, String> {
        let rel = format!("games/{id}.json");
        if let Some(f) = self.disk_get::<GameFacts>(&rel) {
            return Ok(f);
        }
        let data = self.get_json(&format!("{BASE}/summary?event={id}")).await?;
        let empty = vec![];

        let plays = data["scoringPlays"]
            .as_array()
            .unwrap_or(&empty)
            .iter()
            .map(|p| ScoringPlay {
                home: num_of(&p["homeScore"]).unwrap_or(0.0) as i32,
                away: num_of(&p["awayScore"]).unwrap_or(0.0) as i32,
                period: num_of(&p["period"]["number"]).unwrap_or(0.0) as u32,
                clock: num_of(&p["clock"]["value"]).unwrap_or(0.0),
            })
            .collect();

        let wp = data["winprobability"]
            .as_array()
            .unwrap_or(&empty)
            .iter()
            .filter_map(|x| num_of(&x["homeWinPercentage"]))
            .collect();

        let mut line = None;
        for pc in data["pickcenter"].as_array().unwrap_or(&empty) {
            let Some(spread) = num_of(&pc["spread"]) else { continue };
            let favorite = ["homeTeamOdds", "awayTeamOdds"]
                .iter()
                .find(|k| pc[**k]["favorite"].as_bool() == Some(true))
                .map(|k| str_of(&pc[*k], "teamId"));
            line = Some(Line {
                favorite,
                spread: spread.abs(),
                over_under: num_of(&pc["overUnder"]),
            });
            break;
        }

        let periods = data["header"]["competitions"][0]["competitors"]
            .as_array()
            .unwrap_or(&empty)
            .iter()
            .map(|c| c["linescores"].as_array().map_or(0, |l| l.len()))
            .max()
            .unwrap_or(0) as u32;

        let facts = GameFacts { plays, wp, line, periods };
        self.disk_put(&rel, &facts);
        Ok(facts)
    }

    pub async fn many_facts(&self, ids: &[String]) -> HashMap<String, GameFacts> {
        let results = join_all(ids.iter().map(|id| self.game_facts(id))).await;
        ids.iter()
            .cloned()
            .zip(results)
            .filter_map(|(id, r)| match r {
                Ok(f) => Some((id, f)),
                Err(e) => {
                    eprintln!("could not load details for game {id}: {e}");
                    None
                }
            })
            .collect()
    }
}

async fn tokio_sleep(d: Duration) {
    tauri::async_runtime::spawn_blocking(move || std::thread::sleep(d))
        .await
        .ok();
}
