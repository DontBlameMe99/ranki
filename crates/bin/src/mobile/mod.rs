//! Ranki's touch renderer and native mobile entry points.

#![cfg_attr(
    any(target_os = "android", target_os = "ios"),
    expect(
        unsafe_code,
        reason = "mobile entry points are C ABI symbols the platform calls directly"
    )
)]

pub mod card;
pub mod error;
pub mod platform;
pub mod state;
pub mod ui;

#[cfg(test)]
mod tests;

pub use error::{MobileError, Result};
pub use state::AppState;

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub fn android_main(app: winit::platform::android::activity::AndroidApp) {
    if let Err(error) = platform::run_with_android_app(app) {
        eprintln!("ranki mobile: {error}");
    }
}

#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
pub extern "C" fn start_ranki() {
    if let Err(error) = platform::run() {
        eprintln!("ranki mobile: {error}");
    }
}
