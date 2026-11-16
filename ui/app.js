// NFL Watch UI. Talks only to the Rust backend, which never sends scores.
const invoke = window.__TAURI__.core.invoke;

const TIER_COLORS = ["", "var(--tier-1)", "var(--tier-2)", "var(--tier-3)", "var(--tier-4)", "var(--tier-5)"];

const state = {
  overview: null,
  weekIndex: 0,
  games: [],
  user: { favorites: [], watched: [], settings: {} },
  teams: [],
  revealed: new Set(),      // game ids whose rating reasons were opened
  shownRatings: new Set(),  // favorite games whose hidden rating was opened
  results: new Map(),       // game id -> revealed final score
  expandedSlots: new Set(), // gameday-plan slots showing all their games
  otherGamesOpen: false,    // "Other games" section expanded
  loadToken: 0,
};

const $ = (sel) => document.querySelector(sel);

function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v === null || v === undefined || v === false) continue;
    if (k === "class") node.className = v;
    else if (k === "style") node.style.cssText = v;
    else if (k.startsWith("on")) node.addEventListener(k.slice(2), v);
    else node.setAttribute(k, v === true ? "" : v);
  }
  for (const c of children.flat()) {
    if (c === null || c === undefined || c === false) continue;
    node.append(c instanceof Node ? c : document.createTextNode(String(c)));
  }
  return node;
}

// ------------------------------------------------------------------ loading

async function init() {
  wireControls();
  try {
    state.user = await invoke("get_user_data");
    await loadSeason(null);
  } catch (e) {
    showError(e);
  }
}

async function loadSeason(season) {
  $("#main").replaceChildren(el("div", { class: "loading" }, "Checking for new games…"));
  state.overview = await invoke("get_overview", { season });
  state.weekIndex = state.overview.defaultIndex;
  renderSeasons();
  renderWeeks();
  await loadWeek();
}

async function loadWeek() {
  const week = state.overview.weeks[state.weekIndex];
  if (!week) {
    $("#main").replaceChildren(el("div", { class: "empty" }, "No games yet this season."));
    return;
  }
  const token = ++state.loadToken;
  $("#main").replaceChildren(el("div", { class: "loading" }, `Rating ${week.longLabel} games…`));
  try {
    const games = await invoke("get_week", {
      season: state.overview.season,
      seasonType: week.seasonType,
      week: week.week,
    });
    if (token !== state.loadToken) return; // user switched weeks meanwhile
    state.games = games;
    renderWeek();
  } catch (e) {
    if (token === state.loadToken) showError(e);
  }
}

function showError(e) {
  $("#main").replaceChildren(
    el("div", { class: "error" },
      el("p", {}, String(e)),
      el("button", { class: "btn", onclick: () => location.reload() }, "Try again"))
  );
}

// ------------------------------------------------------------------ header / tabs

function renderSeasons() {
  const sel = $("#season");
  sel.replaceChildren(...state.overview.seasons.map((s) =>
    el("option", { value: s, selected: s === state.overview.season }, `${s} season`)));
}

function renderWeeks() {
  const nav = $("#weeks");
  nav.replaceChildren(...state.overview.weeks.map((w, i) =>
    el("button", {
      class: "week-tab" + (i === state.weekIndex ? " active" : ""),
      title: w.longLabel,
      onclick: () => { state.weekIndex = i; renderWeeks(); loadWeek(); },
    }, w.label)));
  nav.querySelector(".active")?.scrollIntoView({ inline: "center", block: "nearest" });
}

function wireControls() {
  $("#season").addEventListener("change", (e) => loadSeason(Number(e.target.value)).catch(showError));

  $("#btn-refresh").addEventListener("click", async (e) => {
    const btn = e.currentTarget;
    btn.disabled = true;
    btn.replaceChildren(el("span", { class: "spin" }, "↻"));
    try {
      await invoke("refresh");
      const season = state.overview?.season ?? null;
      const weekKey = state.overview?.weeks[state.weekIndex];
      state.overview = await invoke("get_overview", { season });
      const idx = weekKey
        ? state.overview.weeks.findIndex((w) => w.seasonType === weekKey.seasonType && w.week === weekKey.week)
        : -1;
      state.weekIndex = idx >= 0 ? idx : state.overview.defaultIndex;
      renderWeeks();
      await loadWeek();
    } catch (err) {
      showError(err);
    } finally {
      btn.disabled = false;
      btn.textContent = "↻";
    }
  });

  $("#btn-teams").addEventListener("click", openTeams);
  $("#btn-settings").addEventListener("click", openSettings);
}

