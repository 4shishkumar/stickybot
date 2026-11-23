//! The custom AppKit view lives on the existing tray menu item.
use block2::{DynBlock, RcBlock};
use serde_json::json;
use std::{
    ffi::{CString, c_char, c_void},
    sync::Arc,
};
use stickybot_server::AppState;
use tauri::{Manager, menu::ContextMenu};
use tokio::sync::mpsc;

unsafe extern "C" {
    fn stickybot_volume_update(
        menu: *mut c_void,
        device: *const c_char,
        enabled: i32,
        on: i32,
        raw: i32,
        change: &DynBlock<dyn Fn(i32, i32)>,
    );
}

struct Change {
    target: Arc<AppState>,
    generation: u64,
    on: bool,
    raw: u8,
}

pub struct Control(mpsc::UnboundedSender<Change>);

impl Control {
    pub fn new(app: tauri::AppHandle, root: Arc<AppState>) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<Change>();
        tauri::async_runtime::spawn(async move {
            while let Some(mut change) = rx.recv().await {
                // If the link is slow, apply the newest position instead of playing back a drag.
                while let Ok(newer) = rx.try_recv() {
                    change = newer;
                }
                let st = change.target;
                if st.link.generation() != change.generation
                    || st.link.state() != stick_link::LinkState::Connected
                    || !root.devices.default_session().is_some_and(|s| Arc::ptr_eq(&s, &st))
                {
                    continue;
                }
                let write = st.sound(json!({"on": change.on, "volume": change.raw}));
                if st.link.with_generation(change.generation, write).await.is_none() {
                    log::warn!("volume: stick did not confirm the change");
                    if let Some(tray) = app.try_state::<Arc<super::Tray>>() {
                        tray.refresh_volume();
                    }
                }
            }
        });
        Self(tx)
    }

    pub fn render(
        &self,
        app: &tauri::AppHandle,
        menu: &tauri::menu::Menu<tauri::Wry>,
        root: &Arc<AppState>,
        reading: Option<(String, u64, bool, u8)>,
    ) {
        let tx = self.0.clone();
        let (id, generation, on, raw) = reading.unwrap_or_default();
        let target = root.devices.resolve(&id);
        let enabled = target.is_some();
        let menu = menu.clone();
        let _ = app.run_on_main_thread(move || {
            let change = RcBlock::new(move |on: i32, raw: i32| {
                if let Some(st) = &target
                    && let Ok(raw) = u8::try_from(raw)
                {
                    let _ = tx.send(Change { target: st.clone(), generation, on: on != 0, raw });
                }
            });
            // Tauri's menu shares this native NSMenu with the status item. Its pointer is valid
            // for the lifetime of our menu handle, and all AppKit operations run on this thread.
            fn native(menu: &impl ContextMenu) -> *mut c_void {
                menu.inner_context().ns_menu()
            }
            let id = CString::new(format!("{id}@{generation}")).unwrap_or_default();
            unsafe {
                stickybot_volume_update(native(&menu), id.as_ptr(), enabled.into(), on.into(), raw.into(), &change)
            }
        });
    }
}
