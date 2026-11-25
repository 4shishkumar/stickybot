//! Physical gesture notifications. The firmware routes these to its selected Mac.
//! Every request and reply stays bound to the device and connection which sent the event.

use crate::{AppState, commands};
use serde_json::{Value, json};
use std::sync::Arc;
use stick_link::buttons::MacAction;

pub(super) fn pressed(st: &Arc<AppState>, msg: &Value) {
    let (Some(gesture), Some(action)) = (msg["gesture"].as_str(), msg["action"].as_str()) else { return };
    if MacAction::parse(gesture, action).is_none() {
        return;
    }
    let (st, gesture, action) = (st.clone(), gesture.to_owned(), action.to_owned());
    let generation = st.link.bound_generation();
    st.pressed();
    tokio::spawn(async move {
        st.link
            .with_generation(generation, async {
                let result = execute(&st, generation, &gesture, &action).await;
                if !current(&st, generation) {
                    return;
                }
                let (phase, text) = match result {
                    Ok(Some(reply)) => ("done", reply),
                    Ok(None) => return,
                    Err(message) => ("error", message.to_string()),
                };
                st.hub.emit("reply", json!({"via":"button", "text":text}));
                if st.voice_owner.lock().unwrap().is_none()
                    && !st.approvals.pending_for(&st.device_id)
                    && !st.approvals.asking_for(&st.device_id)
                {
                    crate::talk::tell_stick(&st, phase, "", &text).await;
                }
            })
            .await;
    });
}

fn current(st: &AppState, generation: u64) -> bool {
    st.link.connected() && st.link.target().is_some() && !st.link.paused() && st.link.generation() == generation
}

async fn execute(
    st: &Arc<AppState>,
    generation: u64,
    gesture: &str,
    action: &str,
) -> Result<Option<String>, &'static str> {
    if !current(st, generation) {
        return Ok(None);
    }
    let Some(kind) = MacAction::parse(gesture, action) else { return Ok(None) };
    let state = st.link.request("@state").await.ok_or("Could not read the stick's button mappings.")?;
    if state["ok"] != true || state["buttons"][gesture] != action || !current(st, generation) {
        return Ok(None);
    }

    let apps = st.apps.clone();
    let runner = st.runner();
    let source = st.clone();
    // Resolving the app or loading a saved command can span a disconnect.
    let guarded: commands::Runner =
        Arc::new(move |action| if current(&source, generation) { runner(action) } else { String::new() });
    match kind {
        MacAction::App(name) => {
            let name = name.to_owned();
            tokio::task::spawn_blocking(move || {
                let path = apps.resolve_exact(&name).ok_or("That Mac app is not installed.")?;
                guarded(&commands::Action::Open(vec!["-a".into(), path]));
                Ok(None)
            })
            .await
            .map_err(|_| "Could not open that Mac app.")?
        }
        MacAction::Quick(name) => run_quick(st, generation, name, guarded).await,
    }
}

