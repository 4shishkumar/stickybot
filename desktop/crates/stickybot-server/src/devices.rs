//! Device registry. A session owns a link and conversation; Mac services remain shared.
use crate::{AppState, Options, config, hub};
use serde_json::{Map, Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, Weak},
};
use stick_link::Backend;

const SCOPED: &[&str] = &[
    "stick",
    "brain",
    "model",
    "system_prompt",
    "speak_replies",
    "speak_output",
    "speak_voice",
    "speak_max_words",
    "speak_quick",
    "followup_minutes",
    "brain_mode",
    "brain_prewarm",
    "brain_idle_min",
    "openrouter_model",
    "openrouter_max_tokens",
    "stream_replies",
    "stream_interval_ms",
    "speak_stream",
    "safe_tools",
    "tool_profiles",
    "ir_brands",
    "wireless_mic",
    "wireless_mic_carrier",
    "wispr_mic_name",
    "wispr_switch_ms",
    "wispr_tail_ms",
    "stick_approvals",
];

#[derive(Default)]
pub struct DeviceManager {
    sessions: Mutex<BTreeMap<String, Arc<AppState>>>,
    root: Mutex<Weak<AppState>>,
    transaction: Mutex<()>,
}

impl DeviceManager {
    pub fn init(&self, root: &Arc<AppState>) -> Result<(), String> {
        *self.root.lock().unwrap() = Arc::downgrade(root);
        let entries = root.cfg.lock().unwrap().get("devices").and_then(Value::as_array).cloned().unwrap_or_default();
        for (index, entry) in entries.into_iter().enumerate() {
            let Some(address) = entry["address"].as_str() else {
                log::warn!("Skipping invalid saved device at position {index}");
                continue;
            };
            if entry["id"] == "primary" {
                continue;
            }
            if self
                .add_inner(root, address, entry["name"].as_str().unwrap_or("Stick"), Some(entry.clone()), false)
                .is_err()
            {
                // Saved user input must not prevent healthy sessions (or the dashboard) starting.
                // Do not log the entry or its error: either can contain private names or paths.
                log::warn!("Skipping invalid saved device at position {index}");
            }
        }
        Ok(())
    }

