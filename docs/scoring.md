# How the scoring works

NFL Watch gives games two different scores:

| | **Watchability** | **Hype** |
|---|---|---|
| Question it answers | *Was this game fun to watch?* | *Will this game be fun to watch?* |
| Rates | finished games | upcoming and live games |
| Uses | everything, including the result (kept on the backend) | pre-game information only |
| Shown as | 0–100 score and 1–5 bars: *Skip it … Must watch* | 0–100 score and 1–5 bars: *Low hype … Can't miss* |
| Spoiler risk | reasons are hidden behind a click | none, so reasons are always shown |

Both are implemented in [`src-tauri/src/rating.rs`](../src-tauri/src/rating.rs).
Each score is a sum of components. `watch_parts()` and `hype_parts()` compute
the components, and `Scorer::rate()` and `Scorer::hype()` turn them into a
score. This document describes the formulas exactly as they are in the code.

The sections below describe the **default weights**. If you change the
weights in Settings, the components are combined differently, as described in
[Custom weights](#custom-weights).

---

## Data sources

All data comes from ESPN's public site API:

| Data | Endpoint | Used for |
|---|---|---|
| Schedule, final score, status, neutral-site flag, pre-game line of upcoming games | `scoreboard` | everything |
| Scoring plays: running score, quarter and clock after each score | `summary` → `scoringPlays` | lead changes, ties, comebacks, late drama, overtime |
| Home win probability after every play | `summary` → `winprobability` | excitement index |
| Closing spread and over/under of finished games | `summary` → `pickcenter` | upsets, line chip |

### Pre-game records
Team records are always **computed by NFL Watch itself** from all completed
regular-season games *before* the week being viewed. For playoff games, the
full regular-season record is used. ESPN's own records are never used,
because after a game they already include that game's result, which would be
a spoiler.

A record's *win percentage* counts ties as half a win. A team with no games
yet counts as 0.500.

### Top matchup
A game is a **top matchup** if all of the following are true:
- both teams have played at least 3 games
- both have a win percentage of at least 0.600
- their average win percentage is at least 0.650

Both scores use this.

---

## Watchability (finished games)

With default weights, the score is the **sum of nine components**, rounded
and clamped to 0–100.
The components overlap on purpose: a truly great game, such as a comeback
with a late go-ahead score in a close finish with big win-probability swings,
collects points from several of them and saturates near 100. A blowout
collects almost nothing.

### 1. Closeness of the final result (−10 to 25)

| Final margin | Points |
|---|---|
| 0 (tie) | 22 |
| 1–3 | 25 |
| 4–7 | 18 |
| 8–10 | 10 |
| 11–14 | 4 |
| 15–20 | 0 |
| 21 or more | **−10** (blowout penalty) |

Reasons: *Nail-biter to the end* (margin ≤ 3) or *One-score game* (margin 4–8).

### 2. Overtime (0 or 10)
+10 if the game had more than four periods. Reason: *Overtime*.

### 3. Excitement index (0 to 25)
This component uses the play-by-play **home win probability**. The excitement
index is the total amount the win probability moved over the whole game:

```
excitement = Σ |WP(play i) − WP(play i−1)|
points     = clamp((excitement − 1.5) × 5, 0, 25)
```

A game that is decided early barely moves the needle, with an index of around
1–2. A seesaw battle where both teams look like winners several times reaches
5–10. The index rewards drama *during* the game, not just a close final score.
For example, a game that looked lost and was won late scores high even if the
final margin was 7.

| Excitement index | Points |
|---|---|
| ≤ 1.5 | 0 |
| 3.0 | 7.5 |
| 4.5 | 15 |
| ≥ 6.5 | 25 (maximum) |

Reason: *Wild momentum swings* (index ≥ 5).

**Fallback:** if ESPN has no win-probability data for a game (10 data points
or fewer), this component is 12 points if the final margin was ≤ 8, and 4
points otherwise.

### 4. Lead changes and ties (0 to 15)
These are counted from the running score after every scoring play:
- **Lead change:** a team takes the lead after the *other* team was the last
  to lead, even if the score was tied in between.
- **Tie:** the score becomes level from a non-tied score. The 0–0 at kickoff
  doesn't count.

```
points = min(3 × lead changes + 1 × ties, 15)
```

Reason: *Back-and-forth (N lead changes)* when there were 3 or more lead changes.

### 5. Comeback (0 to 14)
This is the largest deficit the eventual winner faced at any point:

| Winner's largest deficit | Points |
|---|---|
| 17 or more | 14 |
| 14–16 | 12 |
| 10–13 | 8 |
| 7–9 | 5 |
| less than 7 | 0 |

Reasons: *Big comeback* (≥ 10) or *Comeback* (7–9). Tied games get no
comeback points.

### 6. Late drama (0, 9 or 12)
This checks for a **go-ahead or game-tying score** late in the game: a score
by a team that was trailing or tied before it, and is leading or tied after
it. Only the most dramatic such score counts:

| When | Points |
|---|---|
| Overtime, or the last 2:00 of the 4th quarter | 12 |
| 5:00–2:01 left in the 4th quarter | 9 |

A score that leaves the scoring team still behind doesn't count. Reason:
*Late go-ahead or tying score*.

### 7. Upset (0, 6 or 10)
This uses the closing betting line. If the pre-game favorite lost:

| Spread | Points |
|---|---|
| 7 or more | 10 |
| 3 – 6.5 | 6 |
| less than 3 | 0 (not really an upset) |

Reason: *Underdog upset*.

### 8. Scoring extremes (0 to 12)
Only the first matching row applies:

| Total points | Condition | Points | Reason |
|---|---|---|---|
| ≥ 65 | | 12 | *Shootout* |
| 55–64 | | 8 | *High-scoring* |
| 48–54 | margin ≤ 10 | 4 | none |
| ≤ 20 | margin ≤ 7 | 10 | *Defensive battle* |
| ≤ 20 | margin > 7 | 5 | *Defensive battle* |

Very low-scoring games get a bonus on purpose. A 13–10 slugfest can be very
tense to watch.

### 9. Top matchup (0 or 6)
+6 if the game was a [top matchup](#top-matchup). This bonus has no reason
text.

### From score to bars

| Score | Bars | Label |
|---|---|---|
| 75–100 | 5 | Must watch |
| 58–74 | 4 | Worth it |
| 40–57 | 3 | Solid |
| 15–39 | 2 | Only if bored |
| 0–14 | 1 | Skip it |

If a game earned no reason at all, its reason list says *Not much drama*.

The cutoffs were calibrated on the full 2025 season (285 games, playoffs
included):

| Bars | Share of games |
|---|---|
| 1 (Skip it) | 31% |
| 2 (Only if bored) | 26% |
| 3 (Solid) | 14% |
| 4 (Worth it) | 13% |
| 5 (Must watch) | 15% |

The bottom tiers are large because many NFL games are lopsided by the fourth
quarter. *Must watch* is meant to cover roughly two games per week.

### Worked examples
These are made-up games for illustration.

**A thriller.** Final margin 3, 51 total points, no overtime. The winner was
down 10, there were 4 lead changes and 1 tie, the go-ahead field goal came
with 0:45 left, the winner was a 3.5-point underdog, and the excitement index
was 6.2.

| Component | Points |
|---|---|
| Closeness (margin 3) | 25 |
| Excitement ((6.2 − 1.5) × 5) | 23.5 |
| Lead changes (4 × 3 + 1) | 13 |
| Comeback (10 points down) | 8 |
| Late drama (inside 2:00) | 12 |
| Upset (spread 3.5) | 6 |
| Scoring (51 total, margin ≤ 10) | 4 |
| **Total** | **91.5 → 92 → Must watch** |

**A blowout.** Final margin 24, 44 total points. The loser kicked an early
field goal, and then the winner took over for good. Excitement index 1.9.

| Component | Points |
|---|---|
| Closeness (margin ≥ 21) | −10 |
| Excitement ((1.9 − 1.5) × 5) | 2 |
| Lead changes (1 × 3) | 3 |
| Comeback (3 points down) | 0 |
| **Total** | **−5 → 0 → Skip it** |

---

## Hype (upcoming games)

Hype uses only information that is public **before kickoff**, so it can't
spoil anything. With default weights, it is the sum of five components,
rounded and clamped to 0–100. The highest reachable value is 91.

### 1. Betting line: expected closeness (0 to 30)

| Spread | Points | Reason |
|---|---|---|
| ≤ 1.5 | 30 | *Coin-flip line* |
| 2 – 3.5 | 25 | *Tight line (x)* |
| 4 – 5.5 | 17 | |
| 6 – 7.5 | 10 | |
| 8 – 10 | 4 | *Big favorite (x)* at exactly 10 |
| > 10 | 0 | *Big favorite (x)* |

If no line is available yet, this component is 12.

### 2. Over/under: expected points (0 to 15)

| Over/under | Points |
|---|---|
| ≥ 52 | 15 |
| 48 – 51.5 | 11 |
| 45 – 47.5 | 7 |
| 42 – 44.5 | 3 |
| < 42 | 0 |

Reasons: *Shootout potential* (O/U ≥ 50) and *Defensive slugfest* (O/U ≤ 38).
The second one is informational only and adds no points.

### 3. Team quality (−8 to 30)
If both teams have played at least 2 games:

```
points = average win percentage of both teams × 30
```

If both teams are below 0.350 (with at least 3 games played), 8 points are
subtracted. Reason: *Two struggling teams*.

Earlier in the season, when records don't mean much yet, this component is a
neutral 15.

### 4. Top matchup (0 or 10)
+10 for a [top matchup](#top-matchup). Reason: *Top matchup*.

### 5. Division rivals (0 or 6)
+6 if both teams play in the same division. Reason: *Division rivals*.

International games, which are regular-season games at a neutral site, get
the reason *International game* but no extra points.

### From score to bars

| Score | Bars | Label |
|---|---|---|
| 50–91 | 5 | Can't miss |
| 39–49 | 4 | Promising |
| 31–38 | 3 | Decent |
| 24–30 | 2 | Meh |
| 0–23 | 1 | Low hype |

The cutoffs were calibrated on the 2025 regular season from week 5 onward.
Earlier weeks were left out because records don't mean much there.

### How well does hype predict watchability?
For each 2025 game, I compared the hype the game would have had before kickoff
(using the closing line and records up to that week) with the watchability it
actually earned. This covers 208 games from week 5 onward:

| Hype | Games | Average watchability afterwards |
|---|---|---|
| 1 – Low hype | 35 | 24.9 |
| 2 – Meh | 57 | 39.6 |
| 3 – Decent | 54 | 35.1 |
| 4 – Promising | 39 | 40.2 |
| 5 – Can't miss | 23 | 52.4 |

The correlation between the two scores is **0.22**. That's a real but modest
signal: a *Can't miss* game is on average clearly better than a *Low hype*
one, but the middle tiers overlap (2 bars happened to beat 3 bars in 2025),
and any single game can still surprise you. This is expected. The betting
market already tries to predict close games, and much of what makes football
fun (turnovers, late drives) can't be predicted. That is why the after-game
watchability rating stays the main feature, and hype is only used to plan
live viewing.

### Worked example
Spread 2.5, over/under 49.5. The teams are 7-2 and 6-3, and they're division
rivals.

| Component | Points |
|---|---|
| Line (2.5) | 25 |
| Over/under (49.5) | 11 |
| Quality ((0.778 + 0.667) / 2 × 30) | 21.7 |
| Top matchup | 10 |
| Division rivals | 6 |
| **Total** | **73.7 → 74 → Can't miss** |

---

## Custom weights

*Settings → Rating weights* has a slider for every component of both scores:
the nine watchability components and the five hype components. A slider goes
from 0 (off) through 1 (the default) up to 3×.

### Why the weights aren't just multiplied in
The obvious approach is `score = Σ weightᵢ × componentᵢ`, maybe with the
weights normalized to sum to 1. It doesn't work, for two reasons:

1. **The components have very different ranges.** *Close final score* spans
   −10 to 25 and differs a lot from game to game. *Top matchup* is 0 or 6 and
   is 0 for most games. A weight only says how much a component is
   multiplied by, not how much it actually moves the score. Normalizing the
   weights doesn't change that.
2. **The star cutoffs only fit the default formula.** The cutoffs (75, 58, 40,
   15) were calibrated on the default score distribution, and weights change
   that distribution. Halving *Close final score* pulls nearly every game down
   a tier. Tripling *Momentum swings* pushes most good games past 100.

### The calculation
A weighted score is computed in five steps:

1. **Components.** Compute the component points exactly as described above
   (`watch_parts()` and `hype_parts()` in `rating.rs`).
2. **Weighted sum.** `weighted = Σ weightᵢ × componentᵢ`
3. **Rank in the reference season.** The app embeds the component points of
   every 2025 game (`src-tauri/data/watch-sample.csv` and `hype-sample.csv`).
   It computes the weighted sum of every reference game too, then finds where
   this game's weighted sum ranks among them.
4. **Default score at the same rank.** The score is the *default* score of the
   reference season at that rank. If a game's weighted sum beats 94% of the
   reference games, it gets the default score that beats 94% of them.
5. **Round, clamp and convert to bars** with the usual cutoffs.

In formula terms this is quantile matching. With *F_w* the distribution of
weighted sums in the reference season and *Q₁* the quantile function of the
default scores:

```
score = Q₁( F_w( Σ weightᵢ × componentᵢ ) )
```

Implementation details (`Model` in `rating.rs`):
- Both lists of reference scores, weighted and default, are sorted and paired
  by position. Reference games with the same weighted sum share the average of
  their default scores.
- Between those points the mapping interpolates linearly. Outside the range of
  the reference season it extends linearly, with the average slope of the whole
  range, so a record-breaking game can still exceed the best 2025 game.
- With every weight at exactly 1, the mapping is skipped and the score is the
  plain sum, as before.

The reference samples hold only component points. They have no teams, dates
or game ids, and the rows are sorted, so they can't be matched to games.
Watchability uses all 285 games of 2025, playoffs included. Hype uses the 208
regular-season games from week 5 on, the same set its cutoffs were calibrated
on.

### What this guarantees
- **Default weights give the default formula**, so nothing changes until you
  move a slider.
- **Only the ratios between weights matter.** Setting every slider to 2×
  ranks the games exactly as setting them all to 1× does, so it gives the
  same scores.
- **The star shares stay the same.** About 15% of games are still *Must
  watch* and about 30% are still *Skip it*. What changes is *which* games end
  up there. Tested in `weighting_keeps_the_star_shares`.
- **The score keeps its meaning.** A 75 is still roughly the top 15% of games,
  whatever the weights.

### Worked example
Take the two made-up games from the [watchability examples](#worked-examples),
with *Close final score* switched off and *Momentum swings* at 3×:

| | Thriller | Blowout |
|---|---|---|
| Default score (sum of components) | 91.5 | −5 |
| … beats this share of 2025 games | 96% | 9% |
| Weighted sum: closeness × 0, excitement × 3 | 0 + 3 × 23.5 + 13 + 8 + 12 + 6 + 4 = **113.5** | 0 + 3 × 2 + 3 = **9** |
| … beats this share of 2025 games (weighted) | 94% | 13% |
| Score: default score at that rank | **87 → Must watch** | **−0.9 → 0 → Skip it** |

The weighted sum of the thriller, 113.5, would be meaningless on the 0–100
scale. Mapped by rank, it becomes 87. That is a little lower than before,
because the game's close finish no longer counts, but it is still clearly
*Must watch*. The blowout climbs from the 9th to the 13th percentile, because
the margin penalty no longer counts, but it stays *Skip it*.

For the whole 2025 season, these weights move 52 of 285 games (18%) to a
different tier. The tier shares stay the same.

### Influence
Next to each slider, the settings screen shows the criterion's share of the
influence:

```
influenceᵢ = weightᵢ × σᵢ / Σⱼ weightⱼ × σⱼ
```

σᵢ is the standard deviation of the component's points across the reference
season. A component that is the same for almost every game, such as *Top
matchup*, can't separate games much, whatever its weight. With the default
weights:

| Watchability | σ | Influence | | Hype | σ | Influence |
|---|---|---|---|---|---|---|
| Close final score | 12.8 | 28% | | Close betting line | 7.0 | 30% |
| Momentum swings | 7.9 | 17% | | Team quality | 6.2 | 26% |
| Late drama | 5.2 | 11% | | Expected points | 3.9 | 16% |
| Lead changes | 4.6 | 10% | | Top matchup | 3.7 | 15% |
| Scoring extremes | 4.4 | 10% | | Division rivalry | 2.9 | 12% |
| Comebacks | 3.7 | 8% | | | | |
| Overtime | 2.3 | 5% | | | | |
| Upsets | 2.2 | 5% | | | | |
| Top matchup | 2.3 | 5% | | | | |

The figure ignores correlations between components (comebacks often come
with lead changes), so treat it as a guide, not an exact decomposition.

### Reasons
If you switch a criterion off (weight 0), its reasons are no longer shown. For
example, *Underdog upset* doesn't appear when upsets don't count.

---

## Gameday plan: how picks are chosen

Upcoming games are grouped by gameday and sorted into slots by **local**
kickoff time. The windows are set in *Settings → Live viewing windows*:

| Slot | Kickoff (defaults) | Recommendation |
|---|---|---|
| Afternoon | 09:00 – 16:59 | full game |
| Evening | 17:00 – 20:29 | full game |
| Late | 20:30 – 23:29 | first half, only if promising |
| Night | 23:30 – 08:59 | not live; rated after the game |

Night games after midnight belong to the previous day's gameday, so Sunday
Night Football at 02:20 appears under Sunday.

Within each slot, games are ranked by:
1. games of your favorite teams first
2. then by hype score, highest first
3. then by kickoff time

The first game becomes the slot's **top pick**, or **your team** if one of
your teams plays in that slot.

In the **Late** slot, a pick is made only if the best game is *promising*,
meaning your team plays or it has at least 4 hype bars. Otherwise the plan
suggests going to bed and checking the ratings the next day.

---

## Spoiler protection, summarized

- Final scores, the scoring timeline and win probabilities are read only by
  the Rust backend. The UI receives the 0–100 score, the bars, the label and
  the list of reasons.
- Records shown in the UI are pre-game records computed by the app.
- Watchability reasons are hidden until you click **Why this rating?**, or
  switch on *Always show why* in Settings.
- With *Hide ratings for my teams' games* enabled, your teams' finished games
  show no rating. They are also sorted by kickoff time instead of score, so
  the order doesn't give anything away.
- **Reveal result** works only for games marked as watched. The backend
  refuses it otherwise.

## Tuning the formulas

Change the numbers in `rating.rs`, then check the effect on a whole season.
These tests print aggregates only, never teams or scores:

```sh
cd src-tauri
SEASON=2025 cargo test calibration -- --ignored --nocapture      # watchability star distribution
SEASON=2025 cargo test hype_vs_rating -- --ignored --nocapture   # hype vs. actual watchability
```

After changing a component's formula, regenerate the reference samples used
for [custom weights](#custom-weights). The command prints only row counts:

```sh
SEASON=2025 cargo test export_samples -- --ignored --nocapture
```

Finished games are cached on disk, so you can iterate quickly after the first run.
