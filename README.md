# NFL Watch

A desktop app that tells you **which NFL games are worth watching on Game Pass
without spoiling the results**. Every finished game gets a watchability rating
from 1 to 5 bars ("Skip it" … "Must watch"), grouped by week.

## Features

- **Spoiler-free by design.** Scores never leave the Rust backend. The UI only
  receives teams, kickoff time, pre-game records, the pre-game betting line and
  the rating.
- **Ratings** combine:
  - how close the final margin was, and whether it went to overtime
  - win-probability swings (an "excitement index" built from ESPN's play-by-play win probability)
  - lead changes and ties
  - comebacks
  - go-ahead or tying scores in the last 5 minutes or in overtime
  - underdog upsets against the spread
  - shootouts and low-scoring defensive battles
  - top matchups between two good teams
- **Gameday plan** for upcoming games. Each gameday is split into slots by
  kickoff time in your local time:
  - **Afternoon** (international games, e.g. Munich or London at 15:30): watch the full game
  - **Evening** (19:00): watch the full game
  - **Late** (22:05 / 22:25): watch the first half, recommended only if the game looks promising
  - **Night**: too late to watch live, so catch the replay; it gets rated after the game

  Each slot gets a **top pick** (your team, if it's playing) ranked by a
  pre-game **hype** score. Hype uses the betting line, the over/under, both
  teams' records, top matchups and division rivalries. Over the 2025 season,
  games rated "Can't miss" before kickoff turned out much more watchable than
  "Low hype" games. You can change the time windows in Settings.
- **Why this rating?** shows the reasons, such as "Overtime" or "Comeback".
  They are hidden by default because they are mild spoilers.
- **My teams:** games of your favorite teams are always shown at the top. An
  optional setting hides their ratings too.
- **Watched games** can be marked, dimmed or hidden. Once a game is marked as
  watched, you can reveal its final score.
- New data is fetched every time the app starts (and with ↻). Finished games are
  cached, so later starts are fast.

## Run / build

Requirements: Rust (stable), the Tauri CLI (`cargo install tauri-cli`), and WebView2 (included with Windows 11).

```sh
cd src-tauri
cargo tauri dev      # run in development mode
cargo tauri build    # release build + installer
```

Release outputs:
- `src-tauri/target/release/nfl-watch.exe`: standalone app
- `src-tauri/target/release/bundle/nsis/NFL Watch_0.1.0_x64-setup.exe`: installer

## Data

- Source: ESPN's public NFL API (no account needed).
- Your favorites, watched games and settings are stored in
  `%APPDATA%\com.nflwatch.desktop\user-data.json`.
- The cache is in `%APPDATA%\com.nflwatch.desktop\cache\`. Delete it at any time.

## Tuning the rating

The formula lives in `src-tauri/src/rating.rs`. To check the star distribution
over a whole season, without showing teams or scores:

```sh
cd src-tauri
SEASON=2025 cargo test calibration -- --ignored --nocapture
SEASON=2025 cargo test hype_vs_rating -- --ignored --nocapture   # does hype predict watchability?
```

## Layout

```
ui/                 frontend (plain HTML/CSS/JS, no build step)
src-tauri/src/
  main.rs           Tauri commands + spoiler-free view models
  espn.rs           ESPN API client + disk cache
  rating.rs         watchability scoring
  store.rs          favorites / watched / settings persistence
```
