//! StickyBot's pet progress and the "world" the stick's faces draw (StickyWorld, Aquarium) — a
//! port of `pet.py`. The app keeps the progress (it sees every merged PR, green build, finished
//! Claude task, release), in `<home>/pet.json` (the Python file's format, so it can be imported).
//!
//! Rules (the stick has the same ones, m5-stick-s3/src/logic/pet.cpp):
//! - XP: merged PR +10, CI green +3, a Claude task finished +2, a release +25
//! - Level: 1 at 0 XP; level n -> n+1 needs 10 + 10 n XP
//! - Streak: days in a row with a merged PR or a finished Claude task
//! - Unlocks: hat at a 5-day streak (best ever), sunglasses at 10 green builds, crown at 100 PRs
//! - City: a building per merged PR (from `seed`), a tower per release (`towers`)

use chrono::Datelike;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub const XP_MERGE: i64 = 10;
pub const XP_GREEN: i64 = 3;
pub const XP_CLAUDE: i64 = 2;
pub const XP_RELEASE: i64 = 25;
/// Games on the stick (firmware 2.5.0): a game with points +1, a new best +3, at most
/// [`GAME_XP_DAY_CAP`] a day (one merged PR's worth); they don't count for the streak.
pub const XP_GAME: i64 = 1;
pub const XP_GAME_BEST: i64 = 3;
pub const GAME_XP_DAY_CAP: i64 = 10;
pub const GAMES: &[(&str, &str)] = &[
    ("stack", "Stack"),
    ("flappy", "Flappy"),
    ("dino", "Dino"),
    ("reaction", "Reaction"),
    ("maze", "Maze"),
    ("balance", "Balance"),
    ("catch", "Catch"),
];
pub const MAX_SCORE: i64 = 10_000_000;
pub const HAT: i64 = 1;
pub const SHADES: i64 = 2;
pub const CROWN: i64 = 4;
pub const MAX_TOWERS: usize = 8;
pub const MAX_SESSIONS: usize = 6;
/// A prompt with no Stop after this long: the session is gone, not working.
pub const RUNNING_MAX_S: f64 = 2.0 * 3600.0;

pub fn xp_to_next(level: i64) -> i64 {
    10 + 10 * level
}

/// `(level, xp into it, xp it needs)`
pub fn level_for(xp: i64) -> (i64, i64, i64) {
    let (mut xp, mut level) = (xp.max(0), 1);
    while level < 999 && xp >= xp_to_next(level) {
        xp -= xp_to_next(level);
        level += 1;
    }
    (level, xp, xp_to_next(level))
}

pub fn accessories(best_streak: i64, greens: i64, merged: i64) -> i64 {
    (if best_streak >= 5 { HAT } else { 0 }) | (if greens >= 10 { SHADES } else { 0 }) | (if merged >= 100 { CROWN } else { 0 })
}

/// Python's `date.today().toordinal()`.
pub fn today() -> i64 {
    chrono::Local::now().date_naive().num_days_from_ce() as i64
}

fn n(v: &Value, k: &str) -> i64 {
    v.get(k).and_then(Value::as_i64).unwrap_or(0)
}

pub struct Pet {
    path: PathBuf,
    pub s: Value,
}

