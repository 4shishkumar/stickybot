//! GitHub watcher: changes on your pull requests become stick events (a port of `github.py`).
//!
//! The `gh` CLI (your own login, read-only) every `github.interval_s`: your PRs updated in the last
//! 14 days (opened, merged, approved / changes requested, CI failed / fixed, Devin reviews), PRs
//! waiting for your review (the primary repo's), the primary repo's releases. The first poll only
//! records the state (no events). State in `<home>/github_state.json` (the Python file's format).

use futures_util::future::BoxFuture;
use serde_json::{Map, Value, json};
use std::path::PathBuf;
use std::time::Duration;

pub const MAX_BACKOFF_S: u64 = 30 * 60;

pub const QUERY: &str = r#"
query($mine: String!, $asks: String!, $owner: String!, $name: String!, $withRepo: Boolean!) {
  viewer { login }
  mine: search(query: $mine, type: ISSUE, first: 40) {
    nodes { ... on PullRequest { number title url state merged createdAt repository { nameWithOwner } reviewDecision
      mergedBy { login }
      commits(last: 1) { nodes { commit { statusCheckRollup { state
        contexts(first: 30) { nodes { ... on CheckRun { name conclusion } ... on StatusContext { context state } } } } } } }
      reviews(last: 20) { nodes { id author { login } state } } } }
  }
  asks: search(query: $asks, type: ISSUE, first: 40) {
    nodes { ... on PullRequest { number title url author { login } repository { nameWithOwner }
      timelineItems(itemTypes: [REVIEW_REQUESTED_EVENT], last: 5) { nodes { ... on ReviewRequestedEvent { actor { login } } } } } }
  }
  primary: repository(owner: $owner, name: $name) @include(if: $withRepo) {
    releases(first: 1, orderBy: {field: CREATED_AT, direction: DESC}) { nodes { tagName name } }
  }
}
"#;

/// `github` settings with the defaults.
pub fn settings(cfg: &Map<String, Value>) -> Map<String, Value> {
    let mut g: Map<String, Value> =
        json!({"on": true, "interval_s": 120, "primary_repo": ""}).as_object().cloned().unwrap_or_default();
    if let Some(Value::Object(u)) = cfg.get("github") {
        g.extend(u.clone());
    }
    g
}

/// Runs the GraphQL query; the `data` object. Tests answer from memory.
pub trait Gh: Send + Sync {
    fn available(&self) -> bool;
    fn query(&self, primary: &str, since: &str) -> BoxFuture<'static, Result<Value, String>>;
}

/// `gh api graphql …`
pub struct GhCli;

impl GhCli {
    pub fn args(primary: &str, since: &str) -> Vec<String> {
        let (owner, name) = primary.split_once('/').unwrap_or((primary, ""));
        let with = !owner.is_empty() && !name.is_empty();
        vec![
            "api".into(),
            "graphql".into(),
            "-f".into(),
            format!("query={QUERY}"),
            "-F".into(),
            format!("mine=is:pr author:@me updated:>={since}"),
            "-F".into(),
            format!("asks=is:pr is:open review-requested:@me{}", if with { format!(" repo:{primary}") } else { String::new() }),
            "-F".into(),
            format!("owner={}", if owner.is_empty() { "x" } else { owner }),
            "-F".into(),
            format!("name={}", if name.is_empty() { "x" } else { name }),
            "-F".into(),
            format!("withRepo={with}"),
        ]
    }

    fn path() -> Option<PathBuf> {
        let dirs = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default();
        dirs.into_iter()
            .chain(["/opt/homebrew/bin".into(), "/usr/local/bin".into()])
            .map(|d| d.join("gh"))
            .find(|p| p.is_file())
    }
}

impl Gh for GhCli {
    fn available(&self) -> bool {
        Self::path().is_some()
    }
    fn query(&self, primary: &str, since: &str) -> BoxFuture<'static, Result<Value, String>> {
        let args = Self::args(primary, since);
        Box::pin(async move {
            let gh = Self::path().ok_or("gh CLI not found")?;
            let child = tokio::process::Command::new(gh)
                .args(&args)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true)
                .output();
            let out = tokio::time::timeout(Duration::from_secs(40), child).await.map_err(|_| "gh timed out")?.map_err(|e| e.to_string())?;
            if !out.status.success() {
                let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                let tail: String = err.chars().rev().take(200).collect::<Vec<_>>().into_iter().rev().collect();
                return Err(if tail.is_empty() { format!("gh exit {}", out.status) } else { tail });
            }
            let d: Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
            if let Some(e) = d.get("errors").filter(|e| !e.is_null()) {
                return Err(e.to_string().chars().take(200).collect());
            }
            Ok(d["data"].clone())
        })
    }
}