// ------------------------------------------------------------------ week view

function isFavoriteRatingHidden(g) {
  return g.favorite && state.user.settings.hideFavoriteRatings && !state.shownRatings.has(g.id);
}

function renderWeek() {
  const week = state.overview.weeks[state.weekIndex];
  const s = state.user.settings;
  const games = state.games.filter((g) => !(s.hideWatched && g.watched));

  const finished = games.filter((g) => g.status === "final");
  const pending = games.filter((g) => g.status !== "final");

  const byScore = (a, b) => (a.watched - b.watched) || (b.rating.score - a.rating.score);
  const byKickoff = (a, b) => (a.watched - b.watched) || a.kickoff.localeCompare(b.kickoff);

  const favs = finished.filter((g) => g.favorite)
    // Sorting by score would leak the hidden rating, so keep kickoff order then.
    .sort(s.hideFavoriteRatings ? byKickoff : byScore);
  const others = finished.filter((g) => !g.favorite);
  const recommended = others.filter((g) => g.rating.stars >= s.minStars).sort(byScore);
  const rest = others.filter((g) => g.rating.stars < s.minStars).sort(byScore);
  const total = state.games.length;
  const watchedCount = state.games.filter((g) => g.watched).length;
  const finishedCount = state.games.filter((g) => g.status === "final").length;
  const stats = [
    `${total} games`,
    finishedCount < total ? `${finishedCount} finished` : null,
    `${recommended.length} recommended`,
    watchedCount ? `${watchedCount} watched` : null,
  ].filter(Boolean).join(" · ");

  const main = $("#main");
  main.replaceChildren(...[
    el("div", { class: "week-head" },
      el("h2", {}, `${week.longLabel}`, el("span", { class: "muted" }, `  ${state.overview.season}`)),
      el("div", { class: "week-stats" }, stats)),
    pending.length ? gamedayPlan(pending) : null,
    favs.length ? section("Your teams", favs) : null,
    finished.length
      ? section("Recommended", recommended, "Nothing reaches your threshold this week. Check the other games below.")
      : null,
    rest.length ? collapsible(`Other games (${rest.length})`, rest) : null,
    !games.length ? el("div", { class: "empty" }, "No games to show.") : null,
  ].filter(Boolean));
}

function section(title, games, emptyText) {
  return el("section", { class: "section" },
    el("h3", {}, title),
    games.length ? el("div", { class: "grid" }, games.map((g) => card(g))) : el("div", { class: "empty" }, emptyText || ""));
}

function collapsible(title, games) {
  // renderWeek() rebuilds the DOM on every click, so keep the open state.
  const node = el("details", { class: "section", open: state.otherGamesOpen },
    el("summary", {}, title),
    el("div", { class: "grid" }, games.map((g) => card(g))));
  node.addEventListener("toggle", () => { state.otherGamesOpen = node.open; });
  return node;
}

// ------------------------------------------------------------------ gameday plan

const SLOTS = {
  afternoon: { title: "Afternoon", mode: "Full game", pick: true },
  evening: { title: "Evening", mode: "Full game", pick: true },
  late: { title: "Late", mode: "First half, only if it's promising", pick: true, promisingOnly: true },
  night: { title: "Night", mode: "Too late to watch live. It gets rated after the final whistle, so catch the replay.", pick: false },
};
const SLOT_ORDER = ["afternoon", "evening", "late", "night"];
const PROMISING = 4; // hype stars a late game needs to be worth staying up for

const toMinutes = (hhmm) => {
  const [h, m] = String(hhmm || "0:0").split(":").map(Number);
  return h * 60 + (m || 0);
};
const fmtTime = (d) => d.toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit" });