    /// A firmware name report updates the authoritative config without reconnecting the link.
    pub fn rename(&self, session: &Arc<AppState>, name: &str) -> Result<(), String> {
        let _guard = self.transaction.lock().unwrap();
        let root = self.root().unwrap_or_else(|| session.clone());
        let mut cfg = root.cfg.lock().unwrap();
        let mut next = cfg.clone();
        if session.device_id == "primary" {
            let Some(stick) = next.get_mut("stick").and_then(Value::as_object_mut) else {
                return Ok(());
            };
            stick.insert("name".into(), json!(name));
        } else {
            let entry = next
                .get_mut("devices")
                .and_then(Value::as_array_mut)
                .and_then(|entries| entries.iter_mut().find(|entry| entry["id"] == session.device_id))
                .ok_or("Device removed")?;
            entry["name"] = json!(name);
            if let Some(stick) = entry["config"].get_mut("stick").and_then(Value::as_object_mut) {
                stick.insert("name".into(), json!(name));
            }
        }
        config::save(&root.paths, &next).map_err(|e| e.to_string())?;
        *cfg = next.clone();
        drop(cfg);
        self.refresh_children(&next);
        let snapshot = session.cfg.lock().unwrap().clone();
        session.hub.emit("config", json!({"config":snapshot}));
        Ok(())
    }
    pub fn note_identity(&self, session: &AppState, state: &Value) {
        let Some(id) = state["deviceId"].as_str().filter(|s| !s.is_empty()) else {
            return;
        };
        let Some(root) = self.root() else {
            return;
        };
        let identity = json!({"deviceId":id,"board":state["board"]});
        let mut cfg = root.cfg.lock().unwrap();
        let mut states = cfg.get("device_states").and_then(Value::as_object).cloned().unwrap_or_default();
        if states.get(&session.device_id) == Some(&identity) {
            return;
        }
        states.insert(session.device_id.clone(), identity);
        let mut next = cfg.clone();
        next.insert("device_states".into(), json!(states));
        if config::save(&root.paths, &next).is_ok() {
            *cfg = next;
        }
    }
    pub fn root(&self) -> Option<Arc<AppState>> {
        self.root.lock().unwrap().upgrade()
    }
    pub fn resolve(&self, id: &str) -> Option<Arc<AppState>> {
        if id == "primary" {
            return self.root();
        }
        self.sessions.lock().unwrap().get(id).cloned()
    }
    pub fn all(&self) -> Vec<Arc<AppState>> {
        let mut out: Vec<_> = self.sessions.lock().unwrap().values().cloned().collect();
        if let Some(root) = self.root() {
            out.insert(0, root);
        }
        out
    }
    pub fn default_session(&self) -> Option<Arc<AppState>> {
        let root = self.root()?;
        let id =
            root.cfg.lock().unwrap().get("default_device_id").and_then(Value::as_str).unwrap_or("primary").to_owned();
        self.resolve(&id)
    }
    pub fn describe(&self, root: &Arc<AppState>) -> Value {
        let mut sessions = self.all();
        if sessions.is_empty() {
            sessions.push(root.clone());
        }
        let devices: Vec<_> = sessions.into_iter().filter_map(|st| {
            let target = st.stick()?;
            let hosts = st.host_state.lock().unwrap().clone();
            Some(json!({"id":st.device_id,"name":target.name,"address":target.address,"paused":st.link.paused(),
                "status":{"link":st.link.state().as_str(),"agent_state":st.agent_state()},"state":hosts,"board":hosts["board"],"hosts":hosts}))
        }).collect();
        let default_id = root.cfg.lock().unwrap().get("default_device_id").cloned().unwrap_or(json!("primary"));
        json!({"devices":devices,"default_id":default_id,"host":root.host.json()})
    }
    pub fn add(&self, root: &Arc<AppState>, address: &str, name: &str) -> Result<Arc<AppState>, String> {
        self.add_inner(root, address, name, None, true)
    }
    fn add_inner(
        &self,
        root: &Arc<AppState>,
        address: &str,
        name: &str,
        entry: Option<Value>,
        persist: bool,
    ) -> Result<Arc<AppState>, String> {
        let address = address.trim();
        if address.is_empty() || address.len() > 128 || address.chars().any(char::is_control) {
            return Err("Invalid Bluetooth address".into());
        }
        let _guard = self.transaction.lock().unwrap();
        for existing in self.all() {
            if existing.stick().is_some_and(|s| s.address.eq_ignore_ascii_case(address)) {
                return Ok(existing);
            }
        }
        if self.sessions.lock().unwrap().len() >= 32 {
            return Err("At most 32 configured devices are supported".into());
        }
        let id = entry
            .as_ref()
            .and_then(|e| e["id"].as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("device-{}", config::new_token()));
        let mut cfg = root.cfg.lock().unwrap().clone();
        let overrides = entry.as_ref().and_then(|e| e["config"].as_object()).cloned().unwrap_or_else(|| {
            cfg.iter()
                .filter(|(key, _)| SCOPED.contains(&key.as_str()) && key.as_str() != "stick")
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        });
        cfg.extend(overrides.clone());
        cfg.insert("stick".into(), json!({"address":address,"name":name}));
        let opts = Options { port: root.port, home: root.paths.home.clone(), fake_stick: root.fake, import_from: None };
        let backend = if root.fake { Backend::Fake(stick_link::fake::FakeStick::new()) } else { root.backend.clone() };
        let child = AppState::new_session(&opts, backend, Some(root), id.clone(), Some(cfg))?;
        if persist {
            let mut cfg = root.cfg.lock().unwrap();
            let mut entries = cfg.get("devices").and_then(Value::as_array).cloned().unwrap_or_default();
            entries.push(json!({"id":id,"address":address,"name":name,"config":overrides}));
            cfg.insert("devices".into(), json!(entries));
            if cfg.get("stick").is_none_or(Value::is_null) && self.sessions.lock().unwrap().is_empty() {
                cfg.insert("default_device_id".into(), json!(id));
            }
            config::save(&root.paths, &cfg).map_err(|e| e.to_string())?;
        }
        self.sessions.lock().unwrap().insert(id, child.clone());
        if tokio::runtime::Handle::try_current().is_ok() {
            crate::start_device_session(child.clone());
            if entry.as_ref().is_some_and(|e| e["paused"] == true) {
                child.link.pause(true);
            }
        }
        Ok(child)
    }
    pub fn default_to(&self, root: &Arc<AppState>, id: &str) -> Result<(), String> {
        if self.resolve(id).is_none() {
            return Err("Unknown device".into());
        }
        root.update_config(json!({"default_device_id":id}).as_object().unwrap().clone()).map(|_| ())
    }
    pub fn remove(&self, root: &Arc<AppState>, id: &str) -> Result<(), String> {
        let st = self.resolve(id).ok_or("Unknown device")?;
        let _guard = self.transaction.lock().unwrap();
        let mut cfg = root.cfg.lock().unwrap();
        let mut next = cfg.clone();
        if id == "primary" {
            next.insert("stick".into(), Value::Null);
        }
        if let Some(entries) = next.get_mut("devices").and_then(Value::as_array_mut) {
            entries.retain(|e| e["id"] != id);
        }
        for key in ["approval_devices", "notification_devices"] {
            if let Some(ids) = next.get_mut(key).and_then(Value::as_array_mut) {
                ids.retain(|value| value.as_str() != Some(id));
            }
        }
        if next.get("default_device_id").and_then(Value::as_str) == Some(id) {
            next.insert("default_device_id".into(), json!("primary"));
        }
        config::save(&root.paths, &next).map_err(|e| e.to_string())?;
        *cfg = next.clone();
        drop(cfg);
        self.refresh_children(&next);
        st.link.set_target(None);
        st.brain().cancel();
        crate::speak::stop(&st);
        if id != "primary" && tokio::runtime::Handle::try_current().is_ok() {
            let removed = st.clone();
            tokio::spawn(async move {
                crate::wmic::drop_device(removed.clone(), "Device removed".into()).await;
                let mut state = removed.link.watch_state();
                let _ = tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    state.wait_for(|s| *s != stick_link::LinkState::Connected),
                )
                .await;
                for worker in removed.workers.lock().unwrap().drain(..) {
                    worker.abort();
                }
            });
        }
        self.sessions.lock().unwrap().remove(id);
        Ok(())
    }
    pub fn pause(&self, root: &Arc<AppState>, id: &str, paused: bool) -> Result<(), String> {
        let st = self.resolve(id).ok_or("Unknown device")?;
        let _guard = self.transaction.lock().unwrap();
        let mut cfg = root.cfg.lock().unwrap();
        let mut next = cfg.clone();
        if id == "primary" {
            next.insert("primary_paused".into(), json!(paused));
        }
        if let Some(entries) = next.get_mut("devices").and_then(Value::as_array_mut) {
            for entry in entries {
                if entry["id"] == id {
                    entry["paused"] = json!(paused);
                }
            }
        }
        config::save(&root.paths, &next).map_err(|e| e.to_string())?;
        *cfg = next;
        st.link.pause(paused);
        Ok(())
    }
    pub fn update_session(
        &self,
        root: &Arc<AppState>,
        session: &AppState,
        changes: Map<String, Value>,
    ) -> Result<Map<String, Value>, String> {
        // Serialize read/merge/write through the authoritative root, never save a child's stale snapshot.
        let _guard = self.transaction.lock().unwrap();
        let mut cfg = root.cfg.lock().unwrap();
        let mut next = cfg.clone();
        let entries = next.get_mut("devices").and_then(Value::as_array_mut).ok_or("Device is no longer configured")?;
        let entry =
            entries.iter_mut().find(|e| e["id"] == session.device_id).ok_or("Device is no longer configured")?;
        let scoped = SCOPED;
        let local: Map<_, _> =
            changes.iter().filter(|(k, _)| scoped.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
        let mut overrides = entry["config"].as_object().cloned().unwrap_or_default();
        overrides.extend(local);
        if let Some(stick) = changes.get("stick").and_then(Value::as_object) {
            if let Some(address) = stick.get("address") {
                entry["address"] = address.clone();
            }
            if let Some(name) = stick.get("name") {
                entry["name"] = name.clone();
            }
            overrides.remove("stick");
        }
        entry["config"] = json!(overrides);
        let global: Map<_, _> = changes
            .iter()
            .filter(|(k, _)| {
                !scoped.contains(&k.as_str()) && k.as_str() != "devices" && k.as_str() != "default_device_id"
            })
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        config::save(&root.paths, &next).map_err(|e| e.to_string())?;
        *cfg = next.clone();
        drop(cfg);
        let next = if global.is_empty() { next } else { root.update_config(global)? };
        self.refresh_children(&next);
        if changes.contains_key("brain") {
            session.brain().switched();
        }
        if changes.contains_key("stick") {
            session.link.set_target(session.stick());
        }
        if let Some(child) = self.resolve(&session.device_id) {
            crate::wmic::config_changed(&child, &changes);
        }
        let result = session.cfg.lock().unwrap().clone();
        session.hub.emit("config", json!({"config":result,"t":hub::now()}));
        Ok(result)
    }
    pub fn refresh_children(&self, next: &Map<String, Value>) {
        for child in self.sessions.lock().unwrap().values() {
            let mut effective = next.clone();
            if let Some(entry) =
                next["devices"].as_array().and_then(|es| es.iter().find(|e| e["id"] == child.device_id))
            {
                effective.insert("stick".into(), json!({"address":entry["address"],"name":entry["name"]}));
                if let Some(overrides) = entry["config"].as_object() {
                    effective.extend(overrides.clone());
                }
            }
            *child.cfg.lock().unwrap() = effective;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn root(name: &str) -> Arc<AppState> {
        let home = std::env::temp_dir().join(format!("stickybot-devices-{name}-{}", config::new_token()));
        let opts = Options { home, port: 8670, fake_stick: true, import_from: None };
        let st = AppState::new(&opts, Backend::Fake(stick_link::fake::FakeStick::new())).unwrap();
        st.devices.init(&st).unwrap();
        st
    }
    async fn request(st: &Arc<AppState>, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
        let req = Request::builder()
            .method(method)
            .uri(path)
            .header("host", "localhost:8670")
            .header("x-stick-token", &st.token)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let response = crate::api::router(st.clone()).oneshot(req).await.unwrap();
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&body).unwrap())
    }
    #[test]
    fn invalid_saved_entries_and_capacity_do_not_prevent_other_sessions_loading() {
        let root = root("bad-saved-devices");
        let mut entries = vec![json!({"id":"bad","address":""})];
        entries.extend(
            (0..34).map(|n| json!({"id":format!("saved-{n}"),"address":format!("ADDR-{n}"),"name":"Test stick"})),
        );
        root.cfg.lock().unwrap().insert("devices".into(), json!(entries));
        root.devices.init(&root).unwrap();
        assert!(root.devices.resolve("primary").is_some());
        assert!(root.devices.resolve("bad").is_none());
        assert!(root.devices.resolve("saved-0").is_some());
        assert!(root.devices.resolve("saved-31").is_some());
        assert!(root.devices.resolve("saved-32").is_none());
    }

    #[tokio::test]
    async fn removing_a_device_prunes_saved_and_live_destination_lists() {
        let root = root("remove-destinations");
        let a = root.devices.add(&root, "A", "One").unwrap();
        let b = root.devices.add(&root, "B", "Two").unwrap();
        root.update_config(
            json!({"approval_devices":[a.device_id], "notification_devices":[a.device_id,b.device_id]})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
        root.devices.remove(&root, &a.device_id).unwrap();
        for cfg in [config::load(&root.paths), root.cfg.lock().unwrap().clone(), b.cfg.lock().unwrap().clone()] {
            assert_eq!(cfg["approval_devices"], json!([]));
            assert_eq!(cfg["notification_devices"], json!([b.device_id]));
        }
        root.devices.remove(&root, &b.device_id).unwrap();
    }

    #[tokio::test]
    async fn sessions_share_mac_services_but_preserve_default_and_configuration() {
        let root = root("registry");
        let a = root.devices.add(&root, "A", "Desk stick").unwrap();
        let b = root.devices.add(&root, "B", "Travel stick").unwrap();
        assert!(!Arc::ptr_eq(&a, &b));
        assert!(Arc::ptr_eq(&a.speech, &b.speech));
        assert!(!Arc::ptr_eq(&a.home, &b.home));
        assert!(Arc::ptr_eq(&a.home.weather, &b.home.weather));
        assert!(Arc::ptr_eq(&a.home.feed, &b.home.feed));
        assert!(Arc::ptr_eq(&a.home.pet, &b.home.pet));
        assert!(Arc::ptr_eq(&a.keys, &b.keys));
        assert!(Arc::ptr_eq(&a.calendar, &b.calendar));
        assert!(Arc::ptr_eq(&a.approvals, &b.approvals));
        assert_eq!(root.devices.default_session().unwrap().device_id, a.device_id);
        root.update_config(json!({"model":"primary-only","wireless_mic":true}).as_object().unwrap().clone()).unwrap();
        assert_eq!(a.cfg.lock().unwrap()["model"], "sonnet");
        assert_eq!(a.cfg.lock().unwrap()["wireless_mic"], false);
        a.update_config(json!({"model":"model-a","github":{"on":false}}).as_object().unwrap().clone()).unwrap();
        b.update_config(json!({"model":"model-b"}).as_object().unwrap().clone()).unwrap();
        assert_eq!(a.cfg.lock().unwrap()["model"], "model-a");
        assert_eq!(b.cfg.lock().unwrap()["model"], "model-b");
        assert_eq!(root.cfg.lock().unwrap()["github"]["on"], false);
        root.update_config(json!({"stt_hints":["new word"]}).as_object().unwrap().clone()).unwrap();
        assert_eq!(b.cfg.lock().unwrap()["stt_hints"], json!(["new word"]));
        root.devices.default_to(&root, &b.device_id).unwrap();
        let (code, status) = request(&root, "GET", "/api/status", Value::Null).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(status["stick"]["address"], "B");
        let (_, status) = request(&root, "GET", &format!("/api/devices/{}/api/status", a.device_id), Value::Null).await;
        assert_eq!(status["stick"]["address"], "A");
        let (code, _) =
            request(&root, "POST", "/api/devices/missing/api/ask", json!({"text":"never route elsewhere"})).await;
        assert_eq!(code, StatusCode::NOT_FOUND);
        let disk = config::load(&root.paths);
        assert_eq!(disk["devices"].as_array().unwrap().len(), 2);
        assert_eq!(disk["default_device_id"], b.device_id);
        assert_eq!(disk["github"]["on"], false);
        crate::wmic::note_name(&a, &json!({"board":"m5-sticks3","name":"Renamed on stick"}));
        let disk = config::load(&root.paths);
        assert_eq!(disk["devices"].as_array().unwrap().len(), 2);
        assert_eq!(disk["default_device_id"], b.device_id);
        assert_eq!(a.stick().unwrap().name, "Renamed on stick");
        assert_eq!(b.stick().unwrap().address, "B");
        root.devices.remove(&root, &a.device_id).unwrap();
        root.devices.remove(&root, &b.device_id).unwrap();
    }
    #[tokio::test]
    async fn labels_and_dictation_ownership_are_independent_of_device_selection() {
        let root = root("ownership");
        let a = root.devices.add(&root, "A", "One").unwrap();
        let b = root.devices.add(&root, "B", "Two").unwrap();
        assert!(crate::wmic::claim_audio(&a));
        assert!(!crate::wmic::claim_audio(&b));
        crate::wmic::release_audio(&b);
        assert_eq!(root.audio_owner.lock().unwrap().as_deref(), Some(a.device_id.as_str()));
        crate::wmic::release_audio(&a);
        assert!(crate::wmic::claim_audio(&b));
        crate::wmic::release_audio(&b);
        a.set_activity("listening", "");
        assert!(crate::selfupdate::Busy::of(&root).voice);
        a.set_idle();
        let (code, host) = request(&root, "PUT", "/api/host", json!({"label":"Desk"})).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(host["label"], "Desk");
        assert_eq!(host["name"], "Test Mac");
        assert_eq!(a.host.json()["id"], b.host.json()["id"]);
        let id = host["id"].clone();
        let reloaded = crate::hosts::HostIdentity::with_name(&root.paths, "Renamed Mac").unwrap();
        assert_eq!(reloaded.json()["id"], id);
        assert_eq!(reloaded.json()["label"], "Desk");
        let (code, _) = request(&root, "PUT", "/api/host", json!({"label":"a label that exceeds twelve"})).await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        let (code, host) = request(&root, "PUT", "/api/host", json!({"label":""})).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(host["custom_label"], "");
        let event = a.hub.emit("test", json!({}));
        assert_eq!(event["device_id"], a.device_id);
        root.devices.remove(&root, &a.device_id).unwrap();
        root.devices.remove(&root, &b.device_id).unwrap();
    }
    #[tokio::test]
    async fn scoped_history_filters_before_limit_and_keeps_legacy_rows_on_primary() {
        let root = root("history");
        let a = root.devices.add(&root, "A", "One").unwrap();
        let b = root.devices.add(&root, "B", "Two").unwrap();
        std::fs::write(root.paths.history(), "{\"heard\":\"legacy\"}\n").unwrap();
        a.history(&json!({"heard":"only-a"}));
        b.history(&json!({"heard":"only-b"}));
        let (_, rows) =
            request(&root, "GET", &format!("/api/devices/{}/api/history?n=1", a.device_id), Value::Null).await;
        assert_eq!(rows.as_array().unwrap().len(), 1);
        assert_eq!(rows[0]["heard"], "only-a");
        assert_eq!(rows[0]["device_id"], a.device_id);
        let (_, rows) = request(&root, "GET", "/api/devices/primary/api/history", Value::Null).await;
        assert_eq!(rows, json!([{"heard":"legacy"}]));
        let (_, rows) = request(&root, "GET", &format!("/api/devices/{}/api/history", b.device_id), Value::Null).await;
        assert_eq!(rows.as_array().unwrap().len(), 1);
        assert_eq!(rows[0]["heard"], "only-b");
        crate::events::notify(&a, "test", "a event", "", None, None, None).await;
        assert_eq!(crate::events::history(&a, 1)[0]["title"], "a event");
        assert!(crate::events::history(&b, 10).is_empty());
        root.devices.remove(&root, &a.device_id).unwrap();
        root.devices.remove(&root, &b.device_id).unwrap();
    }

    #[tokio::test]
    async fn home_display_slots_reply_cache_and_world_sends_are_per_device() {
        let root = root("home-display");
        let a = root.devices.add(&root, "A", "One").unwrap();
        let b = root.devices.add(&root, "B", "Two").unwrap();
        let mut aw = a.link.watch_state();
        aw.wait_for(|s| *s == stick_link::LinkState::Connected).await.unwrap();
        let mut bw = b.link.watch_state();
        bw.wait_for(|s| *s == stick_link::LinkState::Connected).await.unwrap();
        crate::home::put(&a, "pin", "A private pin", "A details", 30.);
        assert!(!crate::home::home_card(&b).to_string().contains("A private"));
        assert!(!crate::home::clear(&b, "pin"));
        assert_eq!(crate::home::home_card(&a)["a"], "A private pin");
        crate::home::clear(&a, "pin");
        crate::home::put(&a, "progress", "A private progress", "3/10", 30.);
        assert!(!crate::home::home_card(&b).to_string().contains("A private"));
        crate::home::clear(&a, "progress");
        *a.last_reply.lock().unwrap() = Some(json!({"t":hub::now(),"text":"A private reply"}));
        *b.last_reply.lock().unwrap() = Some(json!({"t":hub::now(),"text":"B private reply"}));
        assert_eq!(crate::home::home_card(&a)["b"], "A private reply");
        assert_eq!(crate::home::home_card(&b)["b"], "B private reply");
        assert!(Arc::ptr_eq(&a.home.weather, &b.home.weather));
        assert!(Arc::ptr_eq(&a.home.gh_state, &b.home.gh_state));
        assert!(crate::home::push_world(&a, true).await);
        assert!(crate::home::push_world(&b, true).await);
        assert!(!crate::home::push_world(&a, true).await);
        assert!(!crate::home::push_world(&b, true).await);
        root.devices.remove(&root, &a.device_id).unwrap();
        root.devices.remove(&root, &b.device_id).unwrap();
    }

    #[tokio::test]
    async fn explicit_primary_notifications_override_default_and_policy_without_changing_background_delivery() {
        let root = root("explicit-primary");
        root.update_config(json!({"stick":{"address":"A","name":"Primary"}}).as_object().unwrap().clone()).unwrap();
        let root_worker = root.link.spawn();
        root.link.set_target(root.stick());
        let b = root.devices.add(&root, "B", "Two").unwrap();
        let mut aw = root.link.watch_state();
        aw.wait_for(|s| *s == stick_link::LinkState::Connected).await.unwrap();
        let mut bw = b.link.watch_state();
        bw.wait_for(|s| *s == stick_link::LinkState::Connected).await.unwrap();
        root.devices.default_to(&root, &b.device_id).unwrap();
        root.update_config(json!({"notification_devices":[b.device_id]}).as_object().unwrap().clone()).unwrap();
        let Backend::Fake(a_fake) = &root.backend else { panic!("fake only") };
        let Backend::Fake(b_fake) = &b.backend else { panic!("fake only") };
        let (a_before, b_before) = (a_fake.writes().len(), b_fake.writes().len());
        let outcome = crate::mcp::call(
            &root,
            "notify",
            &json!({"device_id":"primary","text":"Only primary MCP"}),
            &crate::tool_policy::Context::internal("test"),
        )
        .await;
        assert!(!outcome.1, "{outcome:?}");
        let (code, _) = request(&root, "POST", "/api/devices/primary/api/events/test", json!({"kind":"test"})).await;
        assert_eq!(code, StatusCode::OK);
        *root.mcp.alerts.lock().unwrap() = crate::mcp::RateLimit::new(0., 1000);
        let (code,rpc)=request(&root,"POST","/api/devices/primary/mcp",json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"notify","arguments":{"text":"Scoped primary RPC"}}})).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(rpc["result"]["isError"], false, "{rpc}");
        let a_wire = String::from_utf8(a_fake.writes()[a_before..].concat()).unwrap();
        let b_wire = String::from_utf8(b_fake.writes()[b_before..].concat()).unwrap();
        assert!(a_wire.contains("Only primary MCP"));
        assert!(a_wire.contains("Test event"));
        assert!(a_wire.contains("Scoped primary RPC"));
        assert!(!b_wire.contains("Only primary MCP"));
        assert!(!b_wire.contains("Test event"));
        assert!(!b_wire.contains("Scoped primary RPC"));
        let history = crate::events::history(&root, 10);
        for row in history.iter().filter(|r| {
            r["title"] == "Only primary MCP" || r["title"] == "Test event" || r["title"] == "Scoped primary RPC"
        }) {
            assert_eq!(row["device_id"], "primary");
            assert_eq!(row["device_ids"], json!(["primary"]));
        }
        assert!(crate::events::history(&b, 10).is_empty());
        crate::events::notify(&root, "test", "Background policy", "", None, None, None).await;
        assert!(String::from_utf8(b_fake.writes()[b_before..].concat()).unwrap().contains("Background policy"));
        assert_eq!(crate::events::history(&b, 1)[0]["device_ids"], json!([b.device_id]));
        root.update_config(json!({"notification_devices":null}).as_object().unwrap().clone()).unwrap();
        crate::events::notify(&root, "test", "Background default", "", None, None, None).await;
        assert!(String::from_utf8(b_fake.writes()[b_before..].concat()).unwrap().contains("Background default"));
        root.devices.remove(&root, &b.device_id).unwrap();
        root_worker.abort();
    }

    #[test]
    fn legacy_target_is_preserved_and_duplicate_addresses_do_not_create_sessions() {
        let root = root("migration");
        root.update_config(json!({"stick":{"address":"LEGACY","name":"My stick"}}).as_object().unwrap().clone())
            .unwrap();
        let same = root.devices.add(&root, "legacy", "Renamed").unwrap();
        assert_eq!(same.device_id, "primary");
        assert_eq!(root.stick().unwrap().name, "My stick");
        assert_eq!(root.devices.describe(&root)["devices"].as_array().unwrap().len(), 1);
    }
}