/// Answers from memory: `None` = no `gh` on this Mac (the fake stick's default; tests set data).
#[derive(Default, Clone)]
pub struct FakeGh {
    pub data: std::sync::Arc<std::sync::Mutex<Option<Result<Value, String>>>>,
}

impl FakeGh {
    pub fn set(&self, d: Result<Value, String>) {
        *self.data.lock().unwrap() = Some(d);
    }
}

impl Gh for FakeGh {
    fn available(&self) -> bool {
        self.data.lock().unwrap().is_some()
    }
    fn query(&self, _primary: &str, _since: &str) -> BoxFuture<'static, Result<Value, String>> {
        let d = self.data.lock().unwrap().clone().unwrap_or(Err("gh CLI not found".into()));
        Box::pin(async move { d })
    }
}

/// `'2026-10-20T10:12:00Z'` -> seconds since 1970 (0 if missing).
pub fn epoch(iso: Option<&str>) -> i64 {
    iso.and_then(|s| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%SZ").ok())
        .map(|t| t.and_utc().timestamp())
        .unwrap_or(0)
}

/// `'acme/web'` -> `'web'` for the primary repo; others in full.
pub fn short(repo: &str, primary: &str) -> String {
    if repo == primary { repo.rsplit('/').next().unwrap_or(repo).to_string() } else { repo.to_string() }
}

fn nodes(v: &Value) -> Vec<&Value> {
    v.get("nodes").and_then(Value::as_array).map(|a| a.iter().filter(|n| n.is_object()).collect()).unwrap_or_default()
}

/// The query's data -> `{"prs", "asks", "release", "release_name"}`.
pub fn snapshot(data: &Value) -> Value {
    let mut prs = Map::new();
    for n in nodes(&data["mine"]) {
        let Some(repo) = n["repository"]["nameWithOwner"].as_str() else { continue };
        let key = format!("{repo}#{}", n["number"]);
        let commit = &n["commits"]["nodes"][0]["commit"];
        let devin: Vec<Value> = nodes(&n["reviews"])
            .into_iter()
            .filter(|r| r["author"]["login"].as_str().is_some_and(|l| l.to_lowercase().contains("devin")))
            .map(|r| r["id"].clone())
            .collect();
        // who approved / asked for changes last, and the first failing check
        let reviewer = |state: &str| {
            nodes(&n["reviews"]).into_iter().rev().find(|r| r["state"] == state).map(|r| r["author"]["login"].clone())
        };
        // the first failing check that did the work, not a summary gate ("CI passed", "All checks")
        let gate = |name: &str| {
            let n = name.to_lowercase();
            ["passed", "required", "all checks", "all-checks", "gate", "summary"].iter().any(|g| n.contains(g))
        };
        let failed = nodes(&commit["statusCheckRollup"]["contexts"]).into_iter().filter(|c| {
            !gate(c["name"].as_str().or(c["context"].as_str()).unwrap_or(""))
        }).find_map(|c| {
            let bad = |v: &Value| matches!(v.as_str(), Some("FAILURE" | "ERROR" | "TIMED_OUT" | "STARTUP_FAILURE"));
            if bad(&c["conclusion"]) {
                c["name"].as_str().map(str::to_owned)
            } else if bad(&c["state"]) {
                c["context"].as_str().map(str::to_owned)
            } else {
                None
            }
        });
        prs.insert(
            key,
            json!({"repo": repo, "number": n["number"], "title": n["title"], "state": n["state"], "merged": n["merged"],
                   "decision": n.get("reviewDecision").cloned().unwrap_or(Value::Null),
                   "created": epoch(n["createdAt"].as_str()),
                   "ci": commit["statusCheckRollup"]["state"].clone(), "devin": devin,
                   "url": n["url"].clone(), "merged_by": n["mergedBy"]["login"].clone(),
                   "approved_by": reviewer("APPROVED"), "changes_by": reviewer("CHANGES_REQUESTED"),
                   "failed_check": failed}),
        );
    }
    let mut asks = Map::new();
    for n in nodes(&data["asks"]) {
        let Some(repo) = n["repository"]["nameWithOwner"].as_str() else { continue };
        // who asked: the last review request's actor, else the PR's author
        let by = nodes(&n["timelineItems"]).into_iter().rev().find_map(|e| e["actor"]["login"].as_str().map(str::to_owned));
        let by = by.or_else(|| n["author"]["login"].as_str().map(str::to_owned));
        asks.insert(
            format!("{repo}#{}", n["number"]),
            json!({"repo": repo, "number": n["number"], "title": n["title"], "url": n["url"].clone(), "by": by}),
        );
    }
    let rel = nodes(&data["primary"]["releases"]);
    json!({"prs": prs, "asks": asks, "viewer": data["viewer"]["login"].clone(),
           "release": rel.first().map(|r| r["tagName"].clone()).unwrap_or(Value::Null),
           "release_name": rel.first().map(|r| r["name"].clone()).unwrap_or(Value::Null)})
}

/// What a change means.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// a stick event: kind, title, sub, and for its detail page `{"actor", "url"}` (nulls when unknown)
    Event(&'static str, String, String, Value),
    PetMerged,
    PetGreen,
    PetRelease,
}

fn title60(pr: &Value) -> String {
    pr["title"].as_str().unwrap_or("").chars().take(60).collect()
}

/// A login for a title: none when unknown or it's you ("you" merged your own PR).
fn who<'a>(v: &'a Value, me: Option<&str>) -> Option<&'a str> {
    v.as_str().filter(|l| !l.is_empty() && Some(*l) != me)
}

