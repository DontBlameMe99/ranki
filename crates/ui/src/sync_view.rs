//! Optional `AnkiWeb` sign-in shown in the settings screen.

use std::path::Path;
use std::time::Duration;

use gpui::prelude::FluentBuilder;
use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, div, px,
};
use ranki_storage::Collection;

use ranki_sync::{
    Credentials, HttpTransport, RetryPolicy, SyncClient, SyncOutcome,
    SyncState, SyncTransport, TransportConfig, state_path,
};

use crate::{
    app::{Root, ToastKind},
    components::{icon, small_button},
    theme::{Palette, SPACE_BLOCK, SPACE_GROUP, SPACE_RELATED, SPACE_SECTION},
};

/// Reads the local sync state without changing it.
pub fn read_state(collection: &Path) -> Option<SyncState> {
    SyncState::load(&state_path(collection)).ok()
}
/// Exchanges the password for a host key, then runs one sync immediately.
fn login_and_sync<T: SyncTransport>(
    client: &mut SyncClient<T>,
    username: String,
    password: String,
) -> Result<(String, Result<SyncOutcome, String>), String> {
    client
        .login(&Credentials::new(username.clone(), password))
        .map_err(|error| error.to_string())?;
    Ok((
        username,
        client.sync(None).map_err(|error| error.to_string()),
    ))
}

fn sync_message(outcome: &SyncOutcome) -> (ToastKind, String) {
    match outcome {
        SyncOutcome::Disabled => (
            ToastKind::Error,
            String::from("Signed in, but sync did not start"),
        ),
        SyncOutcome::NoChanges => (
            ToastKind::Success,
            String::from("Signed in. AnkiWeb is up to date"),
        ),
        SyncOutcome::Uploaded => (
            ToastKind::Success,
            String::from("Signed in. Collection uploaded"),
        ),
        SyncOutcome::Downloaded => (
            ToastKind::Success,
            String::from("Signed in. Collection downloaded"),
        ),
        SyncOutcome::Conflict { reason } => (
            ToastKind::Error,
            format!("Signed in. Sync needs a manual choice: {reason}"),
        ),
        SyncOutcome::RetryLater { reason } => (
            ToastKind::Error,
            format!(
                "Signed in. Sync failed: {reason}. Run ranki sync to retry"
            ),
        ),
    }
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
            login_and_sync(&mut client, username, password)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |root, cx| {
                root.sync.busy = false;
                match result {
                    Ok((username, sync)) => {
                        root.sync.signed_in = true;
                        root.options.sync.enabled = true;
                        root.options.sync.username = Some(username);
                        root.sync.password.update(cx, |field, cx| {
                            field.clear(cx);
                        });
                        let downloaded =
                            matches!(&sync, Ok(SyncOutcome::Downloaded));
                        let (kind, message) = match sync {
                            Ok(outcome) => sync_message(&outcome),
                            Err(error) => (
                                ToastKind::Error,
                                format!("Signed in, but sync failed: {error}"),
                            ),
                        };
                        if let Err(error) = (root.save_settings)(&root.options)
                        {
                            root.show_toast(
                                ToastKind::Error,
                                format!(
                                    "{message} Could not save settings: {error}"
                                ),
                                cx,
                            );
                        } else {
                            root.show_toast(kind, message, cx);
                        }
                        if downloaded {
                            match Collection::open(&root.options.collection) {
                                Ok(collection) => {
                                    root.store = Box::new(collection);
                                    root.reload_collection(cx);
                                }
                                Err(error) => root.show_toast(
                                    ToastKind::Error,
                                    format!(
                                        "Downloaded the collection, but could not reopen it: {error}"
                                    ),
                                    cx,
                                ),
                            }
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
            Some(String::from("Signing in with AnkiWeb"))
        } else if signed_in {
            Some(format!(
                "Signed in{}",
                self.options
                    .sync
                    .username
                    .as_deref()
                    .map_or_else(String::new, |name| format!(" as {name}"))
            ))
        } else {
            None
        };
        let note = if enabled {
            "AnkiWeb sync is on. Passwords are used once and never stored."
        } else {
            "Optional. Sign in only if you want to sync with AnkiWeb."
        };
        let toggle = self.action_click(crate::app::UiAction::ToggleSync);
        let sign_in = self.action_click(crate::app::UiAction::SyncSignIn);

        let mut fields = div().flex().flex_col().gap(px(SPACE_RELATED));
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
                    div().flex().items_center().gap(px(SPACE_RELATED)).child(
                        small_button(
                            if self.sync.busy {
                                "Signing in"
                            } else {
                                "Sign in"
                            },
                            self.style(),
                            sign_in,
                        ),
                    ),
                );
        }

        div()
            .id("settings-sync")
            .flex()
            .flex_col()
            .gap(px(SPACE_BLOCK))
            .p(px(SPACE_SECTION))
            .rounded(px(24.))
            .bg(colors.mantle)
            .border_1()
            .border_color(colors.surface0)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(SPACE_RELATED))
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
                    .gap(px(SPACE_SECTION))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(SPACE_GROUP))
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
            .when_some(status, |this, status| {
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(SPACE_GROUP))
                        .text_size(px(12.))
                        .text_color(colors.subtext)
                        .child(icon("icon-check", 14., colors.green))
                        .child(SharedString::from(status)),
                )
            })
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
        .gap(px(SPACE_GROUP))
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
                .px(px(SPACE_RELATED))
                .py(px(SPACE_RELATED))
                .rounded(px(12.))
                .border_1()
                .border_color(colors.surface1)
                .bg(colors.base)
                .text_size(px(14.))
                .child(div().flex_1().child(field)),
        )
}

