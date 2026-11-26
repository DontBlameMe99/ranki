//! Optional `AnkiWeb` sign-in shown in the settings screen.

use std::future::Future;
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
    components::{ButtonTone, action_button, icon, panel},
    theme::{Palette, SPACE_GROUP, SPACE_RELATED, SPACE_SECTION},
};

/// Reads the local sync state without changing it.
pub fn read_state(collection: &Path) -> Option<SyncState> {
    SyncState::load(&state_path(collection)).ok()
}

fn sync_status(busy: bool) -> Option<&'static str> {
    busy.then_some("Syncing with AnkiWeb")
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

fn refresh_message(outcome: &SyncOutcome) -> (ToastKind, String) {
    match outcome {
        SyncOutcome::Disabled => {
            (ToastKind::Error, String::from("Sync did not start"))
        }
        SyncOutcome::NoChanges => {
            (ToastKind::Success, String::from("Already up to date"))
        }
        SyncOutcome::Uploaded => {
            (ToastKind::Success, String::from("Collection uploaded"))
        }
        SyncOutcome::Downloaded => {
            (ToastKind::Success, String::from("Collection downloaded"))
        }
        SyncOutcome::Conflict { reason } => (
            ToastKind::Error,
            format!("Sync needs a manual choice: {reason}"),
        ),
        SyncOutcome::RetryLater { reason } => (
            ToastKind::Error,
            format!("Sync failed: {reason}. Run ranki sync to retry"),
        ),
    }
}

fn retained_conflict(
    result: &Result<SyncOutcome, String>,
    fallback: Option<String>,
) -> Option<String> {
    if result.is_err() || matches!(result, Ok(SyncOutcome::RetryLater { .. })) {
        fallback
    } else {
        None
    }
}

fn build_client(
    collection: &Path,
    media_dir: &Path,
    retries: u8,
    backoff_ms: u64,
) -> Result<SyncClient<HttpTransport>, String> {
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
    SyncClient::new(transport, collection, media_dir, &state_path(collection))
        .map_err(|error| error.to_string())
}

