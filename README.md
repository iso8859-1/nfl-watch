# NFL Watch

**Spoiler-free NFL game recommendations for people who watch games after they happen.**

If you watch the NFL on Game Pass from Europe, most games end in the middle of
your night. You wake up with a whole slate of replays and no idea which ones
are worth your time, and every attempt to find out risks spoiling the result.

NFL Watch rates every finished game by how *fun it was to watch*: close games,
comebacks, late drama, shootouts and upsets. It never shows you a score.
For upcoming games it builds a **gameday plan** that fits your local time zone.

![Gameday plan](docs/gameday-plan.png)

---

## Features

### Spoiler-free ratings for finished games
Every finished game gets a watchability score from 0 to 100, shown as 1–5 bars:
*Skip it · Only if bored · Solid · Worth it · Must watch*.

- **Scores never reach the UI.** The Rust backend reads the results and sends
  the frontend only teams, kickoff time and the rating.
- **Pre-game records only.** ESPN's team records already include the game you
  haven't watched yet, so NFL Watch works out each team's record itself from
  earlier weeks.
- **The reasons are hidden by default.** Hints like *Overtime* or *Comeback*
  are mild spoilers, so they appear only when you click **Why this rating?**
  (or switch them on permanently in Settings).
- **Ratings for your teams can be hidden.** A low rating for your own team's
  game gives away a blowout, so there's an optional setting to hide those too.

### Gameday plan for upcoming games
Upcoming games are grouped by gameday and by kickoff slot in **your local
time**:

| Slot | Default window | What it suggests |
|---|---|---|
| Afternoon | 09:00 – 17:00 | Full game. International games in London, Munich, etc. land here |
| Evening | 17:00 – 20:30 | Full game |
| Late | 20:30 – 23:30 | First half only, and only if the game looks promising |
| Night | later | Too late to watch live; catch the replay, which gets rated after the game |

Each slot gets a **top pick** based on a pre-game **hype** score. Your favorite
team always takes the pick for its slot. All windows can be changed in Settings.

### Favorites and watch tracking
- **My teams:** your favorite teams' games always appear at the top, whatever
  their rating.
- **Mark watched:** watched games are dimmed or hidden. Once a game is marked
  as watched, **Reveal result** shows the final score.
- **Seasons and weeks:** browse the regular season and playoffs of the current
  season and the four before it.
- **Fresh data on every start:** finished games are cached, so later starts
  are fast.

---

## How the ratings work

### Watchability (after the game)
The score adds up several independent signals of entertainment value:

| Signal | Source |
|---|---|
| Final margin, ties, overtime | final result |
| **Excitement index**: total swing in win probability during the game | ESPN play-by-play win probability |
| Lead changes and ties | scoring timeline |
| Comebacks (largest deficit the winner overcame) | scoring timeline |
| Go-ahead or tying scores in the last 5 minutes or in overtime | scoring timeline |
| Underdog wins | pre-game betting line |
| Shootouts and low-scoring defensive battles | total points |
| Top matchup (two good teams) | pre-game records |

The star cutoffs were calibrated on the full 2025 season. About 15% of games
come out as *Must watch* and about 30% as *Skip it*.

### Hype (before the game)
Hype uses only information available before kickoff:
- how close the betting line is
- the expected total points (over/under)
- both teams' records so far
- top matchups
- division rivalries

Checked against the 2025 season, games rated *Can't miss* averaged 48/100
watchability afterwards, against 28/100 for *Low hype* games. That's a useful
but modest signal: plenty of great NFL games come out of nowhere, which is why
the after-game ratings stay the main feature.

**[Read the full scoring documentation →](docs/scoring.md)** It covers every
component, threshold and calibration result, with worked examples.

---

## Installation

Download the installer from the releases page if one is available, or build it
yourself.

### Requirements
- Windows 10/11 with WebView2 (already included in Windows 11)
- [Rust](https://rustup.rs) (stable)
- Tauri CLI: `cargo install tauri-cli --version "^2"`

macOS and Linux should work with Tauri's usual
[prerequisites](https://v2.tauri.app/start/prerequisites/), but they have not
been tested.

### Build

```sh
git clone https://github.com/iso8859-1/nfl-watch.git
cd nfl-watch/src-tauri

cargo tauri dev      # run in development mode
cargo tauri build    # optimized build + installer
```

Release outputs:
- `src-tauri/target/release/bundle/nsis/NFL Watch_<version>_x64-setup.exe`: the installer.
  It installs for the current user only and needs no admin rights.
- `src-tauri/target/release/nfl-watch.exe`: a standalone executable.

The installer is not code-signed, so Windows SmartScreen shows "Unknown
publisher" the first time you run it. Click *More info* and then *Run anyway*.

### Tests

```sh
cd src-tauri
cargo test
```

Two ignored tests fetch a whole season and print **only aggregate numbers**,
with no teams and no scores. You can use them to tune the formulas:

```sh
SEASON=2025 cargo test calibration -- --ignored --nocapture      # star distribution
SEASON=2025 cargo test hype_vs_rating -- --ignored --nocapture   # does hype predict watchability?
```

---

## Data and privacy

- Game data comes from ESPN's public site API. It needs no account or API key.
- Everything else stays on your machine:
  - `%APPDATA%\com.nflwatch.desktop\user-data.json`: favorites, watched games, settings
  - `%APPDATA%\com.nflwatch.desktop\cache\`: cached game data, which you can delete at any time

## Project layout

```
ui/                    frontend: plain HTML/CSS/JS, no build step
  index.html
  app.js
  style.css
src-tauri/
  tauri.conf.json      window, security policy (CSP), installer config
  src/
    main.rs            Tauri commands and spoiler-free view models
    espn.rs            ESPN API client and disk cache
    rating.rs          watchability and hype scoring
    store.rs           favorites, watched games and settings
docs/                  screenshots
```

## Disclaimer

This is an unofficial hobby project. It is not affiliated with, endorsed by or
connected to the NFL, NFL Game Pass, DAZN or ESPN. Team names and logos are
trademarks of their respective owners. The ESPN API is undocumented and may
change without notice.

## License

[MIT](LICENSE)