impl Pet {
    /// Loads `pet.json`; a new pet starts with the PRs the GitHub watcher already saw merged.
    pub fn load(path: &Path, initial_merged: i64) -> Self {
        let s = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).filter(Value::is_object);
        let mut p = Pet { path: path.to_path_buf(), s: Value::Null };
        match s {
            Some(s) => p.s = s,
            None => {
                let mut b = [0u8; 4];
                getrandom::fill(&mut b).expect("system randomness");
                let seed = (u32::from_le_bytes(b) % (i32::MAX as u32 - 1)) as i64 + 1;
                p.s = json!({"xp": initial_merged * XP_MERGE, "merged": initial_merged, "greens": 0, "claude": 0,
                             "releases": 0, "towers": [], "seed": seed,
                             "streak": {"days": 0, "best": 0, "last": -1}, "today": {"day": today(), "merged": 0}});
                p.save();
            }
        }
        p
    }

    pub fn save(&self) {
        let tmp = self.path.with_extension("tmp");
        if let Err(e) = std::fs::write(&tmp, self.s.to_string()).and_then(|_| std::fs::rename(&tmp, &self.path)) {
            log::warn!("pet: can't save ({e})");
        }
    }

    fn add(&mut self, k: &str, by: i64) {
        self.s[k] = json!(n(&self.s, k) + by);
    }

    fn active(&mut self, day: i64) {
        let st = &mut self.s["streak"];
        let last = st.get("last").and_then(Value::as_i64).unwrap_or(-1);
        if last != day {
            let days = if last >= 0 && day - last == 1 { n(st, "days") + 1 } else { 1 };
            st["days"] = json!(days);
            st["last"] = json!(day);
            st["best"] = json!(n(st, "best").max(days));
        }
    }

    fn today_mut(&mut self, day: i64) -> &mut Value {
        if self.s["today"].get("day").and_then(Value::as_i64) != Some(day) {
            self.s["today"] = json!({"day": day, "merged": 0});
        }
        &mut self.s["today"]
    }

    pub fn merged_on(&mut self, day: i64) {
        self.add("merged", 1);
        self.add("xp", XP_MERGE);
        let t = self.today_mut(day);
        t["merged"] = json!(n(t, "merged") + 1);
        self.active(day);
        self.save();
    }
    pub fn merged(&mut self) {
        self.merged_on(today())
    }

    pub fn green(&mut self) {
        self.add("greens", 1);
        self.add("xp", XP_GREEN);
        self.save();
    }

    pub fn claude_done_on(&mut self, day: i64) {
        self.add("claude", 1);
        self.add("xp", XP_CLAUDE);
        self.active(day);
        self.save();
    }
    pub fn claude_done(&mut self) {
        self.claude_done_on(today())
    }

    pub fn release(&mut self) {
        self.add("releases", 1);
        self.add("xp", XP_RELEASE);
        let merged = n(&self.s, "merged");
        let mut towers: Vec<Value> = self.s["towers"].as_array().cloned().unwrap_or_default();
        towers.push(json!(merged));
        let skip = towers.len().saturating_sub(MAX_TOWERS);
        self.s["towers"] = Value::Array(towers.split_off(skip));
        self.save();
    }

    /// A game finished on the stick (`{"evt":"game"}`, pet.py `game`). Returns the XP it earned
    /// (0..3): +1 with points, +3 for a new best, at most [`GAME_XP_DAY_CAP`] a day. Junk is
    /// ignored (0). `score` / `best` are the message's raw values (only integers count).
    pub fn game_on(&mut self, gid: &str, score: &Value, best: &Value, new_best: bool, day: i64) -> i64 {
        let int = |v: &Value| if v.is_i64() || v.is_u64() { v.as_i64() } else { None };
        let Some(score) = int(score).filter(|s| (0..=MAX_SCORE).contains(s)) else { return 0 };
        if !is_game(gid) {
            return 0;
        }
        if !self.s.get("games").is_some_and(Value::is_object) {
            self.s["games"] = json!({});
        }
        let g = &mut self.s["games"];
        if g.get("day").and_then(Value::as_i64) != Some(day) {
            g["day"] = json!(day);
            g["xp"] = json!(0);
        }
        for k in ["plays", "best"] {
            if !g.get(k).is_some_and(Value::is_object) {
                g[k] = json!({});
            }
        }
        g["plays"][gid] = json!(n(&g["plays"], gid) + 1);
        if let Some(b) = int(best).filter(|b| (1..=MAX_SCORE).contains(b)) {
            g["best"][gid] = json!(b); // the stick keeps the bests (NVS); this is the last it told us
        }
        let had = n(g, "xp");
        let xp = if score > 0 { (if new_best { XP_GAME_BEST } else { XP_GAME }).min((GAME_XP_DAY_CAP - had).max(0)) } else { 0 };
        g["xp"] = json!(had + xp);
        self.add("xp", xp);
        self.save();
        xp
    }
    pub fn game(&mut self, gid: &str, score: &Value, best: &Value, new_best: bool) -> i64 {
        self.game_on(gid, score, best, new_best, today())
    }

    /// For the dashboard: `{"xpToday", "cap", "best": {id: n}, "plays": {id: n}}`.
    pub fn games_on(&self, day: i64) -> Value {
        let g = self.s.get("games").cloned().unwrap_or(json!({}));
        let xp = if g.get("day").and_then(Value::as_i64) == Some(day) { n(&g, "xp") } else { 0 };
        let obj = |k: &str| g.get(k).filter(|v| v.is_object()).cloned().unwrap_or(json!({}));
        json!({"xpToday": xp, "cap": GAME_XP_DAY_CAP, "best": obj("best"), "plays": obj("plays")})
    }

    pub fn streak_on(&self, day: i64) -> i64 {
        let st = &self.s["streak"];
        let last = st.get("last").and_then(Value::as_i64).unwrap_or(-1);
        if last >= 0 && day - last <= 1 { n(st, "days") } else { 0 }
    }

    pub fn merged_today(&mut self) -> i64 {
        n(self.today_mut(today()), "merged")
    }

    fn acc(&self) -> i64 {
        accessories(n(&self.s["streak"], "best"), n(&self.s, "greens"), n(&self.s, "merged"))
    }

    /// `GET /api/pet`'s `pet`.
    pub fn summary(&mut self) -> Value {
        let xp = n(&self.s, "xp");
        let (level, into, need) = level_for(xp);
        json!({"level": level, "xp": xp, "into": into, "need": need, "streak": self.streak_on(today()),
               "best": n(&self.s["streak"], "best"), "merged": n(&self.s, "merged"), "mergedToday": self.merged_today(),
               "greens": n(&self.s, "greens"), "claude": n(&self.s, "claude"), "releases": n(&self.s, "releases"),
               "acc": self.acc(), "games": self.games_on(today())})
    }

    /// The `pet` part of the world.
    pub fn pet_json(&self) -> Value {
        let towers: Vec<Value> = self.s["towers"].as_array().cloned().unwrap_or_default();
        let skip = towers.len().saturating_sub(MAX_TOWERS);
        json!({"xp": n(&self.s, "xp"), "streak": self.streak_on(today()), "acc": self.acc(),
               "seed": n(&self.s, "seed"), "greens": n(&self.s, "greens"), "towers": &towers[skip..]})
    }
}

