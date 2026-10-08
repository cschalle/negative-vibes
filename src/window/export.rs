//! Exporting from the window: queueing jobs, the queue popover in the header,
//! and what happens when an export finishes.

use adw::prelude::*;
use adw::subclass::prelude::ObjectSubclassIsExt;
use nv_export::{render_template, ExportSettings, NameContext};
use nv_store::{ExportRecord, FrameState, Roll, RollStore};

use super::Window;
use crate::export_queue::{ExportEvent, ExportJob, ExportQueue};

#[derive(Debug, Clone, PartialEq)]
pub enum JobState {
    Queued,
    Running,
    Done(std::path::PathBuf),
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct JobInfo {
    pub id: u64,
    pub frame: u32,
    pub label: String,
    pub state: JobState,
    pub fraction: f32,
}

fn today() -> String {
    glib::DateTime::now_local()
        .ok()
        .and_then(|d| d.format("%Y-%m-%d").ok())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

fn now_iso() -> String {
    glib::DateTime::now_utc()
        .ok()
        .and_then(|d| d.format_iso8601().ok())
        .map(|s| s.to_string())
        .unwrap_or_default()
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
            gtk::FileLauncher::new(Some(&gio::File::for_path(path)))
                .open_containing_folder(Some(&win), None::<&gio::Cancellable>, |_| {});
        });
        self.add_action(&show);

        let clear = gio::SimpleAction::new("clear-finished-exports", None);
        let weak = self.downgrade();
        clear.connect_activate(move |_, _| {
            if let Some(win) = weak.upgrade() {
                win.imp().export_jobs.borrow_mut().retain(|j| matches!(j.state, JobState::Queued | JobState::Running));
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
        settings.folder.clone().unwrap_or_else(|| {
            glib::user_special_dir(glib::UserDirectory::Pictures)
                .unwrap_or_else(glib::home_dir)
                .join("Scans")
                .join(folder_safe(&self.roll_name()))
        })
    }

    /// The file name (no extension) `frame` would be exported under.
    pub(super) fn export_stem(&self, frame: u32, template: &str) -> Result<String, nv_export::NameError> {
        let roll = self.imp().roll.borrow();
        let f = roll.as_ref().and_then(|r| r.frame(frame));
        render_template(
            template,
            &NameContext {
                roll: self.roll_name(),
                frame,
                label: f.and_then(|f| f.label.clone()),
                film: roll.as_ref().and_then(|r| r.film_stock.clone()),
                date: today(),
            },
        )
    }

    /// The XMP packet for frame `idx`: what the person wrote for the roll plus
    /// what is known about the frame and its scan. `None` when switched off.
    pub(super) fn frame_xmp(&self, idx: u32, settings: &ExportSettings) -> Option<Vec<u8>> {
        if !settings.embed_metadata {
            return None;
        }
        let roll = self.imp().roll.borrow();
        let roll = roll.as_ref()?;
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
        // The roll's film stock fills the field when the person left it empty.
        let mut meta = settings.metadata.clone();
        if meta.film_stock.trim().is_empty() {
            meta.film_stock = roll.film_stock.clone().unwrap_or_default();
        }
        Some(nv_export::metadata::xmp_packet(&meta, &info))
    }

    /// Queue the frames' scans for export with the roll's export settings.
    /// Frames that have not been scanned are skipped, and the toast says so.
    pub(super) fn queue_export(&self, frames: &[u32]) {
        let Some(queue) = self.imp().export_queue.get() else { return };
        let settings = self.export_settings().normalised();
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
            self.imp().export_jobs.borrow_mut().push(JobInfo { id, frame: idx, label: label.clone(), state: JobState::Queued, fraction: 0.0 });
            queue.enqueue(ExportJob { id, frame: idx, label, raw, params: self.effective_params(idx), orientation: self.orientation_of(idx), angle: self.angle_of(idx), xmp: self.frame_xmp(idx, &settings), settings: settings.clone(), dir: dir.clone(), stem });
            queued += 1;
        }
        self.refresh_queue_ui();
        if queued == 0 && !unscanned.is_empty() {
            self.show_toast("Nothing to export: scan the frame first");
        } else if !unscanned.is_empty() {
            self.show_toast(&format!("Skipped {} frame(s) that have not been scanned", unscanned.len()));
        }
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
        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).margin_top(12).margin_bottom(12).margin_start(12).margin_end(12).width_request(320).build();
        body.append(&head);
        let scroll = gtk::ScrolledWindow::builder().child(&list).max_content_height(320).propagate_natural_height(true).hscrollbar_policy(gtk::PolicyType::Never).build();
        body.append(&scroll);
        let popover = gtk::Popover::builder().child(&body).build();

        let btn = gtk::MenuButton::builder()
            .icon_name("document-send-symbolic")
            .tooltip_text("Exports")
            .popover(&popover)
            .visible(false)
            .build();
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
        let active = jobs.iter().filter(|j| matches!(j.state, JobState::Queued | JobState::Running)).count();
        btn.set_icon_name(if active > 0 { "emblem-synchronizing-symbolic" } else { "document-send-symbolic" });
        btn.set_tooltip_text(Some(&if active > 0 { format!("{active} export(s) in progress") } else { "Exports".to_owned() }));
    }
}
