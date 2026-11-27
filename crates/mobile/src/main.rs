//! Native entry point for the mobile application.

#[cfg(not(target_os = "android"))]
use ranki_mobile::platform;

#[cfg(not(target_os = "android"))]
fn main() -> eframe::Result<()> {
    platform::run()
}

#[cfg(target_os = "android")]
fn main() {}
