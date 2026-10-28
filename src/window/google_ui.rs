//! Google Photos in the window: signing in and out, the sign-in the keyring
//! holds, what an upload says about a picture, and the albums made for rolls.

use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use adw::prelude::*;
use adw::subclass::prelude::ObjectSubclassIsExt;
use nv_export::ExportSettings;
use serde::{Deserialize, Serialize};

use super::Window;
use crate::export_queue::UploadSpec;

/// How long a sign-in waits for the person to finish in the browser.
const SIGN_IN_WAIT: Duration = Duration::from_secs(300);

/// `google-photos.json` in the roll folder: the album made for the roll.
#[derive(Default, Serialize, Deserialize)]
struct Albums {
    #[serde(default)]
    albums: BTreeMap<String, String>,
}

enum Msg {
    Open(String),
    Done(Result<nv_gphotos::Tokens, nv_gphotos::Error>),
}

impl Window {
    pub fn google_signed_in(&self) -> bool {
        self.imp().google_token.borrow().is_some()
    }

    /// Ask the keyring for the saved sign-in (once), then call `then`. The keyring can
    /// ask the person to unlock it, so this is not done until Google Photos is wanted.
    pub(super) fn with_google_token(&self, then: impl FnOnce(&Window) + 'static) {
        if self.imp().google_token_checked.get() {
            then(self);
            return;
        }
        self.imp().google_token_checked.set(true);
        let (tx, rx) = async_channel::bounded::<Option<String>>(1);
        std::thread::spawn(move || {
            let _ = tx.send_blocking(crate::google::load_refresh_token());
        });
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let token = rx.recv().await.ok().flatten();
            if let Some(win) = weak.upgrade() {
                if token.is_some() {
                    *win.imp().google_token.borrow_mut() = token;
                }
                then(&win);
            }
        });
    }

    /// Open the browser on Google's consent page and wait for the person to come back.
    /// `done` is told how it went: `Ok`, or the reason (empty when they cancelled).
    pub(super) fn google_sign_in(&self, done: Rc<dyn Fn(Result<(), String>)>) {
        let cfg = crate::google::oauth_config(&self.imp().prefs.borrow());
        if !cfg.is_set() {
            done(Err("This copy of Negative Vibes has no Google client. Add one under Google Photos in Preferences.".into()));
            return;
        }
        if self.imp().google_signin_cancel.borrow().is_some() {
            return; // one at a time
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *self.imp().google_signin_cancel.borrow_mut() = Some(cancel.clone());

        let (tx, rx) = async_channel::unbounded::<Msg>();
        std::thread::spawn(move || {
            let open_tx = tx.clone();
            let result = nv_gphotos::sign_in(&nv_gphotos::Endpoints::google(), &cfg, &move |url| drop(open_tx.send_blocking(Msg::Open(url.to_owned()))), &cancel, SIGN_IN_WAIT);
            let _ = tx.send_blocking(Msg::Done(result));
        });

        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            while let Ok(msg) = rx.recv().await {
                let Some(win) = weak.upgrade() else { return };
                match msg {
                    Msg::Open(url) => gtk::UriLauncher::new(&url).launch(Some(&win), None::<&gio::Cancellable>, |r| {
                        if let Err(e) = r {
                            eprintln!("nv: could not open the browser: {e}");
                        }
                    }),
                    Msg::Done(result) => {
                        *win.imp().google_signin_cancel.borrow_mut() = None;
                        match result {
                            Ok(tokens) => {
                                let token = tokens.refresh_token;
                                *win.imp().google_token.borrow_mut() = Some(token.clone());
                                win.imp().google_token_checked.set(true);
                                let (tx, rx) = async_channel::bounded::<Result<(), String>>(1);
                                std::thread::spawn(move || {
                                    let _ = tx.send_blocking(crate::google::save_refresh_token(&token));
                                });
                                let weak = win.downgrade();
                                glib::MainContext::default().spawn_local(async move {
                                    if let (Ok(Err(e)), Some(win)) = (rx.recv().await, weak.upgrade()) {
                                        eprintln!("nv: could not keep the Google sign-in: {e}");
                                        win.show_toast("Signed in, but the sign-in could not be saved: you will be asked again next time");
                                    }
                                });
                                done(Ok(()));
                            }
                            Err(nv_gphotos::Error::Cancelled) => done(Err(String::new())),
                            Err(e) => done(Err(e.to_string())),
                        }
                        return;
                    }
                }
            }
        });
    }

    /// Stop a sign-in that is waiting for the browser.
    pub(super) fn google_cancel_sign_in(&self) {
        if let Some(c) = self.imp().google_signin_cancel.borrow().as_ref() {
            c.store(true, Ordering::Relaxed);
        }
    }

    pub(super) fn google_signing_in(&self) -> bool {
        self.imp().google_signin_cancel.borrow().is_some()
    }

    /// Forget the sign-in here and tell Google the app no longer needs access.
    pub(super) fn google_sign_out(&self) {
        let cfg = crate::google::oauth_config(&self.imp().prefs.borrow());
        let token = self.imp().google_token.borrow_mut().take();
        self.imp().google_token_checked.set(true);
        std::thread::spawn(move || {
            crate::google::delete_refresh_token();
            if let Some(t) = token {
                let tokens = nv_gphotos::Tokens { refresh_token: t, access_token: None, expires_at: None };
                nv_gphotos::Client::new(nv_gphotos::Endpoints::google(), cfg, tokens).revoke();
            }
        });
    }

    /// Google no longer accepts the saved sign-in.
    pub(super) fn google_forget_token(&self) {
        *self.imp().google_token.borrow_mut() = None;
        std::thread::spawn(crate::google::delete_refresh_token);
    }

    // ── What to upload ──────────────────────────────────────────────────

    /// How to send frame `idx`'s export, or `None` when that is not wanted or not possible
    /// (not signed in, no client, a rejected frame).
    pub(super) fn upload_spec(&self, idx: u32, settings: &ExportSettings) -> Option<UploadSpec> {
        if !settings.google_photos {
            return None;
        }
        let token = self.imp().google_token.borrow().clone()?;
        let cfg = crate::google::oauth_config(&self.imp().prefs.borrow());
        if !cfg.is_set() {
            return None;
        }
        let rejected = self.imp().roll.borrow().as_ref().and_then(|r| r.frame(idx)).is_some_and(|f| f.rating == Some(-1));
        if rejected {
            return None;
        }
        // A name set in the export settings is the one album for every roll; otherwise the roll's own
        let title = match settings.google_album_name.trim() {
            "" => self.imp().roll.borrow().as_ref().map(|r| r.name.clone()).unwrap_or_default(),
            fixed => fixed.to_owned(),
        };
        let album = (settings.google_album && !title.trim().is_empty()).then(|| {
            let known = self.known_album(&title);
            (title, known)
        });
        Some(UploadSpec { cfg, refresh_token: token, description: self.upload_description(idx, settings), album })
    }

    /// The same spec with the current sign-in, for trying an upload again.
    pub(super) fn refreshed_spec(&self, mut spec: UploadSpec) -> Option<UploadSpec> {
        spec.refresh_token = self.imp().google_token.borrow().clone()?;
        Some(spec)
    }

    /// What Google Photos shows under the picture: what the person wrote, then the film and place in the roll.
    fn upload_description(&self, idx: u32, settings: &ExportSettings) -> String {
        let (meta, info) = self.frame_metadata(idx, settings);
        let first = [meta.caption.trim(), meta.title.trim(), info.label.trim()].into_iter().find(|s| !s.is_empty());
        let film = [meta.film_stock.trim(), info.film_profile.trim()].into_iter().find(|s| !s.is_empty());
        let mut context = Vec::new();
        context.extend(film.map(str::to_owned));
        if !info.roll.trim().is_empty() {
            context.push(info.roll.trim().to_owned());
        }
        context.extend(info.frame_number.map(|n| format!("frame {n}")));
        let mut lines: Vec<String> = first.map(str::to_owned).into_iter().collect();
        if !context.is_empty() {
            lines.push(context.join(" · "));
        }
        lines.join("\n")
    }

    // ── Albums ──────────────────────────────────────────────────────────

    fn albums_file(&self) -> Option<std::path::PathBuf> {
        self.imp().roll_root.borrow().as_ref().map(|r| r.join("google-photos.json"))
    }

    fn known_album(&self, title: &str) -> Option<String> {
        let text = std::fs::read_to_string(self.albums_file()?).ok()?;
        serde_json::from_str::<Albums>(&text).ok()?.albums.get(title).cloned()
    }

    /// Keep the id of the album made for the roll, so the next export goes into it.
    pub(super) fn remember_album(&self, title: &str, id: &str) {
        let Some(path) = self.albums_file() else { return };
        let mut albums: Albums = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        albums.albums.insert(title.to_owned(), id.to_owned());
        if let Ok(text) = serde_json::to_string_pretty(&albums)
            && let Err(e) = std::fs::write(&path, text)
        {
            eprintln!("nv: could not remember the Google Photos album: {e}");
        }
    }
}
