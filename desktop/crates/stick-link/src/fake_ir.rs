//! The fake stick's 2.5.0 parts: the IR remote (`@ir …` and `{"ir":{…}}`, like `src/ir/ir.cpp` and
//! test_ir.py's `IRStick`), the games (`@games`) and the remote modes (`@air`, `@mode`).

use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

/// The stick's learned remotes and what it sent.
#[derive(Debug, Default)]
pub struct FakeIr {
    /// device id -> buttons `{"n", "c", "f"?}`
    pub devices: BTreeMap<String, Vec<Value>>,
    /// what went out: `(dev, btn, repeat)`, or `("code", <code>, repeat)`
    pub sent: Vec<(String, String, i64)>,
    /// a learn running: `(dev, btn)`
    pub learning: Option<(String, String)>,
    /// firmware before 2.5.0: `@ir` is an unknown command and `@state` has no `ir`
    pub unsupported: bool,
}

impl FakeIr {
    pub fn add(&mut self, dev: &str, btn: &str, code: &str, fav: bool) {
        let mut b = json!({"n": btn, "c": code});
        if fav {
            b["f"] = json!(1);
        }
        self.devices.entry(dev.into()).or_default().push(b);
    }
}

fn nf() -> Value {
    json!({"ok": false, "error": "No such remote or button", "code": "notfound"})
}

/// `@ir op=…&…` -> the reply; `emit` gets the learning events.
pub fn run(ir: &mut FakeIr, args: &[(String, String)], emit: &mut dyn FnMut(Value)) -> Value {
    if ir.unsupported {
        return json!({"ok": false, "error": "Unknown command: ir"});
    }
    let get = |k: &str| args.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap_or_default();
    let num = |k: &str| get(k).parse::<i64>().unwrap_or(0);
    let (op, dev, btn) = (get("op"), get("dev"), get("btn"));
    let find = |ir: &mut FakeIr| -> Option<usize> { ir.devices.get(&dev)?.iter().position(|r| r["n"] == btn.as_str()) };
    match op.as_str() {
        "list" if dev.is_empty() => {
            let devices: Vec<Value> = ir
                .devices
                .iter()
                .map(|(d, r)| {
                    let fav: i64 = r.iter().map(|x| x["f"].as_i64().unwrap_or(0)).sum();
                    json!({"id": d, "n": r.len(), "fav": fav})
                })
                .collect();
            json!({"ok": true, "devices": devices, "max": 16, "maxButtons": 48, "free": 700})
        }
        "list" => {
            let Some(rows) = ir.devices.get(&dev) else { return nf() };
            let page = num("page").max(0) as usize;
            let part: Vec<Value> = rows
                .iter()
                .skip(page * 12)
                .take(12)
                .map(|r| {
                    let c = r["c"].as_str().unwrap_or("");
                    let mut b = json!({"n": r["n"], "p": c.split(':').next().unwrap_or("")});
                    if r["f"].as_i64().unwrap_or(0) != 0 {
                        b["f"] = json!(1);
                    }
                    b
                })
                .collect();
            json!({"ok": true, "dev": dev, "buttons": part, "page": page, "total": rows.len(),
                   "more": rows.len() > (page + 1) * 12})
        }
        "get" => match find(ir) {
            Some(i) => json!({"ok": true, "dev": dev, "btn": btn, "c": ir.devices[&dev][i]["c"]}),
            None => nf(),
        },
        "send" => {
            let code = get("code");
            if !code.is_empty() {
                ir.sent.push(("code".into(), code, num("repeat")));
                return json!({"ok": true, "sent": "code"});
            }
            if find(ir).is_none() {
                return nf();
            }
            ir.sent.push((dev.clone(), btn.clone(), num("repeat")));
            json!({"ok": true, "sent": btn, "dev": dev})
        }
        "put" => {
            let fav = num("fav");
            match find(ir) {
                Some(i) => {
                    let r = &mut ir.devices.get_mut(&dev).unwrap()[i];
                    r["c"] = json!(get("code"));
                    r["f"] = json!(fav);
                }
                None => ir.devices.entry(dev.clone()).or_default().push(json!({"n": btn, "c": get("code"), "f": fav})),
            }
            json!({"ok": true, "dev": dev, "btn": btn})
        }
        "rm" => {
            if !ir.devices.contains_key(&dev) {
                return nf();
            }
            if btn.is_empty() {
                ir.devices.remove(&dev);
            } else if let Some(i) = find(ir) {
                ir.devices.get_mut(&dev).unwrap().remove(i);
            } else {
                return nf();
            }
            json!({"ok": true})
        }
        "rename" => {
            if !ir.devices.contains_key(&dev) {
                return nf();
            }
            let to = get("to");
            if btn.is_empty() {
                let rows = ir.devices.remove(&dev).unwrap();
                ir.devices.insert(to, rows);
            } else if let Some(i) = find(ir) {
                ir.devices.get_mut(&dev).unwrap()[i]["n"] = json!(to);
            } else {
                return nf();
            }
            json!({"ok": true})
        }
        "fav" => match find(ir) {
            Some(i) => {
                let on = num("on");
                ir.devices.get_mut(&dev).unwrap()[i]["f"] = json!(on);
                json!({"ok": true, "fav": on != 0})
            }
            None => nf(),
        },
        "learn" => {
            ir.learning = Some((dev.clone(), btn.clone()));
            emit(json!({"src": "agent", "evt": "ir", "state": "learning", "dev": dev, "btn": btn, "step": 1}));
            json!({"ok": true, "learning": true, "dev": dev, "btn": btn})
        }
        "cancel" => {
            ir.learning = None;
            emit(json!({"src": "agent", "evt": "ir", "state": "cancelled", "by": "agent"}));
            json!({"ok": true})
        }
        "diag" => json!({"ok": true, "tx": 46, "rx": 42, "txIdle": 0, "rxIdle": 1, "pins": "ok"}),
        _ => json!({"ok": false, "error": "Unknown @ir op", "code": "op"}),
    }
}