/// Old state -> new snapshot: the events (and the pet's progress), primary repo first (`_diff`).
/// Titles say who did it when GitHub says ("alex approved #12"); the extra carries the actor and
/// the PR's link for the stick's detail page.
pub fn diff(old: &Value, new: &Value, primary: &str) -> Vec<Change> {
    let mut out = Vec::new();
    let me = new["viewer"].as_str();
    let ev = |out: &mut Vec<Change>, k: &'static str, t: String, s: String, actor: Option<&str>, url: &Value| {
        out.push(Change::Event(k, t, s, json!({"actor": actor, "url": url})))
    };
    let empty = Map::new();
    let new_prs = new["prs"].as_object().unwrap_or(&empty);
    let mut order: Vec<(&String, &Value)> = new_prs.iter().collect();
    order.sort_by(|a, b| (a.1["repo"] != primary, a.0).cmp(&(b.1["repo"] != primary, b.0)));
    for (key, pr) in order {
        let num = &pr["number"];
        let where_ = format!("{} #{num}", short(pr["repo"].as_str().unwrap_or(""), primary));
        let title = title60(pr);
        let sub = format!("{where_} - {title}");
        let url = &pr["url"];
        let Some(was) = old["prs"].get(key.as_str()) else {
            let seen = old.get("seen_at").and_then(Value::as_f64).unwrap_or(0.0);
            if pr["state"] == "OPEN" && pr["created"].as_f64().unwrap_or(0.0) >= seen - 300.0 {
                ev(&mut out, "pr_created", format!("PR #{num} opened"), sub.clone(), None, url);
            }
            continue;
        };
        if pr["merged"] == true && was["merged"] != true {
            out.push(Change::PetMerged);
            let by = who(&pr["merged_by"], me);
            let t = match by {
                Some(l) => format!("{l} merged #{num}!"),
                None => format!("PR #{num} merged!"),
            };
            ev(&mut out, "pr_merged", t, sub.clone(), by, url);
        }
        if pr["decision"] != was["decision"] {
            match pr["decision"].as_str() {
                Some("APPROVED") => {
                    let by = who(&pr["approved_by"], me);
                    let t = by.map_or(format!("PR #{num} approved"), |l| format!("{l} approved #{num}"));
                    ev(&mut out, "pr_approved", t, sub.clone(), by, url)
                }
                Some("CHANGES_REQUESTED") => {
                    let by = who(&pr["changes_by"], me);
                    let t = by.map_or("Changes requested".into(), |l| format!("{l} wants changes on #{num}"));
                    ev(&mut out, "changes_requested", t, sub.clone(), by, url)
                }
                _ => {}
            }
        }
        if pr["ci"] != was["ci"] {
            if pr["ci"] == "SUCCESS" {
                out.push(Change::PetGreen);
            }
            let bad = |v: &Value| matches!(v.as_str(), Some("FAILURE" | "ERROR"));
            if bad(&pr["ci"]) {
                let t = match pr["failed_check"].as_str() {
                    Some(check) => format!("CI failed on #{num}: {check}"),
                    None => format!("CI failed on #{num}"),
                };
                ev(&mut out, "ci_failed", t, sub.clone(), None, url);
            } else if pr["ci"] == "SUCCESS" && bad(&was["ci"]) {
                ev(&mut out, "ci_fixed", format!("CI is green again on #{num}"), sub.clone(), None, url);
            }
        }
        let had: Vec<&Value> = was["devin"].as_array().map(|a| a.iter().collect()).unwrap_or_default();
        if pr["devin"].as_array().into_iter().flatten().any(|r| !had.contains(&r)) {
            ev(&mut out, "devin_review", format!("Devin reviewed #{num}"), sub.clone(), Some("Devin"), url);
        }
    }
    let new_asks = new["asks"].as_object().unwrap_or(&empty);
    let mut asks: Vec<(&String, &Value)> = new_asks.iter().collect();
    asks.sort_by(|a, b| (a.1["repo"] != primary, a.0).cmp(&(b.1["repo"] != primary, b.0)));
    for (key, a) in asks {
        if old["asks"].get(key.as_str()).is_none() {
            let by = who(&a["by"], me);
            let t = by.map_or("Review requested".into(), |l| format!("{l} asked for your review"));
            let sub = format!("{} #{} - {}", short(a["repo"].as_str().unwrap_or(""), primary), a["number"], title60(a));
            ev(&mut out, "review_requested", t, sub, by, &a["url"]);
        }
    }
    if let (Some(r), Some(was)) = (new["release"].as_str(), old["release"].as_str())
        && r != was
    {
        out.push(Change::PetRelease);
        let t: String = format!("Release {r}").chars().take(40).collect();
        ev(&mut out, "release", t, short(primary, primary), None, &Value::Null);
    }
    out
}

