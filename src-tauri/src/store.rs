//! Persistent user data: favorite teams, watched games, settings.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Show the reasons behind a rating without clicking (mild spoilers).
    pub show_reasons: bool,
    /// Hide ratings of games involving a favorite team (a low rating hints at a blowout).
    pub hide_favorite_ratings: bool,
    /// Hide games already marked as watched.
    pub hide_watched: bool,
    /// Minimum stars for the "Recommended" section.
    pub min_stars: u8,
    /// Live-viewing windows for the gameday plan (local time, "HH:MM"):
    /// games kicking off before `earliest_kickoff` count as night games,
    /// up to `full_game_before` you watch the full game,
    /// up to `first_half_before` only the first half.
    pub earliest_kickoff: String,
    pub full_game_before: String,
    pub first_half_before: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            show_reasons: false,
            hide_favorite_ratings: false,
            hide_watched: false,
            min_stars: 3,
            earliest_kickoff: "09:00".into(),
            full_game_before: "20:30".into(),
            first_half_before: "23:30".into(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UserData {
    pub favorites: BTreeSet<String>,
    pub watched: BTreeSet<String>,
    pub settings: Settings,
}

pub struct Store {
    path: PathBuf,
    pub data: UserData,
}

impl Store {
    pub fn load(path: PathBuf) -> Self {
        let data = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        Self { path, data }
    }

    pub fn save(&self) -> Result<(), String> {
        let text = serde_json::to_string_pretty(&self.data).map_err(|e| e.to_string())?;
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &self.path).map_err(|e| e.to_string())
    }
}
