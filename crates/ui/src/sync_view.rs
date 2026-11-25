//! Optional `AnkiWeb` sign-in shown in the settings screen.

use std::path::Path;
use std::time::Duration;

use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, div, px,
};

use ranki_sync::{
    Credentials, HttpTransport, RetryPolicy, SyncClient, SyncState,
    TransportConfig, state_path,
};

use crate::{
    app::{Root, status_label},
    components::{icon, small_button},
    theme::Palette,
};

/// Reads the local sync state without changing it.
pub fn read_state(collection: &Path) -> Option<SyncState> {
    SyncState::load(&state_path(collection)).ok()
}

impl Root {
    pub(crate) fn set_sync_enabled(
        &mut self,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        self.options.sync.enabled = enabled;
        if let Err(error) = (self.save_settings)(&self.options) {
            self.show_toast(
                crate::app::ToastKind::Error,
                format!("Could not save sync setting: {error}"),
                cx,
            );
        }
        cx.notify();
    }

    pub(crate) fn sign_in(&mut self, cx: &mut Context<Self>) {
        if self.sync.busy {
            return;
        }
        let username = self.sync.username.read(cx).text().trim().to_owned();
        let password = self.sync.password.read(cx).text();
        if username.is_empty() || password.is_empty() {
            self.show_toast(
                crate::app::ToastKind::Error,
                "Enter your AnkiWeb username and password",
                cx,
            );
            return;
        }
        self.sync.busy = true;
        cx.notify();

        let collection = self.options.collection.clone();
        let media_dir = self.options.media_dir.clone();
        let retries = self.options.sync.retries;
        let backoff_ms = self.options.sync.backoff_ms;
        let task = cx.background_executor().spawn(async move {
            let transport = HttpTransport::with_config(
                ranki_sync::DEFAULT_ENDPOINT,
                TransportConfig {
                    retry_policy: RetryPolicy {
                        retries,
                        backoff: Duration::from_millis(backoff_ms),
                    },
                    ..TransportConfig::default()
                },
            );
            let mut client = SyncClient::new(
                transport,
                &collection,
                &media_dir,
                &state_path(&collection),
            )
            .map_err(|error| error.to_string())?;
            client
                .login(&Credentials::new(username.clone(), password))
                .map_err(|error| error.to_string())?;
            Ok::<(String, Option<i64>), String>((
                username,
                client.state().last_success,
            ))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |root, cx| {
                root.sync.busy = false;
                match result {
                    Ok((username, last_sync)) => {
                        root.sync.signed_in = true;
                        root.last_sync = last_sync;
                        root.options.sync.enabled = true;
                        root.options.sync.username = Some(username);
                        root.sync.password.update(cx, |field, cx| {
                            field.clear(cx);
                        });
                        if let Err(error) = (root.save_settings)(&root.options)
                        {
                            root.show_toast(
                                crate::app::ToastKind::Error,
                                format!(
                                    "Signed in, but could not save settings: {error}"
                                ),
                                cx,
                            );
                        } else {
                            root.show_toast(
                                crate::app::ToastKind::Success,
                                "Signed in. AnkiWeb sync runs at the next launch",
                                cx,
                            );
                        }
                    }
                    Err(error) => root.show_toast(
                        crate::app::ToastKind::Error,
                        format!("AnkiWeb sign-in failed: {error}"),
                        cx,
                    ),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn render_sync(&self) -> impl IntoElement {
        let colors = self.colors();
        let enabled = self.options.sync.enabled;
        let signed_in = self.sync.signed_in;
        let status = if self.sync.busy {
            String::from("Signing in with AnkiWeb")
        } else if signed_in {
            format!(
                "Signed in{}",
                self.options
                    .sync
                    .username
                    .as_deref()
                    .map_or_else(String::new, |name| format!(" as {name}"))
            )
        } else {
            status_label(self.last_sync, Self::now())
        };
        let note = if enabled {
            "AnkiWeb sync is on. Passwords are used once and never stored."
        } else {
            "Optional. Sign in only if you want to sync with AnkiWeb."
        };
        let toggle = self.action_click(crate::app::UiAction::ToggleSync);
        let sign_in = self.action_click(crate::app::UiAction::SyncSignIn);

        let mut fields = div().flex().flex_col().gap(px(12.));
        if enabled && !signed_in {
            fields = fields
                .child(labeled_input(
                    "Username",
                    self.sync.username.clone(),
                    colors,
                ))
                .child(labeled_input(
                    "Password",
                    self.sync.password.clone(),
                    colors,
                ))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .child(small_button(
                            if self.sync.busy {
                                "Signing in"
                            } else {
                                "Sign in"
                            },
                            self.style(),
                            sign_in,
                        ))
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(colors.subtext)
                                .child("Sync starts on the next launch"),
                        ),
                );
        }

        div()
            .id("settings-sync")
            .flex()
            .flex_col()
            .gap(px(18.))
            .p(px(24.))
            .rounded(px(24.))
            .bg(colors.mantle)
            .border_1()
            .border_color(colors.surface0)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(icon("icon-clock", 18., colors.mauve))
                    .child(
                        div()
                            .text_size(px(18.))
                            .font_weight(gpui::FontWeight::BOLD)
                            .child("AnkiWeb sync"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(24.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child("Sync collection"),
                            )
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(colors.subtext)
                                    .child(note),
                            ),
                    )
                    .child(small_button(
                        if enabled { "Turn off" } else { "Turn on" },
                        self.style(),
                        toggle,
                    )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(12.))
                    .text_color(colors.subtext)
                    .child(icon("icon-check", 14., colors.green))
                    .child(SharedString::from(status)),
            )
            .child(fields)
    }
}

fn labeled_input(
    label: &'static str,
    field: gpui::Entity<crate::text_field::TextField>,
    colors: Palette,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            div()
                .text_size(px(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.subtext)
                .child(label),
        )
        .child(
            div()
                .flex()
                .px(px(14.))
                .py(px(10.))
                .rounded(px(12.))
                .border_1()
                .border_color(colors.surface1)
                .bg(colors.base)
                .text_size(px(14.))
                .child(field),
        )
}

#[cfg(test)]
mod tests {
    use ranki_sync::{SyncState, state_path};

    use super::read_state;

    #[test]
    fn footer_reads_the_last_sync_from_the_state_file() {
        let Ok(directory) = tempfile::tempdir() else {
            return;
        };
        let collection = directory.path().join("collection.anki2");
        let state = SyncState {
            host_key: String::from("stored-key"),
            last_success: Some(1_700_000_000),
            ..SyncState::default()
        };
        let Ok(text) = serde_json::to_string(&state) else {
            return;
        };
        if std::fs::write(state_path(&collection), text).is_err() {
            return;
        }
        let loaded = read_state(&collection).unwrap_or_default();
        assert_eq!(loaded.host_key, "stored-key");
        assert_eq!(loaded.last_success, Some(1_700_000_000));
    }
}
