//! Exporting from the window: queueing jobs, the queue popover in the header,
//! and what happens when an export finishes.

use adw::prelude::*;
use adw::subclass::prelude::ObjectSubclassIsExt;
use nv_export::{ExportSettings, NameContext, render_template};
use nv_store::{ExportRecord, FrameState, Roll, RollStore};

use super::Window;
use crate::export_queue::{ExportEvent, ExportJob, ExportQueue, UploadSpec};

#[derive(Debug, Clone, PartialEq)]
pub enum JobState {
    Queued,
    Running,
    Done(std::path::PathBuf),
    Failed(String),
    Cancelled,
}

/// Where a picture's trip to Google Photos has got to.
#[derive(Debug, Clone, PartialEq)]
pub enum UploadState {
    /// Not going there.
    No,
    /// Waiting for the export to finish.
    Waiting,
    Sending,
    /// In the library; the link opens it, when Google gave one.
    Done(Option<String>),
    Failed {
        message: String,
        sign_in_needed: bool,
    },
}

impl UploadState {
    fn active(&self) -> bool {
        matches!(self, UploadState::Waiting | UploadState::Sending)
    }
}

#[derive(Debug, Clone)]
pub struct JobInfo {
    pub id: u64,
    pub frame: u32,
    pub label: String,
    pub state: JobState,
    pub fraction: f32,
    pub upload: UploadState,
    /// How the picture is sent, kept to try again.
    pub spec: Option<UploadSpec>,
}

fn today() -> String {
    glib::DateTime::now_local().ok().and_then(|d| d.format("%Y-%m-%d").ok()).map(|s| s.to_string()).unwrap_or_default()
}

fn now_iso() -> String {
    glib::DateTime::now_utc().ok().and_then(|d| d.format_iso8601().ok()).map(|s| s.to_string()).unwrap_or_default()
}

/// Folder names the roll's name can safely become.
fn folder_safe(name: &str) -> String {
    let s: String = name.chars().map(|c| if c == '/' || c == '\\' || c.is_control() { '_' } else { c }).collect();
    let s = s.trim().trim_start_matches('.').to_owned();
    if s.is_empty() { "Roll".into() } else { s }
}