impl Root {
    pub(crate) fn sign_out(&mut self, cx: &mut Context<Self>) {
        if self.sync.busy {
            return;
        }
        if let Err(error) = ranki_sync::clear_host_key(&self.options.collection)
        {
            self.show_toast(
                ToastKind::Error,
                format!("Could not sign out: {error}"),
                cx,
            );
            return;
        }
        self.sync.signed_in = false;
        self.options.sync.enabled = false;
        self.options.sync.username = None;
        self.sync
            .username
            .update(cx, crate::text_field::TextField::clear);
        self.sync
            .password
            .update(cx, crate::text_field::TextField::clear);
        if let Err(error) = (self.save_settings)(&self.options) {
            self.show_toast(
                ToastKind::Error,
                format!("Signed out, but could not save settings: {error}"),
                cx,
            );
        } else {
            self.show_toast(
                ToastKind::Success,
                String::from("Signed out of AnkiWeb"),
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
                ToastKind::Error,
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
            let mut client =
                build_client(&collection, &media_dir, retries, backoff_ms)?;
            login_and_sync(&mut client, username, password)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |root, cx| {
                match result {
                    Ok((username, sync)) => {
                        root.sync.signed_in = true;
                        root.options.sync.enabled = true;
                        root.options.sync.username = Some(username);
                        root.sync.password.update(cx, |field, cx| {
                            field.clear(cx);
                        });
                        root.finish_sync(sync, true, cx);
                        if let Err(error) = (root.save_settings)(&root.options)
                        {
                            root.show_toast(
                                ToastKind::Error,
                                format!("Could not save sync setting: {error}"),
                                cx,
                            );
                        }
                    }
                    Err(error) => {
                        root.sync.busy = false;
                        root.show_toast(
                            ToastKind::Error,
                            format!("AnkiWeb sign-in failed: {error}"),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn sync_now(&mut self, cx: &mut Context<Self>) {
        if self.sync.busy || !self.sync.signed_in {
            return;
        }
        let reason = self.sync.conflict.clone();
        self.sync.busy = true;
        cx.notify();

        let collection = self.options.collection.clone();
        let media_dir = self.options.media_dir.clone();
        let retries = self.options.sync.retries;
        let backoff_ms = self.options.sync.backoff_ms;
        let task = cx.background_executor().spawn(async move {
            let mut client =
                build_client(&collection, &media_dir, retries, backoff_ms)?;
            client.sync(None).map_err(|error| error.to_string())
        });
        Self::spawn_sync_task(task, false, reason, cx);
    }

    pub(crate) fn resolve_sync_conflict(
        &mut self,
        keep_local: bool,
        cx: &mut Context<Self>,
    ) {
        if self.sync.busy {
            return;
        }
        let reason = self.sync.conflict.clone();
        self.sync.busy = true;
        cx.notify();

        let collection = self.options.collection.clone();
        let media_dir = self.options.media_dir.clone();
        let retries = self.options.sync.retries;
        let backoff_ms = self.options.sync.backoff_ms;
        let task = cx.background_executor().spawn(async move {
            let mut client =
                build_client(&collection, &media_dir, retries, backoff_ms)?;
            if keep_local {
                client
                    .full_upload()
                    .map(|()| SyncOutcome::Uploaded)
                    .map_err(|error| error.to_string())
            } else {
                client
                    .full_download()
                    .map(|()| SyncOutcome::Downloaded)
                    .map_err(|error| error.to_string())
            }
        });
        Self::spawn_sync_task(task, false, reason, cx);
    }

    fn spawn_sync_task(
        task: impl Future<Output = Result<SyncOutcome, String>> + Send + 'static,
        signed_in: bool,
        fallback_conflict: Option<String>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |root, cx| {
                let remaining_conflict =
                    retained_conflict(&result, fallback_conflict);
                root.finish_sync(result, signed_in, cx);
                if remaining_conflict.is_some() {
                    root.sync.conflict = remaining_conflict;
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn finish_sync(
        &mut self,
        result: Result<SyncOutcome, String>,
        signed_in: bool,
        cx: &mut Context<Self>,
    ) {
        self.sync.busy = false;
        self.sync.conflict = None;
        let (kind, message) = match &result {
            Ok(outcome) => {
                if let SyncOutcome::Conflict { reason } = outcome {
                    self.sync.conflict = Some(reason.clone());
                }
                if signed_in {
                    sync_message(outcome)
                } else {
                    refresh_message(outcome)
                }
            }
            Err(error) => (
                ToastKind::Error,
                if signed_in {
                    format!("Signed in, but sync failed: {error}")
                } else {
                    format!("Sync failed: {error}")
                },
            ),
        };
        self.show_toast(kind, message, cx);
        if matches!(result, Ok(SyncOutcome::Downloaded)) {
            self.reopen_collection(cx);
        }
    }

    fn reopen_collection(&mut self, cx: &mut Context<Self>) {
        match Collection::open(&self.options.collection) {
            Ok(collection) => {
                self.store = Box::new(collection);
                self.reload_collection(cx);
            }
            Err(error) => self.show_toast(
                ToastKind::Error,
                format!("Downloaded the collection, but could not reopen it: {error}"),
                cx,
            ),
        }
    }

    pub(crate) fn render_sync(&self) -> impl IntoElement {
        let colors = self.colors();
        let enabled = self.options.sync.enabled;
        let signed_in = self.sync.signed_in;
        let status = sync_status(self.sync.busy);
        let note = if !signed_in {
            "Optional. Sign in only if you want to sync with AnkiWeb."
        } else if enabled {
            "You are currently using AnkiWeb"
        } else {
            "Sync is off. Sign out and sign in again to resume."
        };
        let sign_in = self.action_click(crate::app::UiAction::SyncSignIn);
        let sign_out = self.action_click(crate::app::UiAction::SyncSignOut);
        let sync_now = self.action_click(crate::app::UiAction::SyncNow);
        let keep_local = self.action_click(crate::app::UiAction::SyncKeepLocal);
        let keep_remote =
            self.action_click(crate::app::UiAction::SyncKeepRemote);

        let mut fields = div().flex().flex_col().gap(px(SPACE_RELATED));
        if enabled && signed_in {
            fields = fields.child(action_button(
                "sync-now",
                if self.sync.busy {
                    "Syncing"
                } else {
                    "Sync now"
                },
                ButtonTone::Neutral,
                self.style(),
                sync_now,
            ));
        }
        if !signed_in {
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
                        action_button(
                            "sync-sign-in",
                            if self.sync.busy {
                                "Signing in"
                            } else {
                                "Sign in"
                            },
                            ButtonTone::Primary,
                            self.style(),
                            sign_in,
                        ),
                    ),
                );
        }

        panel(colors)
            .id("settings-sync")
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
                    .when(signed_in, |this| {
                        this.child(action_button(
                            "sync-sign-out",
                            "Sign out",
                            ButtonTone::Neutral,
                            self.style(),
                            sign_out,
                        ))
                    }),
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
            .when_some(self.sync.conflict.clone(), |this, reason| {
                this.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(SPACE_RELATED))
                        .child(
                            div()
                                .text_size(px(14.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.red)
                                .child("Sync conflict"),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(colors.subtext)
                                .child(reason),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(colors.subtext)
                                .child("Choose which copy to keep. The other one is replaced."),
                        )
                        .child(
                            div()
                                .flex()
                                .gap(px(SPACE_RELATED))
                                .child(action_button(
                                    "sync-keep-remote",
                                    "Keep AnkiWeb copy",
                                    ButtonTone::Primary,
                                    self.style(),
                                    keep_remote,
                                ))
                                .child(action_button(
                                    "sync-keep-local",
                                    "Keep local copy",
                                    ButtonTone::Danger,
                                    self.style(),
                                    keep_local,
                                )),
                        ),
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
        SyncState, clear_host_key, state_path,
    };
    use serde_json::json;

    use super::{
        login_and_sync, read_state, refresh_message, retained_conflict,
        sync_message, sync_status,
    };
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
        assert_eq!(outcome, Ok(SyncOutcome::NoChanges));
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

    #[test]
    fn refresh_conflict_asks_for_a_choice() {
        let (kind, text) = refresh_message(&SyncOutcome::Conflict {
            reason: String::from("both collections changed"),
        });
        assert_eq!(kind, crate::app::ToastKind::Error);
        assert_eq!(
            text,
            "Sync needs a manual choice: both collections changed"
        );
    }

    #[test]
    fn unresolved_conflict_is_retained_after_failed_sync() {
        let fallback = Some(String::from("both collections changed"));
        let downloaded = Ok(SyncOutcome::Downloaded);
        assert_eq!(retained_conflict(&downloaded, fallback.clone()), None);

        let uploaded = Ok(SyncOutcome::Uploaded);
        assert_eq!(retained_conflict(&uploaded, fallback.clone()), None);

        let retried = Ok(SyncOutcome::RetryLater {
            reason: String::from("offline"),
        });
        assert_eq!(
            retained_conflict(&retried, fallback.clone()).as_deref(),
            Some("both collections changed")
        );

        let failed = Err(String::from("server rejected the collection"));
        assert_eq!(
            retained_conflict(&failed, fallback).as_deref(),
            Some("both collections changed")
        );
    }

    #[test]
    fn idle_status_is_hidden() {
        assert_eq!(sync_status(false), None);
        assert_eq!(sync_status(true), Some("Syncing with AnkiWeb"));
    }

    #[test]
    fn sign_out_clears_the_cached_host_key() {
        let Ok(directory) = tempfile::tempdir() else {
            return;
        };
        let collection = directory.path().join("collection.anki2");
        let path = state_path(&collection);
        let state = SyncState {
            host_key: String::from("stored-key"),
            last_mod: Some(7),
            last_scm: Some(8),
            ..SyncState::default()
        };
        let Ok(text) = serde_json::to_string(&state) else {
            return;
        };
        assert!(std::fs::write(&path, text).is_ok());
        assert!(clear_host_key(&collection).is_ok());
        let saved = read_state(&collection).unwrap_or_default();
        assert_eq!(saved.host_key, "");
        assert_eq!(saved.last_mod, Some(7));
        assert_eq!(saved.last_scm, Some(8));
        assert!(clear_host_key(&collection).is_ok());
    }
}