pub fn is_game(id: &str) -> bool {
    GAMES.iter().any(|(g, _)| *g == id)
}

/// zlib's CRC-32 (Python `zlib.crc32`), for the sessions' small ids.
pub fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
        }
    }
    !c
}

/// Claude Code sessions working right now, from the hooks' file (`claude_sessions.json`, written
/// by `claude_hooks.py`): a prompt sent and its Stop not come yet. -> `[{"p", "s", "id"}]`
pub fn running_sessions(file: &Path, now: f64) -> Vec<Value> {
    let Some(Value::Object(d)) = std::fs::read_to_string(file).ok().and_then(|t| serde_json::from_str(&t).ok()) else {
        return Vec::new();
    };
    let mut out: Vec<(i64, Value)> = Vec::new();
    for (sid, v) in &d {
        let age = now - v.get("t").and_then(Value::as_f64).unwrap_or(0.0);
        if (0.0..RUNNING_MAX_S).contains(&age) {
            let cwd = v.get("cwd").and_then(Value::as_str).unwrap_or("").trim_end_matches('/');
            let base = cwd.rsplit('/').next().unwrap_or("");
            let project: String = (if base.is_empty() { "claude" } else { base }).chars().take(12).collect();
            let id = (crc32(sid.as_bytes()) & 0xFFFF).max(1);
            out.push((age as i64, json!({"p": project, "s": age as i64, "id": id})));
        }
    }
    out.sort_by_key(|(s, _)| -s);
    out.into_iter().take(MAX_SESSIONS).map(|(_, v)| v).collect()
}