impl Window {
    /// Start the export thread, listen to it, and register the actions.
    pub(super) fn setup_export(&self) {
        let queue = ExportQueue::spawn();
        let events = queue.events.clone();
        let _ = self.imp().export_queue.set(queue);

        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            while let Ok(ev) = events.recv().await {
                match weak.upgrade() {
                    Some(win) => win.handle_export_event(ev),
                    None => break,
                }
            }
        });

        let open = gio::SimpleAction::new("export", None);
        let weak = self.downgrade();
        open.connect_activate(move |_, _| {
            if let Some(win) = weak.upgrade() {
                win.open_export_dialog();
            }
        });
        self.add_action(&open);

        for (name, f) in [
            ("film-create", Window::open_film_profile_dialog as fn(&Window)),
            ("film-import", Window::import_film_profile),
            ("film-export", Window::export_film_profile),
            ("film-delete", Window::delete_film_profile),
        ] {
            let action = gio::SimpleAction::new(name, None);
            let weak = self.downgrade();
            action.connect_activate(move |_, _| {
                if let Some(win) = weak.upgrade() {
                    f(&win);
                }
            });
            self.add_action(&action);
        }

        let guided = gio::SimpleAction::new("guided", None);
        let weak = self.downgrade();
        guided.connect_activate(move |_, _| {
            if let Some(win) = weak.upgrade() {
                win.open_guided();
            }
        });
        self.add_action(&guided);

        let metadata = gio::SimpleAction::new("metadata", None);
        let weak = self.downgrade();
        metadata.connect_activate(move |_, _| {
            if let Some(win) = weak.upgrade() {
                win.open_metadata_dialog();
            }
        });
        self.add_action(&metadata);

        let calibrate = gio::SimpleAction::new("calibrate", None);
        let weak = self.downgrade();
        calibrate.connect_activate(move |_, _| {
            if let Some(win) = weak.upgrade() {
                win.open_calibrate_dialog();
            }
        });
        self.add_action(&calibrate);

        let show = gio::SimpleAction::new("show-in-files", Some(&String::static_variant_type()));
        let weak = self.downgrade();
        show.connect_activate(move |_, target| {
            let (Some(win), Some(path)) = (weak.upgrade(), target.and_then(|t| t.get::<String>())) else { return };
            gtk::FileLauncher::new(Some(&gio::File::for_path(path))).open_containing_folder(Some(&win), None::<&gio::Cancellable>, |_| {});
        });
        self.add_action(&show);

        let retry = gio::SimpleAction::new("retry-upload", Some(&u64::static_variant_type()));
        let weak = self.downgrade();
        retry.connect_activate(move |_, target| {
            if let (Some(win), Some(id)) = (weak.upgrade(), target.and_then(|t| t.get::<u64>())) {
                win.retry_upload(id);
            }
        });
        self.add_action(&retry);

        let sign_in = gio::SimpleAction::new("google-sign-in", None);
        let weak = self.downgrade();
        sign_in.connect_activate(move |_, _| {
            if let Some(win) = weak.upgrade() {
                let w = win.downgrade();
                win.google_sign_in(std::rc::Rc::new(move |r| {
                    if let Some(win) = w.upgrade() {
                        match r {
                            Ok(()) => win.show_toast("Signed in to Google Photos"),
                            Err(e) if e.is_empty() => {}
                            Err(e) => win.show_toast(&format!("Could not sign in: {e}")),
                        }
                    }
                }));
            }
        });
        self.add_action(&sign_in);

        let clear = gio::SimpleAction::new("clear-finished-exports", None);
        let weak = self.downgrade();
        clear.connect_activate(move |_, _| {
            if let Some(win) = weak.upgrade() {
                win.imp().export_jobs.borrow_mut().retain(|j| matches!(j.state, JobState::Queued | JobState::Running) || j.upload.active());
                win.refresh_queue_ui();
            }
        });
        self.add_action(&clear);
    }

    /// The roll's export settings (the defaults for a roll that has none yet).
    pub(super) fn export_settings(&self) -> ExportSettings {
        self.imp().roll.borrow().as_ref().map(|r| r.settings.export.clone()).unwrap_or_default()
    }

    fn roll_name(&self) -> String {
        self.imp().roll.borrow().as_ref().map(|r| r.name.clone()).unwrap_or_else(|| "Roll".into())
    }

    /// Where exports go: the chosen folder, else `~/Pictures/Scans/<roll>`.
    pub(super) fn export_folder(&self, settings: &ExportSettings) -> std::path::PathBuf {
        settings
            .folder
            .clone()
            .unwrap_or_else(|| glib::user_special_dir(glib::UserDirectory::Pictures).unwrap_or_else(glib::home_dir).join("Scans").join(folder_safe(&self.roll_name())))
    }

    /// The file name (no extension) `frame` would be exported under.
    pub(super) fn export_stem(&self, frame: u32, template: &str) -> Result<String, nv_export::NameError> {
        let roll = self.imp().roll.borrow();
        let f = roll.as_ref().and_then(|r| r.frame(frame));
        render_template(
            template,
            &NameContext { roll: self.roll_name(), frame, label: f.and_then(|f| f.label.clone()), film: roll.as_ref().and_then(|r| r.film_stock.clone()), date: today() },
        )
    }

    /// What a DNG of frame `idx` says about its picture and scan.
    pub(super) fn frame_dng_info(&self, idx: u32, settings: &ExportSettings) -> Option<nv_export::dng::DngInfo> {
        if settings.format != nv_export::ExportFormat::Dng {
            return None;
        }
        let (meta, info) = self.frame_metadata(idx, settings);
        let iso = Some(self.effective_params(idx).profile.iso);
        Some(nv_export::dng::DngInfo::from_metadata(&meta, &info, iso))
    }

    /// The roll's metadata and what is known about frame `idx`, shared by XMP and DNG tags.
    pub(super) fn frame_metadata(&self, idx: u32, settings: &ExportSettings) -> (nv_export::metadata::Metadata, nv_export::metadata::FrameInfo) {
        let roll = self.imp().roll.borrow();
        let mut meta = settings.metadata.clone();
        let Some(roll) = roll.as_ref() else { return (meta, Default::default()) };
        let frame = roll.frame(idx);
        let scan = frame.and_then(|f| f.raw.as_ref()).map(|r| &r.scan);
        let info = nv_export::metadata::FrameInfo {
            roll: roll.name.clone(),
            frame_number: Some(idx),
            label: frame.and_then(|f| f.label.clone()).unwrap_or_default(),
            scanner: roll.device_id.clone().unwrap_or_default(),
            resolution_dpi: scan.map(|s| s.resolution_dpi),
            scan_date: scan.map(|s| s.scanned_at.clone()).unwrap_or_default(),
            film_profile: {
                // A generic profile says nothing about the film.
                let p = self.effective_params(idx).profile;
                if p.vendor == "Generic" { String::new() } else { format!("{} {}", p.vendor, p.name) }
            },
        };
        if meta.film_stock.trim().is_empty() {
            meta.film_stock = roll.film_stock.clone().unwrap_or_default();
        }
        (meta, info)
    }

    /// The XMP packet for frame `idx`: what the person wrote for the roll plus
    /// what is known about the frame and its scan. `None` when switched off.
    pub(super) fn frame_xmp(&self, idx: u32, settings: &ExportSettings) -> Option<Vec<u8>> {
        if !settings.embed_metadata {
            return None;
        }
        self.imp().roll.borrow().as_ref()?;
        let (meta, info) = self.frame_metadata(idx, settings);
        Some(nv_export::metadata::xmp_packet(&meta, &info))
    }

    /// Queue the frames' scans for export with the roll's export settings.
    /// Frames that have not been scanned are skipped, and the toast says so.
    pub(super) fn queue_export(&self, frames: &[u32]) {
        // Uploading needs the saved sign-in, which lives in the keyring: fetch it first.
        if self.export_settings().google_photos && !self.imp().google_token_checked.get() {
            let frames = frames.to_vec();
            self.with_google_token(move |win| win.queue_export(&frames));
            return;
        }
        let Some(queue) = self.imp().export_queue.get() else { return };
        let settings = self.export_settings().normalised();
        let mut no_upload = false;
        let dir = self.export_folder(&settings);

        let (mut queued, mut unscanned) = (0, Vec::new());
        for &idx in frames {
            let raw = self.imp().roll.borrow().as_ref().and_then(|r| r.frame(idx)?.raw.as_ref().map(|raw| raw.filename.clone()));
            let Some(raw) = raw.and_then(|name| self.raw_file(&name)) else {
                unscanned.push(idx);
                continue;
            };
            let stem = match self.export_stem(idx, &settings.template) {
                Ok(s) => s,
                Err(e) => {
                    self.show_toast(&format!("Cannot name frame {idx}: {e}"));
                    continue;
                }
            };
            let id = self.imp().next_export_id.get() + 1;
            self.imp().next_export_id.set(id);
            let label = format!("Frame {idx}");
            let upload = self.upload_spec(idx, &settings);
            no_upload |= settings.google_photos && upload.is_none();
            self.imp().export_jobs.borrow_mut().push(JobInfo {
                id,
                frame: idx,
                label: label.clone(),
                state: JobState::Queued,
                fraction: 0.0,
                upload: if upload.is_some() { UploadState::Waiting } else { UploadState::No },
                spec: upload.clone(),
            });
            queue.enqueue(ExportJob {
                id,
                frame: idx,
                label,
                raw,
                params: self.effective_params(idx),
                orientation: self.orientation_of(idx),
                angle: self.angle_of(idx),
                xmp: self.frame_xmp(idx, &settings),
                dng: self.frame_dng_info(idx, &settings),
                settings: settings.clone(),
                dir: dir.clone(),
                stem,
                upload,
            });
            queued += 1;
        }
        self.refresh_queue_ui();
        if no_upload && queued > 0 {
            self.warn_no_upload();
        }
        if queued == 0 && !unscanned.is_empty() {
            self.show_toast("Nothing to export: scan the frame first");
        } else if !unscanned.is_empty() {
            self.show_toast(&format!("Skipped {} frame(s) that have not been scanned", unscanned.len()));
        }
    }

    /// Google Photos was asked for but cannot be used: say why, once, and offer the way out.
    fn warn_no_upload(&self) {
        let cfg = crate::google::oauth_config(&self.imp().prefs.borrow());
        let toast = if !cfg.is_set() {
            adw::Toast::new("Exported, but not sent to Google Photos: no Google client is set up (Preferences)")
        } else if !self.google_signed_in() {
            let t = adw::Toast::new("Exported, but not sent to Google Photos: you are not signed in");
            t.set_button_label(Some("Sign In"));
            t.set_action_name(Some("win.google-sign-in"));
            t
        } else {
            return; // only rejected frames were left out
        };
        if let Some(o) = self.imp().toast_overlay.borrow().as_ref() {
            o.add_toast(toast);
        }
    }

    /// Send a written picture to Google Photos again.
    fn retry_upload(&self, id: u64) {
        let job = self.imp().export_jobs.borrow().iter().find(|j| j.id == id).cloned();
        let (Some(job), Some(queue)) = (job, self.imp().export_queue.get()) else { return };
        let (JobState::Done(path), Some(spec)) = (job.state, job.spec) else { return };
        let Some(spec) = self.refreshed_spec(spec) else {
            self.warn_no_upload();
            return;
        };
        if let Some(j) = self.imp().export_jobs.borrow_mut().iter_mut().find(|j| j.id == id) {
            j.upload = UploadState::Waiting;
            j.spec = Some(spec.clone());
        }
        queue.retry_upload(id, path, spec);
        self.refresh_queue_ui();
    }

    fn handle_export_event(&self, ev: ExportEvent) {
        let set = |id: u64, f: &dyn Fn(&mut JobInfo)| {
            if let Some(j) = self.imp().export_jobs.borrow_mut().iter_mut().find(|j| j.id == id) {
                f(j);
            }
        };
        match ev {
            ExportEvent::Started { id } => set(id, &|j| j.state = JobState::Running),
            ExportEvent::Progress { id, fraction } => set(id, &|j| j.fraction = fraction),
            ExportEvent::Cancelled { id } => set(id, &|j| j.state = JobState::Cancelled),
            ExportEvent::Failed { id, message } => {
                let label = self.imp().export_jobs.borrow().iter().find(|j| j.id == id).map(|j| j.label.clone()).unwrap_or_default();
                set(id, &|j| j.state = JobState::Failed(message.clone()));
                self.show_toast(&format!("Could not export {label}: {message}"));
            }
            ExportEvent::Uploading { id } => set(id, &|j| j.upload = UploadState::Sending),
            ExportEvent::Uploaded { id, url } => set(id, &|j| j.upload = UploadState::Done(url.clone())),
            ExportEvent::AlbumCreated { title, id } => self.remember_album(&title, &id),
            ExportEvent::UploadFailed { id, message, sign_in_needed } => {
                let label = self.imp().export_jobs.borrow().iter().find(|j| j.id == id).map(|j| j.label.clone()).unwrap_or_default();
                set(id, &|j| j.upload = UploadState::Failed { message: message.clone(), sign_in_needed });
                let toast = adw::Toast::new(&format!("{label} was exported but not sent to Google Photos: {message}"));
                if sign_in_needed {
                    self.google_forget_token();
                    toast.set_button_label(Some("Sign In"));
                    toast.set_action_name(Some("win.google-sign-in"));
                }
                if let Some(o) = self.imp().toast_overlay.borrow().as_ref() {
                    o.add_toast(toast);
                }
            }
            ExportEvent::Done { id, report } => {
                let frame = self.imp().export_jobs.borrow().iter().find(|j| j.id == id).map(|j| j.frame);
                set(id, &|j| {
                    j.state = JobState::Done(report.path.clone());
                    j.fraction = 1.0;
                });
                if let Some(frame) = frame {
                    self.finish_export(frame, &report);
                }
            }
        }
        self.refresh_queue_ui();
    }

    /// An export finished: the frame is "exported", the roll remembers it,
    /// and the toast offers the folder.
    fn finish_export(&self, frame: u32, report: &nv_export::ExportReport) {
        if let Some(f) = self.imp().roll.borrow_mut().as_mut().and_then(|r| r.frame_mut(frame)) {
            f.state = FrameState::Exported;
        }
        if let Some(root) = self.imp().roll_root.borrow().clone() {
            let record = ExportRecord {
                frame,
                file: report.path.to_string_lossy().into_owned(),
                width: report.width,
                height: report.height,
                bytes: report.bytes,
                at: now_iso(),
                settings: self.export_settings(),
            };
            if let Err(e) = RollStore::from_parts(root, Roll::new("")).record_export(&record) {
                eprintln!("nv: could not record the export: {e}");
            }
        }
        self.rebuild_frame_list();
        self.schedule_autosave();
        if std::env::var_os("NV_AUTOSCAN").is_some() {
            eprintln!("nv: NV_AUTOSCAN: exported {}", report.path.display());
            self.flush_autosave();
            self.application().map(|a| a.quit());
        }

        if self.imp().prefs.borrow().open_after_export && std::env::var_os("NV_AUTOSCAN").is_none() {
            gtk::FileLauncher::new(Some(&gio::File::for_path(&report.path))).launch(Some(self), None::<&gio::Cancellable>, |r| {
                if let Err(e) = r {
                    eprintln!("nv: could not open the exported picture: {e}");
                }
            });
        }

        let name = report.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let toast = adw::Toast::new(&format!("Exported {name}"));
        toast.set_button_label(Some("Show in Files"));
        toast.set_action_name(Some("win.show-in-files"));
        toast.set_action_target_value(Some(&report.path.to_string_lossy().to_string().to_variant()));
        if let Some(o) = self.imp().toast_overlay.borrow().as_ref() {
            o.add_toast(toast);
        }
    }

    /// Cancel a queued or running export.
    pub(super) fn cancel_export(&self, id: u64) {
        if let Some(q) = self.imp().export_queue.get() {
            q.cancel(id);
        }
    }

    // ── The queue button and its popover ──────────────────────────────────────

    /// The header button that shows exports in progress, like Files' operations.
    pub(super) fn build_queue_button(&self) -> gtk::MenuButton {
        let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).build();
        let clear = gtk::Button::builder().label("Clear Finished").action_name("win.clear-finished-exports").css_classes(["flat"]).halign(gtk::Align::End).build();
        let title = gtk::Label::builder().label("Exports").css_classes(["heading"]).halign(gtk::Align::Start).build();
        let head = gtk::Box::builder().spacing(12).build();
        title.set_hexpand(true);
        head.append(&title);
        head.append(&clear);
        let body =
            gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).margin_top(12).margin_bottom(12).margin_start(12).margin_end(12).width_request(320).build();
        body.append(&head);
        let scroll = gtk::ScrolledWindow::builder().child(&list).max_content_height(320).propagate_natural_height(true).hscrollbar_policy(gtk::PolicyType::Never).build();
        body.append(&scroll);
        let popover = gtk::Popover::builder().child(&body).build();

        let btn = gtk::MenuButton::builder().icon_name("document-send-symbolic").tooltip_text("Exports").popover(&popover).visible(false).build();
        *self.imp().queue_list.borrow_mut() = Some(list);
        btn
    }

    /// Redraw the popover's rows and the button's state from the job list.
    pub(super) fn refresh_queue_ui(&self) {
        let imp = self.imp();
        let (Some(list), Some(btn)) = (imp.queue_list.borrow().clone(), imp.queue_button.borrow().clone()) else { return };
        while let Some(c) = list.first_child() {
            list.remove(&c);
        }
        let jobs = imp.export_jobs.borrow().clone();
        for j in &jobs {
            let row = adw::ActionRow::builder().title(&j.label).build();
            match &j.state {
                JobState::Queued => row.set_subtitle("Waiting…"),
                JobState::Running => {
                    let bar = gtk::ProgressBar::builder().fraction(j.fraction as f64).valign(gtk::Align::Center).width_request(90).build();
                    row.add_suffix(&bar);
                    row.set_subtitle(&format!("{}%", (j.fraction * 100.0).round()));
                }
                JobState::Done(path) => {
                    row.set_subtitle(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                    let show = gtk::Button::builder().icon_name("folder-open-symbolic").tooltip_text("Show in Files").valign(gtk::Align::Center).css_classes(["flat"]).build();
                    show.set_action_name(Some("win.show-in-files"));
                    show.set_action_target_value(Some(&path.to_string_lossy().to_string().to_variant()));
                    row.add_suffix(&show);
                    let sent = match &j.upload {
                        UploadState::No => None,
                        UploadState::Waiting => Some("waiting to upload".to_owned()),
                        UploadState::Sending => Some("uploading to Google Photos…".to_owned()),
                        UploadState::Done(_) => Some("in Google Photos".to_owned()),
                        UploadState::Failed { message, .. } => Some(format!("upload failed: {message}")),
                    };
                    if let Some(sent) = sent {
                        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        row.set_subtitle(&format!("{name} · {sent}"));
                    }
                    match &j.upload {
                        UploadState::Sending => row.add_suffix(&gtk::Spinner::builder().spinning(true).valign(gtk::Align::Center).build()),
                        UploadState::Done(Some(url)) => {
                            let open = gtk::Button::builder()
                                .icon_name("web-browser-symbolic")
                                .tooltip_text("Open in Google Photos")
                                .valign(gtk::Align::Center)
                                .css_classes(["flat"])
                                .build();
                            let url = url.clone();
                            open.connect_clicked(move |b| {
                                let win = b.root().and_downcast::<gtk::Window>();
                                gtk::UriLauncher::new(&url).launch(win.as_ref(), None::<&gio::Cancellable>, |_| {});
                            });
                            row.add_suffix(&open);
                        }
                        UploadState::Failed { sign_in_needed, .. } => {
                            row.add_css_class("error");
                            let again = gtk::Button::builder()
                                .icon_name("view-refresh-symbolic")
                                .tooltip_text(if *sign_in_needed { "Sign in to Google Photos" } else { "Try the upload again" })
                                .valign(gtk::Align::Center)
                                .css_classes(["flat"])
                                .build();
                            if *sign_in_needed {
                                again.set_action_name(Some("win.google-sign-in"));
                            } else {
                                again.set_action_name(Some("win.retry-upload"));
                                again.set_action_target_value(Some(&j.id.to_variant()));
                            }
                            row.add_suffix(&again);
                        }
                        _ => {}
                    }
                }
                JobState::Failed(m) => {
                    row.set_subtitle(m);
                    row.add_css_class("error");
                }
                JobState::Cancelled => row.set_subtitle("Cancelled"),
            }
            if matches!(j.state, JobState::Queued | JobState::Running) {
                let stop = gtk::Button::builder().icon_name("process-stop-symbolic").tooltip_text("Cancel").valign(gtk::Align::Center).css_classes(["flat"]).build();
                let (weak, id) = (self.downgrade(), j.id);
                stop.connect_clicked(move |_| {
                    if let Some(win) = weak.upgrade() {
                        win.cancel_export(id);
                    }
                });
                row.add_suffix(&stop);
            }
            list.append(&row);
        }

        btn.set_visible(!jobs.is_empty());
        let active = jobs.iter().filter(|j| matches!(j.state, JobState::Queued | JobState::Running) || j.upload.active()).count();
        btn.set_icon_name(if active > 0 { "emblem-synchronizing-symbolic" } else { "document-send-symbolic" });
        btn.set_tooltip_text(Some(&if active > 0 { format!("{active} export(s) in progress") } else { "Exports".to_owned() }));
        super::chrome::label_icon_buttons(btn.upcast_ref());
        super::chrome::label_icon_buttons(list.upcast_ref());
    }
}