/// The user presses the remote twice while learning: the stick saves it (test_ir.py `press`).
pub fn press(ir: &mut FakeIr, code: &str, emit: &mut dyn FnMut(Value)) -> bool {
    let Some((dev, btn)) = ir.learning.take() else { return false };
    let btn = if btn.is_empty() { "button 1".to_string() } else { btn };
    emit(json!({"src": "agent", "evt": "ir", "state": "learning", "dev": dev, "btn": btn, "step": 2}));
    ir.devices.entry(dev.clone()).or_default().push(json!({"n": btn, "c": code}));
    let p = code.split(':').next().unwrap_or("");
    emit(json!({"src": "agent", "evt": "ir", "state": "saved", "dev": dev, "btn": btn, "p": p}));
    true
}

/// `{"ir":{"<op>":{…args}}}` -> the same as `@ir op=<op>&…`.
pub fn message_args(o: &Map<String, Value>) -> Option<(String, Vec<(String, String)>)> {
    let (op, a) = o.iter().next()?;
    let mut args = vec![("op".to_string(), op.clone())];
    if let Some(a) = a.as_object() {
        for (k, v) in a {
            args.push((k.clone(), v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())));
        }
    }
    Some((op.clone(), args))
}

/// The 2.5.0 parts of `@state`: `ir` (unless unsupported) and `air`.
pub fn state_extra(st: &mut Value, ir: &FakeIr) {
    if ir.unsupported {
        if let Some(m) = st.as_object_mut() {
            m.remove("ir");
        }
    } else {
        st["ir"] = json!({"v": 1, "n": ir.devices.len(), "tx": 46, "rx": 42});
    }
    if st.get("air").is_none() {
        st["air"] = json!({"mode": "home", "sens": 5, "invX": false, "invY": false, "scroll": 5,
                           "invScroll": false, "talkMin": 0, "keys": "arrows", "mouseKnown": true});
    }
}

/// `@air`, `@mode`, `@games` (StickS3 2.5.0); `None` for other actions.
pub fn run_25(st: &mut Value, action: &str, args: &[(String, String)]) -> Option<Value> {
    let get = |k: &str| args.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    let num = |k: &str| get(k).and_then(|v| v.parse::<i64>().ok());
    Some(match action {
        "games" => {
            let games: Vec<Value> = [("stack", "Stack", 23, true), ("flappy", "Flappy", 0, false), ("dino", "Dino", 0, false),
                                     ("reaction", "Reaction", 231, false), ("maze", "Maze", 0, true),
                                     ("balance", "Balance", 0, true), ("catch", "Catch", 0, true)]
                .iter()
                .map(|(id, t, best, tilt)| json!({"id": id, "title": t, "best": best, "tilt": tilt, "low": *id == "reaction"}))
                .collect();
            json!({"ok": true, "games": games})
        }
        "mode" => match get("name").as_deref() {
            Some(m @ ("mouse" | "presenter" | "media" | "home")) => {
                st["air"]["mode"] = json!(m);
                json!({"ok": true, "mode": m})
            }
            _ => json!({"ok": false, "error": "name must be mouse, presenter, media or home"}),
        },
        "air" => {
            let a = &mut st["air"];
            for (k, key, lo, hi) in [("sens", "sens", 1, 10), ("scroll", "scroll", 1, 10), ("talk", "talkMin", 0, 600)]
            {
                if let Some(v) = num(k) {
                    if !(lo..=hi).contains(&v) {
                        return Some(json!({"ok": false, "error": format!("{k} must be {lo}-{hi}")}));
                    }
                    a[key] = json!(v);
                }
            }
            for (k, key) in [("invx", "invX"), ("invy", "invY"), ("invscroll", "invScroll")] {
                if let Some(v) = num(k) {
                    a[key] = json!(v != 0);
                }
            }
            if let Some(k) = get("keys") {
                if k != "arrows" && k != "pages" {
                    return Some(json!({"ok": false, "error": "keys must be arrows or pages"}));
                }
                a["keys"] = json!(k);
            }
            json!({"ok": true, "air": a.clone()})
        }
        _ => return None,
    })
}