/// `green | red | none` over your open PRs (red wins).
pub fn ci_state(gh_state: Option<&Value>) -> &'static str {
    let prs: Vec<&Value> = gh_state
        .and_then(|g| g.get("prs"))
        .and_then(Value::as_object)
        .map(|m| m.values().filter(|p| p["state"] == "OPEN").collect())
        .unwrap_or_default();
    if prs.iter().any(|p| matches!(p["ci"].as_str(), Some("FAILURE" | "ERROR"))) {
        "red"
    } else if prs.iter().any(|p| p["ci"] == "SUCCESS") {
        "green"
    } else {
        "none"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("stickybot-pet-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn levels_and_unlocks_like_pet_cpp() {
        assert_eq!(level_for(0), (1, 0, 20));
        assert_eq!(level_for(19), (1, 19, 20));
        assert_eq!(level_for(20), (2, 0, 30));
        assert_eq!(level_for(55), (3, 5, 40));
        assert_eq!(level_for(-5), (1, 0, 20));
        assert_eq!(accessories(5, 0, 0), HAT);
        assert_eq!(accessories(4, 10, 100), SHADES | CROWN);
    }

    #[test]
    fn progress_streaks_towers_and_the_file() {
        let d = tmp("progress");
        let f = d.join("pet.json");
        let mut p = Pet::load(&f, 3);
        assert_eq!(p.s["xp"], 30);
        assert_eq!(p.s["merged"], 3);
        let seed = p.s["seed"].as_i64().unwrap();
        assert!((1..2i64.pow(31)).contains(&seed));
        p.merged_on(1000);
        p.claude_done_on(1001);
        p.merged_on(1002);
        assert_eq!(p.s["streak"], json!({"days": 3, "best": 3, "last": 1002}));
        assert_eq!(p.streak_on(1003), 3);
        assert_eq!(p.streak_on(1004), 0);
        p.merged_on(1010);
        assert_eq!(p.s["streak"]["days"], 1);
        assert_eq!(p.s["streak"]["best"], 3);
        p.green();
        for _ in 0..9 {
            p.release();
        }
        assert_eq!(p.s["towers"].as_array().unwrap().len(), MAX_TOWERS);
        assert_eq!(p.s["xp"], json!(30 + 30 + 2 + 3 + 9 * 25));
        // saved and read back as is (the Python format)
        let back = Pet::load(&f, 0);
        assert_eq!(back.s, p.s);
        let pj = back.pet_json();
        assert_eq!(pj["seed"], seed);
        assert_eq!(pj["towers"].as_array().unwrap().len(), 8);
        assert_eq!(pj["acc"], 0);
    }

    // --- test_pet.py: games ---------------------------------------------------------------------

    #[test]
    fn games_earn_a_little_xp_capped_a_day() {
        let d = tmp("games");
        let f = d.join("pet.json");
        let mut p = Pet::load(&f, 0);
        let day = 2000;
        let xp0 = n(&p.s, "xp");
        let g = |p: &mut Pet, id: &str, s: i64, b: i64, nb: bool, day: i64| p.game_on(id, &json!(s), &json!(b), nb, day);
        assert_eq!(g(&mut p, "stack", 12, 12, true, day), XP_GAME_BEST);
        assert_eq!(g(&mut p, "stack", 5, 12, false, day), XP_GAME);
        assert_eq!(g(&mut p, "reaction", 0, 231, false, day), 0); // a false start: nothing
        for _ in 0..20 {
            g(&mut p, "flappy", 3, 9, false, day); // farming: stops at the cap
        }
        assert_eq!(p.games_on(day)["xpToday"], GAME_XP_DAY_CAP);
        assert_eq!(n(&p.s, "xp"), xp0 + GAME_XP_DAY_CAP);
        assert_eq!(g(&mut p, "dino", 900, 900, true, day), 0);
        assert_eq!(g(&mut p, "dino", 900, 900, true, day + 1), XP_GAME_BEST); // a new day
        let gs = p.games_on(day + 1);
        assert_eq!((gs["best"]["stack"].as_i64(), gs["best"]["reaction"].as_i64()), (Some(12), Some(231)));
        assert_eq!(gs["plays"]["flappy"], 20);
        assert_eq!((gs["xpToday"].as_i64(), gs["cap"].as_i64()), (Some(XP_GAME_BEST), Some(GAME_XP_DAY_CAP)));
        assert_eq!(Pet::load(&f, 0).s["games"]["best"]["dino"], 900); // saved
        assert_eq!(p.streak_on(day + 1), 0); // games don't make a streak
    }

    #[test]
    fn game_events_are_checked() {
        let d = tmp("games-bad");
        let mut p = Pet::load(&d.join("pet.json"), 0);
        let xp0 = n(&p.s, "xp");
        for (id, s) in [("tetris", json!(5)), ("stack", json!(-1)), ("stack", json!("9")), ("stack", json!(true)),
                        ("stack", json!(1_000_000_000i64)), ("", json!(3)), ("stack", json!(2.5))] {
            assert_eq!(p.game(id, &s, &Value::Null, false), 0, "{id} {s}");
        }
        assert_eq!(n(&p.s, "xp"), xp0);
        assert_eq!(p.game("maze", &json!(3), &json!("x"), false), XP_GAME); // odd extras: a plain game
        assert!(p.summary()["games"]["best"].get("maze").is_none());
    }

    #[test]
    fn crc_like_zlib_and_sessions() {
        assert_eq!(crc32(b"hello"), 0x3610_a686);
        let d = tmp("sessions");
        let f = d.join("claude_sessions.json");
        assert!(running_sessions(&f, 0.0).is_empty());
        std::fs::write(
            &f,
            json!({"a": {"t": 1000.0, "cwd": "/Users/x/code/very-long-project-name/"},
                   "b": {"t": 1900.0, "cwd": ""},
                   "old": {"t": -9000.0, "cwd": "/x"}})
            .to_string(),
        )
        .unwrap();
        let s = running_sessions(&f, 2000.0);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0]["p"], "very-long-pr");
        assert_eq!(s[0]["s"], 1000);
        assert_eq!(s[0]["id"], (crc32(b"a") & 0xFFFF).max(1));
        assert_eq!(s[1]["p"], "claude");
    }

    #[test]
    fn ci_over_open_prs() {
        let g = |prs: Value| json!({"prs": prs});
        assert_eq!(ci_state(None), "none");
        assert_eq!(ci_state(Some(&g(json!({"a": {"state": "OPEN", "ci": "SUCCESS"}})))), "green");
        assert_eq!(
            ci_state(Some(&g(json!({"a": {"state": "OPEN", "ci": "SUCCESS"}, "b": {"state": "OPEN", "ci": "ERROR"}})))),
            "red"
        );
        assert_eq!(ci_state(Some(&g(json!({"a": {"state": "MERGED", "ci": "FAILURE"}})))), "none");
    }
}