#[cfg(test)]
mod tests {
    use ranki_sync::{
        MockResponse, MockTransport, SyncClient, SyncMethod, SyncOutcome,
        SyncState, state_path,
    };
    use serde_json::json;

    use super::{login_and_sync, read_state, sync_message};
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
    #[test]
    fn sign_in_runs_a_sync_immediately() {
        let Ok(directory) = tempfile::tempdir() else {
            return;
        };
        let collection = directory.path().join("collection.anki2");
        if ranki_storage::Collection::create(&collection).is_err() {
            return;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let transport = MockTransport::new()
            .with_response(
                SyncMethod::HostKey,
                MockResponse::ok_json(&json!({ "key": "host-key" })).unwrap(),
            )
            .with_response(
                SyncMethod::Meta,
                MockResponse::ok_json(&json!({
                    "mod": 1,
                    "scm": 1,
                    "usn": 1,
                    "ts": now,
                    "empty": true,
                    "cont": true,
                    "media_usn": 1,
                }))
                .unwrap(),
            );
        let state_path = state_path(&collection);
        let Ok(mut client) = SyncClient::new(
            transport,
            &collection,
            directory.path(),
            &state_path,
        ) else {
            return;
        };

        let password = directory
            .path()
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .chars()
            .rev()
            .collect();
        let result =
            login_and_sync(&mut client, String::from("user"), password);

        let Ok((username, outcome)) = result else {
            return;
        };
        assert_eq!(username, "user");
        assert!(matches!(outcome, Ok(SyncOutcome::Conflict { .. })));
        let requests = client.transport().requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].method, SyncMethod::HostKey);
        assert_eq!(requests[1].method, SyncMethod::Meta);
    }

    #[test]
    fn sync_outcome_messages_name_the_result() {
        let (_, uploaded) = sync_message(&SyncOutcome::Uploaded);
        assert_eq!(uploaded, "Signed in. Collection uploaded");
        let (_, downloaded) = sync_message(&SyncOutcome::Downloaded);
        assert_eq!(downloaded, "Signed in. Collection downloaded");
    }
}