async fn run_quick(
    st: &Arc<AppState>,
    generation: u64,
    name: &str,
    guarded: commands::Runner,
) -> Result<Option<String>, &'static str> {
    let store = st.commands.clone();
    let name = name.to_owned();
    let command = tokio::task::spawn_blocking(move || {
        let mut matches = store.load().into_iter().filter(|c| c["name"].as_str() == Some(&name));
        let c = matches.next().ok_or("That quick command is no longer saved.")?;
        if matches.next().is_some() {
            return Err("Several quick commands have that name.");
        }
        Ok(c)
    })
    .await
    .map_err(|_| "Could not load quick commands.")??;
    // Button presses supply no speech slots. A saved command may still contain unrelated braces.
    if ["open_url", "open_app", "shell", "browser"]
        .iter()
        .filter_map(|k| command[k].as_str())
        .any(|v| ["{n}", "{q}", "{app}"].iter().any(|slot| v.contains(slot)))
    {
        return Err("That quick command needs a spoken argument.");
    }
    if !current(st, generation) {
        return Ok(None);
    }
    if let Some(spec) = command.get("ir") {
        let (ok, reply) = crate::ir::voice(st, spec).await;
        return ok.then_some(Some(reply)).ok_or("The quick command could not send IR.");
    }
    if let Some(what) = command["calendar"].as_str() {
        use crate::connections::calendar;
        // Joining opens a URL. Apply the same live-connection check on the blocking worker.
        if what == "join" {
            let (url, reply) = calendar::resolve_join(st).map_err(|_| "No meeting is available to join.")?;
            tokio::task::spawn_blocking(move || guarded(&commands::Action::Open(vec![url])))
                .await
                .map_err(|_| "Could not join the meeting.")?;
            return Ok(Some(reply));
        }
        let (ok, reply) = calendar::voice(st, what).await;
        return ok.then_some(Some(reply)).ok_or("The calendar command is unavailable.");
    }
    let apps = st.apps.clone();
    tokio::task::spawn_blocking(move || commands::run(&command, &commands::Slots::new(), false, &apps, &guarded))
        .await
        .map_err(|_| "Could not run the quick command.")?
        .map(Some)
        .ok_or("The quick command is unavailable.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::Action;
    use crate::talk::tests::agent;
    use tokio::time::{Duration, timeout};

    #[tokio::test]
    async fn a_press_uses_the_emitting_stick_even_when_another_stick_is_default() {
        use stick_link::{Backend, LinkState};
        let t = agent("physical-source", false).await;
        t.st.devices.init(&t.st).unwrap();
        let child = t.st.devices.add(&t.st, "SECOND", "Other stick").unwrap();
        t.st.devices.default_to(&t.st, "primary").unwrap();
        let mut state = child.link.watch_state();
        timeout(Duration::from_secs(3), state.wait_for(|s| *s == LinkState::Connected)).await.unwrap().unwrap();
        let Backend::Fake(second) = &child.backend else { unreachable!() };
        let bindings = json!({"b2":"app:Fixture Editor","b3":"none","ab":"reply","bh":"talk"});
        second.set_stick("buttons", bindings);
        let msg = json!({"src":"agent","evt":"button_action","gesture":"b2","action":"app:Fixture Editor"});
        // The default stick has never been configured for that action.
        assert_eq!(execute(&t.st, t.st.link.generation(), "b2", "app:Fixture Editor").await.unwrap(), None);
        second.notify(&msg);
        timeout(Duration::from_secs(2), async {
            while t.ran.lock().unwrap().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            *t.ran.lock().unwrap(),
            [Action::Open(vec!["-a".into(), "/Applications/Fixture Editor.app".into()])]
        );
        t.st.devices.remove(&t.st, &child.device_id).unwrap();
    }

    #[tokio::test]
    async fn physical_press_uses_current_mapping_and_exact_saved_action() {
        let t = agent("physical-buttons", false).await;
        t.fake.set_stick(
            "buttons",
            json!({"b2":"app:Fixture Editor","b3":"quick:Docs","ab":"quick:Open app","bh":"talk"}),
        );
        t.st.commands
            .save(
                json!({"commands":[
                    {"name":"Docs","phrases":["docs"],"open_url":"https://example.org"},
                    {"name":"Open app","phrases":["open {app}"],"open_app":"{app}"}
                ]})
                .as_object()
                .unwrap(),
            )
            .unwrap();
        let event = |g: &str, a: &str| json!({"src":"agent", "evt":"button_action", "gesture":g, "action":a});
        t.fake.notify(&event("b2", "app:Fixture Editor"));
        timeout(Duration::from_secs(2), async {
            while t.ran.lock().unwrap().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let gen_id = t.st.link.generation();
        assert_eq!(execute(&t.st, gen_id, "b3", "quick:Docs").await.unwrap().as_deref(), Some("Opened Docs"));
        assert!(execute(&t.st, gen_id, "ab", "quick:Open app").await.is_err());
        for (g, a) in [("b2", "app:Slack"), ("bh", "app:Fixture Editor"), ("b3", "quick:docs")] {
            assert_eq!(execute(&t.st, gen_id, g, a).await.unwrap(), None);
        }
        assert_eq!(execute(&t.st, gen_id.wrapping_sub(1), "b3", "quick:Docs").await.unwrap(), None);
        t.fake.set_stick("buttons", json!({"b2":"app:Fixture","b3":"quick:docs","ab":"quick:Other","bh":"talk"}));
        for (g, a) in [("b2", "app:Fixture"), ("b3", "quick:docs"), ("ab", "quick:Other")] {
            assert!(execute(&t.st, gen_id, g, a).await.is_err());
        }
        assert_eq!(
            *t.ran.lock().unwrap(),
            [
                Action::Open(vec!["-a".into(), "/Applications/Fixture Editor.app".into()]),
                Action::Open(vec!["https://example.org".into()])
            ]
        );
    }
}
