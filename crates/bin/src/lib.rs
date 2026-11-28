//! Ranki's native application library and mobile entry points.

#[cfg(any(feature = "mobile", target_os = "android", target_os = "ios"))]
pub mod mobile;
