//! Start at login with `SMAppService.mainAppService` (macOS 13+, ServiceManagement): the app
//! registers itself; macOS shows it in System Settings → General → Login Items. No LaunchAgent plist,
//! no `launchctl`. Only works from the bundled app (a terminal binary reports "unavailable").

use stickybot_server::login::{LoginItem, LoginState};

#[cfg(target_os = "macos")]
mod sm {
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Bool};
    use objc2::msg_send;
    use std::ptr;

    #[link(name = "ServiceManagement", kind = "framework")]
    unsafe extern "C" {}

    fn service() -> Option<Retained<AnyObject>> {
        let cls = AnyClass::get(c"SMAppService")?;
        unsafe { msg_send![cls, mainAppService] }
    }

    pub fn status() -> isize {
        match service() {
            Some(s) => unsafe { msg_send![&*s, status] },
            None => -1,
        }
    }

    pub fn set(on: bool) -> Result<(), String> {
        let s = service().ok_or("ServiceManagement isn't available (macOS 13+)")?;
        let mut error: *mut AnyObject = ptr::null_mut();
        let ok: Bool = unsafe {
            if on {
                msg_send![&*s, registerAndReturnError: &mut error]
            } else {
                msg_send![&*s, unregisterAndReturnError: &mut error]
            }
        };
        if ok.as_bool() {
            return Ok(());
        }
        if error.is_null() {
            return Err("macOS refused".into());
        }
        let text: Option<Retained<objc2_foundation::NSString>> =
            unsafe { msg_send![&*error, localizedDescription] };
        Err(text.map(|t| t.to_string()).unwrap_or_else(|| "macOS refused".into()))
    }
}

pub struct AppLoginItem;

impl LoginItem for AppLoginItem {
    fn state(&self) -> LoginState {
        #[cfg(target_os = "macos")]
        return LoginState::from_status(sm::status());
        #[cfg(not(target_os = "macos"))]
        return LoginState::Unavailable;
    }

    fn set(&self, on: bool) -> Result<LoginState, String> {
        #[cfg(target_os = "macos")]
        {
            sm::set(on)?;
            Ok(self.state())
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = on;
            Err("start at login needs macOS".into())
        }
    }
}