/// One poll's result: the new state to keep, and what changed (`None` on the first look).
pub fn merge(old: Option<&Value>, mut new: Value, primary: &str, now: f64) -> (Value, Option<Vec<Change>>) {
    let changes = match old {
        Some(old) if old.get("primary").and_then(Value::as_str) == Some(primary) => {
            let ch = diff(old, &new, primary);
            // PRs that dropped out of the 14-day window keep their last state
            if let (Some(np), Some(op)) = (new["prs"].as_object_mut(), old["prs"].as_object()) {
                for (k, v) in op {
                    np.entry(k.clone()).or_insert_with(|| v.clone());
                }
                if np.len() > 500 {
                    let mut all: Vec<(String, Value)> = std::mem::take(np).into_iter().collect();
                    all.sort_by_key(|(_, v)| -v["number"].as_i64().unwrap_or(0));
                    all.truncate(500);
                    *np = all.into_iter().collect();
                }
            }
            Some(ch)
        }
        _ => {
            log::info!(
                "github: tracking {} PRs, {} review requests (first look, no events)",
                new["prs"].as_object().map_or(0, Map::len),
                new["asks"].as_object().map_or(0, Map::len)
            );
            None
        }
    };
    new["primary"] = json!(primary);
    new["seen_at"] = json!(now);
    (new, changes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_say_who_and_carry_the_link() {
        let p = "acme/web";
        let pr = |merged: bool, decision: &str, ci: &str| {
            json!({"number": 12, "title": "Fix login", "url": "https://github.com/acme/web/pull/12", "state": "OPEN",
                   "merged": merged, "reviewDecision": decision, "createdAt": "2026-11-01T10:00:00Z",
                   "repository": {"nameWithOwner": p}, "mergedBy": {"login": "sam"},
                   "commits": {"nodes": [{"commit": {"statusCheckRollup": {"state": ci, "contexts": {"nodes": [
                       {"name": "build", "conclusion": "SUCCESS"}, {"name": "CI passed", "conclusion": "FAILURE"},
                       {"name": "lint", "conclusion": "FAILURE"}]}}}}]},
                   "reviews": {"nodes": [{"id": "r1", "author": {"login": "alex"}, "state": "APPROVED"},
                                         {"id": "r2", "author": {"login": "kim"}, "state": "CHANGES_REQUESTED"}]}})
        };
        let ask = json!({"number": 9, "title": "Update deps", "url": "https://github.com/acme/web/pull/9",
                         "author": {"login": "bot"}, "repository": {"nameWithOwner": p},
                         "timelineItems": {"nodes": [{"actor": {"login": "priya"}}]}});
        let data = |prs: Vec<Value>, asks: Vec<Value>| json!({"viewer": {"login": "me"}, "mine": {"nodes": prs}, "asks": {"nodes": asks}});
        let (s1, _) = merge(None, snapshot(&data(vec![pr(false, "REVIEW_REQUIRED", "SUCCESS")], vec![])), p, 0.0);
        let changes = |s: &Value, d: Value| {
            let (_, ch) = merge(Some(s), snapshot(&d), p, 1.0);
            ch.unwrap()
                .into_iter()
                .filter_map(|c| match c {
                    Change::Event(k, t, _, x) => Some((k, t, x["actor"].clone(), x["url"].clone())),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let url = json!("https://github.com/acme/web/pull/12");
        assert_eq!(
            changes(&s1, data(vec![pr(false, "APPROVED", "FAILURE")], vec![ask.clone()])),
            vec![
                ("pr_approved", "alex approved #12".into(), json!("alex"), url.clone()),
                ("ci_failed", "CI failed on #12: lint".into(), Value::Null, url.clone()),
                ("review_requested", "priya asked for your review".into(), json!("priya"), json!("https://github.com/acme/web/pull/9")),
            ]
        );
        assert_eq!(changes(&s1, data(vec![pr(false, "CHANGES_REQUESTED", "SUCCESS")], vec![]))[0].1, "kim wants changes on #12");
        assert_eq!(changes(&s1, data(vec![pr(true, "REVIEW_REQUIRED", "SUCCESS")], vec![]))[0].1, "sam merged #12!");
        // your own merge doesn't name you
        let mut mine = pr(true, "REVIEW_REQUIRED", "SUCCESS");
        mine["mergedBy"]["login"] = json!("me");
        assert_eq!(changes(&s1, data(vec![mine], vec![]))[0].1, "PR #12 merged!");
    }

    pub fn pr(repo: &str, n: i64, state: &str, merged: bool, decision: Value, ci: Value, devin: &[&str]) -> Value {
        json!({"number": n, "title": format!("PR {n} title"), "state": state, "merged": merged,
               "createdAt": "2026-11-09T10:00:00Z", "repository": {"nameWithOwner": repo}, "reviewDecision": decision,
               "commits": {"nodes": [{"commit": {"statusCheckRollup": if ci.is_null() { Value::Null } else { json!({"state": ci}) }}}]},
               "reviews": {"nodes": devin.iter().map(|id| json!({"id": id, "author": {"login": "devin-ai-integration"}, "state": "COMMENTED"})).collect::<Vec<_>>()}})
    }

    fn data(mine: Vec<Value>, asks: Vec<Value>, release: Option<&str>) -> Value {
        json!({"mine": {"nodes": mine}, "asks": {"nodes": asks},
               "primary": {"releases": {"nodes": release.map(|r| vec![json!({"tagName": r, "name": r})]).unwrap_or_default()}}})
    }

    #[test]
    fn args_like_python() {
        let a = GhCli::args("acme/web", "2026-10-26");
        assert_eq!(a[5], "mine=is:pr author:@me updated:>=2026-10-26");
        assert_eq!(a[7], "asks=is:pr is:open review-requested:@me repo:acme/web");
        assert_eq!(&a[8..], ["-F", "owner=acme", "-F", "name=web", "-F", "withRepo=true"]);
        let a = GhCli::args("", "2026-10-26");
        assert_eq!(a[7], "asks=is:pr is:open review-requested:@me");
        assert_eq!(&a[8..], ["-F", "owner=x", "-F", "name=x", "-F", "withRepo=false"]);
    }

    #[test]
    fn epoch_and_short() {
        assert_eq!(epoch(Some("1970-01-02T00:00:00Z")), 86400);
        assert_eq!(epoch(None), 0);
        assert_eq!(short("acme/web", "acme/web"), "web");
        assert_eq!(short("acme/api", "acme/web"), "acme/api");
    }

    #[test]
    fn first_look_records_then_changes_become_events() {
        let p = "acme/web";
        let d1 = data(
            vec![
                pr(p, 1, "OPEN", false, Value::Null, json!("PENDING"), &[]),
                pr("other/x", 2, "OPEN", false, Value::Null, json!("FAILURE"), &[]),
                pr(p, 3, "OPEN", false, Value::Null, json!("SUCCESS"), &[]),
            ],
            vec![],
            Some("v1.0"),
        );
        let (s1, ch) = merge(None, snapshot(&d1), p, 1000.0);
        assert!(ch.is_none());
        assert_eq!(s1["prs"]["acme/web#1"]["ci"], "PENDING");
        assert_eq!(s1["seen_at"], 1000.0);

        let d2 = data(
            vec![
                pr(p, 1, "MERGED", true, json!("APPROVED"), json!("SUCCESS"), &["r1"]),
                pr("other/x", 2, "OPEN", false, json!("CHANGES_REQUESTED"), json!("SUCCESS"), &[]),
                pr(p, 3, "OPEN", false, Value::Null, json!("FAILURE"), &[]),
                pr(p, 4, "OPEN", false, Value::Null, Value::Null, &[]),
            ],
            vec![json!({"number": 9, "title": "Update deps", "repository": {"nameWithOwner": p}})],
            Some("v1.1"),
        );
        // #4 was created "now" (epoch of createdAt) -> make seen_at older than it
        let mut s1 = s1;
        s1["seen_at"] = json!(epoch(Some("2026-11-09T10:00:00Z")) as f64);
        let (s2, ch) = merge(Some(&s1), snapshot(&d2), p, 2000.0);
        let ch = ch.unwrap();
        let evs: Vec<(&str, String, String)> = ch
            .iter()
            .filter_map(|c| match c {
                Change::Event(k, t, s, _) => Some((*k, t.clone(), s.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            evs,
            vec![
                ("pr_merged", "PR #1 merged!".into(), "web #1 - PR 1 title".into()),
                ("pr_approved", "PR #1 approved".into(), "web #1 - PR 1 title".into()),
                ("devin_review", "Devin reviewed #1".into(), "web #1 - PR 1 title".into()),
                ("ci_failed", "CI failed on #3".into(), "web #3 - PR 3 title".into()),
                ("pr_created", "PR #4 opened".into(), "web #4 - PR 4 title".into()),
                ("changes_requested", "Changes requested".into(), "other/x #2 - PR 2 title".into()),
                ("ci_fixed", "CI is green again on #2".into(), "other/x #2 - PR 2 title".into()),
                ("review_requested", "Review requested".into(), "web #9 - Update deps".into()),
                ("release", "Release v1.1".into(), "web".into()),
            ]
        );
        assert_eq!(ch.iter().filter(|c| **c == Change::PetMerged).count(), 1);
        assert_eq!(ch.iter().filter(|c| **c == Change::PetGreen).count(), 2);
        assert!(ch.contains(&Change::PetRelease));

        // a PR that drops out of the window is kept; nothing new -> no events
        let d3 = data(vec![pr(p, 3, "OPEN", false, Value::Null, json!("FAILURE"), &[])], vec![json!({"number": 9, "title": "Update deps", "repository": {"nameWithOwner": p}})], Some("v1.1"));
        let (s3, ch) = merge(Some(&s2), snapshot(&d3), p, 3000.0);
        assert_eq!(ch.unwrap(), vec![]);
        assert_eq!(s3["prs"].as_object().unwrap().len(), 4);

        // another primary repo: a first look again
        let (_, ch) = merge(Some(&s3), snapshot(&d3), "acme/api", 4000.0);
        assert!(ch.is_none());
    }
}