/** Group upcoming games into gamedays and viewing slots (local time). */
function buildPlan(games) {
  const s = state.user.settings;
  const earliest = toMinutes(s.earliestKickoff);
  const full = toMinutes(s.fullGameBefore);
  const half = toMinutes(s.firstHalfBefore);
  const days = new Map();
  for (const g of games) {
    const d = new Date(g.kickoff);
    const mins = d.getHours() * 60 + d.getMinutes();
    let slot = "night";
    if (mins >= earliest && mins < full) slot = d.getHours() < 17 ? "afternoon" : "evening";
    else if (mins >= full && mins < half) slot = "late";
    // Games after midnight belong to the previous evening's gameday.
    const day = new Date(d.getFullYear(), d.getMonth(), d.getDate());
    if (mins < earliest) day.setDate(day.getDate() - 1);
    const key = day.getTime();
    if (!days.has(key)) days.set(key, { day, slots: {} });
    (days.get(key).slots[slot] ??= []).push(g);
  }
  const rank = (a, b) => (b.favorite - a.favorite) || (b.hype.score - a.hype.score) || a.kickoff.localeCompare(b.kickoff);
  for (const d of days.values()) for (const list of Object.values(d.slots)) list.sort(rank);
  return [...days.values()].sort((a, b) => a.day - b.day);
}

function gamedayPlan(games) {
  const plan = buildPlan(games);
  return el("section", { class: "section plan" },
    el("h3", {}, "Gameday plan"),
    el("p", { class: "muted plan-note" },
      "Kickoff times are in your local time. Picks are based only on pre-game info: betting line, expected points, records and rivalries."),
    plan.map(({ day, slots }) =>
      el("div", { class: "day" },
        el("h4", { class: "day-title" },
          day.toLocaleDateString("en-GB", { weekday: "long", day: "numeric", month: "long" })),
        SLOT_ORDER.filter((k) => slots[k]).map((k) => planSlot(day, k, slots[k])))));
}

function planSlot(day, key, games) {
  const meta = SLOTS[key];
  const times = [...new Set(games.map((g) => fmtTime(new Date(g.kickoff))))].sort().join(" / ");
  const head = el("div", { class: "slot-head" },
    el("span", { class: "slot-time" }, times),
    el("span", { class: "slot-title" }, meta.title),
    el("span", { class: "slot-mode" }, meta.mode));

  if (!meta.pick) {
    return el("div", { class: `slot slot-${key}` }, head,
      el("div", { class: "compact-list" }, games.map(compactRow)));
  }

  const top = games[0];
  const promising = top.favorite || top.hype.stars >= PROMISING;
  if (meta.promisingOnly && !promising) {
    return el("div", { class: `slot slot-${key}` }, head,
      el("p", { class: "slot-verdict" }, "Nothing promising this late. Get some sleep and check the ratings tomorrow."),
      el("div", { class: "compact-list" }, games.map(compactRow)));
  }

  const slotKey = `${day.getTime()}-${key}`;
  const expanded = state.expandedSlots.has(slotKey);
  const shown = expanded ? games : games.slice(0, 3);
  const more = games.length - shown.length;
  return el("div", { class: `slot slot-${key}` }, head,
    el("div", { class: "grid" }, shown.map((g, i) => card(g, i === 0 ? (g.favorite ? "Your team" : "Top pick") : null))),
    more > 0
      ? el("button", {
          class: "btn small more",
          onclick: () => { state.expandedSlots.add(slotKey); renderWeek(); },
        }, `Show ${more} more game${more > 1 ? "s" : ""} in this slot`)
      : null);
}

function compactRow(g) {
  const favIds = new Set(state.user.favorites);
  const name = (t) => el("span", { class: favIds.has(t.id) ? "fav-name" : "" }, t.short);
  return el("div", { class: "compact" },
    el("span", { class: "c-time" }, fmtTime(new Date(g.kickoff))),
    el("img", { src: g.away.logo, alt: "" }), name(g.away),
    el("span", { class: "at" }, "@"),
    el("img", { src: g.home.logo, alt: "" }), name(g.home),
    el("span", { class: "c-hype", style: `--tier:${TIER_COLORS[g.hype.stars]}` }, g.hype.tier));
}

function fmtKickoff(iso) {
  const d = new Date(iso);
  return d.toLocaleString("en-GB", { weekday: "short", day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
}

function teamBlock(t, side, favIds) {
  const fav = favIds.has(t.id);
  return el("div", { class: `team ${side}` },
    el("img", { src: t.logo, alt: "", loading: "lazy" }),
    el("div", { class: "names" },
      el("div", { class: "name", title: t.name }, fav ? el("span", { class: "star" }, "★ ") : null, t.short),
      el("div", { class: "rec", title: "Record before this game" }, t.record || t.abbr)));
}

function card(g, badge = null) {
  const favIds = new Set(state.user.favorites);
  const color = (t) => (t.color ? `#${t.color}` : "var(--border)");

  const node = el("article", {
    class: "card" + (g.favorite ? " fav" : "") + (g.watched ? " watched" : "") + (badge ? " pick" : ""),
    style: `--c-away:${color(g.away)};--c-home:${color(g.home)}`,
  },
    badge ? el("span", { class: "badge" }, badge) : null,
    el("div", { class: "meta" },
      el("span", { class: "when" }, fmtKickoff(g.kickoff)),
      el("span", {}, [g.broadcast, g.note].filter(Boolean).join(" · "))),
    el("div", { class: "matchup" },
      teamBlock(g.away, "away", favIds),
      el("div", { class: "at" }, "@"),
      teamBlock(g.home, "home", favIds)),
    ratingRow(g),
    chipsRow(g),
    actionsRow(g),
  );
  return node;
}

function ratingRow(g) {
  if (g.status !== "final") {
    const h = g.hype;
    return el("div", { class: "rating", style: `--tier:${TIER_COLORS[h.stars]}` },
      el("div", { class: "meter", title: `Pre-game hype: ${h.stars} of 5` },
        [1, 2, 3, 4, 5].map((i) => el("i", { class: i <= h.stars ? "on" : "" }))),
      el("span", { class: "tier" }, `Hype: ${h.tier}`),
      g.status === "live"
        ? el("span", { class: "score status live" }, "● Live")
        : el("span", { class: "score", title: "Pre-game hype score (0-100)" }, `${h.score}/100`));
  }
  if (isFavoriteRatingHidden(g)) {
    return el("div", { class: "rating hidden-rating" },
      "Rating hidden for your team",
      el("button", {
        class: "btn small", style: "margin-left:auto",
        onclick: () => { state.shownRatings.add(g.id); renderWeek(); },
      }, "Show rating"));
  }
  const r = g.rating;
  return el("div", { class: "rating", style: `--tier:${TIER_COLORS[r.stars]}` },
    el("div", { class: "meter", title: `${r.stars} of 5` },
      [1, 2, 3, 4, 5].map((i) => el("i", { class: i <= r.stars ? "on" : "" }))),
    el("span", { class: "tier" }, r.tier),
    el("span", { class: "score", title: "Watchability score (0-100)" }, `${r.score}/100`));
}

function chipsRow(g) {
  const chips = [];
  if (g.line) chips.push(el("span", { class: "chip", title: "Pre-game betting line" }, g.line));
  if (g.hype) {
    // Pre-game reasons are not spoilers, so they are always shown.
    for (const r of g.hype.reasons) {
      chips.push(el("span", { class: /Top matchup|Coin-flip|International/.test(r) ? "chip hot" : "chip" }, r));
    }
  } else if (g.topMatchup) {
    chips.push(el("span", { class: "chip hot" }, "Top matchup"));
  }
  const showReasons = g.rating && !isFavoriteRatingHidden(g)
    && (state.user.settings.showReasons || state.revealed.has(g.id));
  if (showReasons) {
    for (const r of g.rating.reasons) chips.push(el("span", { class: "chip reason" }, r));
  }
  return chips.length ? el("div", { class: "chips" }, chips) : null;
}

function actionsRow(g) {
  const buttons = [];
  const s = state.user.settings;
  if (g.rating && !s.showReasons && !state.revealed.has(g.id) && !isFavoriteRatingHidden(g)) {
    buttons.push(el("button", {
      class: "btn small", title: "Shows hints like 'Overtime' or 'Comeback' (mild spoilers)",
      onclick: () => { state.revealed.add(g.id); renderWeek(); },
    }, "Why this rating?"));
  }
  // Upcoming/live games can be marked too, for watching live.
  buttons.push(el("button", {
    class: "btn small" + (g.watched ? " on" : ""),
    onclick: () => toggleWatched(g),
  }, g.watched ? "✓ Watched" : "Mark watched"));
  if (g.watched && g.status === "final") {
    const res = state.results.get(g.id);
    if (res) {
      buttons.push(el("span", { class: "result" },
        `${res.awayAbbr} ${res.awayScore} : ${res.homeScore} ${res.homeAbbr}`));
    } else {
      buttons.push(el("button", {
        class: "btn small", onclick: () => revealResult(g),
      }, "Reveal result"));
    }
  }
  return buttons.length ? el("div", { class: "actions" }, buttons) : null;
}

async function toggleWatched(g) {
  try {
    state.user = await invoke("set_watched", { gameId: g.id, watched: !g.watched });
    g.watched = !g.watched;
    if (!g.watched) state.results.delete(g.id);
    renderWeek();
  } catch (e) {
    alert(e);
  }
}

async function revealResult(g) {
  try {
    state.results.set(g.id, await invoke("reveal_result", { gameId: g.id }));
    renderWeek();
  } catch (e) {
    alert(e);
  }
}

// ------------------------------------------------------------------ dialogs

async function openTeams() {
  const dlg = $("#dlg-teams");
  const grid = $("#team-grid");
  if (!state.teams.length) {
    grid.replaceChildren(el("div", { class: "muted" }, "Loading teams…"));
    dlg.showModal();
    try {
      state.teams = await invoke("get_teams");
    } catch (e) {
      grid.replaceChildren(el("div", { class: "error" }, String(e)));
      return;
    }
  }
  const selected = new Set(state.user.favorites);
  const draw = () => grid.replaceChildren(...state.teams.map((t) =>
    el("button", {
      type: "button",
      class: "team-pick" + (selected.has(t.id) ? " on" : ""),
      onclick: () => { selected.has(t.id) ? selected.delete(t.id) : selected.add(t.id); draw(); },
    }, el("img", { src: t.logo, alt: "" }), t.name)));
  draw();
  if (!dlg.open) dlg.showModal();
  dlg.onclose = async () => {
    try {
      state.user = await invoke("set_favorites", { teamIds: [...selected] });
      for (const g of state.games) g.favorite = selected.has(g.home.id) || selected.has(g.away.id);
      if (state.overview) renderWeek();
    } catch (e) {
      alert(e);
    }
  };
}

function openSettings() {
  const dlg = $("#dlg-settings");
  const s = state.user.settings;
  $("#set-reasons").checked = s.showReasons;
  $("#set-hidefav").checked = s.hideFavoriteRatings;
  $("#set-hidewatched").checked = s.hideWatched;
  $("#set-minstars").value = String(s.minStars);
  $("#set-earliest").value = s.earliestKickoff;
  $("#set-full").value = s.fullGameBefore;
  $("#set-half").value = s.firstHalfBefore;
  dlg.showModal();
  dlg.onclose = async () => {
    const settings = {
      showReasons: $("#set-reasons").checked,
      hideFavoriteRatings: $("#set-hidefav").checked,
      hideWatched: $("#set-hidewatched").checked,
      minStars: Number($("#set-minstars").value),
      earliestKickoff: $("#set-earliest").value || s.earliestKickoff,
      fullGameBefore: $("#set-full").value || s.fullGameBefore,
      firstHalfBefore: $("#set-half").value || s.firstHalfBefore,
    };
    try {
      state.user = await invoke("set_settings", { settings });
      if (state.overview && state.games.length) renderWeek();
    } catch (e) {
      alert(e);
    }
  };
}

init();
