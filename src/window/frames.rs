//! The roll's frames: the sidebar list, selection, previews on disk,
//! detection, delete/duplicate/keep, importing raw scans, and auto-save.

use std::path::PathBuf;

use adw::prelude::*;
use adw::subclass::prelude::ObjectSubclassIsExt;
use gtk::gdk;
use nv_store::{Frame, FrameState, RawReader, RawWriter, Roll, RollStore};

use super::{imp, Window};
use crate::canvas_geom as geom;

/// Longest side of a sidebar thumbnail, in pixels.
const THUMB_SIDE: u32 = 96;
/// Auto-save delay after the last edit (DESIGN.md §6.4).
const AUTOSAVE_SECS: u32 = 2;

/// Where a frame's image comes from: the file, whether it is the preview
/// (as opposed to a full scan), and the frame's crop within a preview.
pub(super) struct Source {
    pub path: PathBuf,
    pub is_preview: bool,
    pub crop: Option<[u32; 4]>,
}

fn now_iso() -> String {
    glib::DateTime::now_utc()
        .ok()
        .and_then(|d| d.format_iso8601().ok())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

impl Window {
    // ── Auto-save ─────────────────────────────────────────────────────────────

    /// Save `roll.json` two seconds after the last edit.
    pub(super) fn schedule_autosave(&self) {
        if let Some(id) = self.imp().autosave_id.take() {
            id.remove();
        }
        let weak = self.downgrade();
        let id = glib::timeout_add_seconds_local_once(AUTOSAVE_SECS, move || {
            if let Some(win) = weak.upgrade() {
                // The source is gone once it has fired; drop the handle
                // without removing it.
                drop(win.imp().autosave_id.take());
                win.save_roll();
            }
        });
        *self.imp().autosave_id.borrow_mut() = Some(id);
    }

    /// Write any pending change now (on close, before a long scan).
    pub(super) fn flush_autosave(&self) {
        if let Some(id) = self.imp().autosave_id.take() {
            id.remove();
            self.save_roll();
        }
    }

    // ── Lookups ───────────────────────────────────────────────────────────────

    fn raw_file(&self, name: &str) -> Option<PathBuf> {
        let root = self.imp().roll_root.borrow().clone()?;
        Some(RollStore::from_parts(root, Roll::new("")).raw_path(name))
    }

    pub(super) fn frame_source(&self, idx: u32) -> Option<Source> {
        let roll = self.imp().roll.borrow();
        let frame = roll.as_ref()?.frame(idx)?;
        if let Some(prev) = &frame.preview {
            Some(Source { path: self.raw_file(prev)?, is_preview: true, crop: frame.geometry.crop })
        } else {
            let raw = frame.raw.as_ref()?;
            Some(Source { path: self.raw_file(&raw.filename)?, is_preview: false, crop: None })
        }
    }

    fn frame_index_at(&self, position: usize) -> Option<u32> {
        self.imp().roll.borrow().as_ref()?.frames.get(position).map(|f| f.index)
    }

    fn selected(&self) -> Option<u32> {
        *self.imp().selected_frame.borrow()
    }

    /// Whether a scanner is attached and open.
    pub(super) fn device_ready(&self) -> bool {
        matches!(*self.imp().scan_state.borrow(), imp::ScanUiState::Ready { .. })
    }

    // ── Busy state ────────────────────────────────────────────────────────────

    /// Lock the frame list while an acquisition is in flight, so a click on
    /// another frame cannot swap the image under the incoming scan.
    pub(super) fn set_busy(&self, busy: bool) {
        self.imp().busy.set(busy);
        if let Some(list) = self.imp().frame_list.borrow().as_ref() {
            list.set_sensitive(!busy);
        }
        if busy {
            self.set_scan_controls_sensitive(false, false);
        } else {
            self.update_scan_controls();
        }
    }

    /// Preview needs a scanner; Scan also needs a preview to frame.
    pub(super) fn update_scan_controls(&self) {
        if self.imp().busy.get() {
            return;
        }
        let ready = self.device_ready();
        self.set_scan_controls_sensitive(ready, ready && self.imp().preview_ready.get());
    }

    // ── Selection and loading ─────────────────────────────────────────────────

    /// Show a frame: select its row and load its image.
    pub(super) fn select_frame(&self, idx: u32) {
        *self.imp().selected_frame.borrow_mut() = Some(idx);
        self.sync_list_selection();

        let Some(src) = self.frame_source(idx) else {
            self.clear_canvas();
            return;
        };

        // Frames cut from one preview share its file: only the rectangle moves.
        let already = self.imp().displayed_file.borrow().as_deref() == Some(src.path.as_path())
            && self.imp().scan_buf.borrow().is_some();
        if already {
            self.show_frame_rect(&src);
            return;
        }

        let generation = self.imp().load_gen.get() + 1;
        self.imp().load_gen.set(generation);
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let path = src.path.clone();
            let result = gio::spawn_blocking(move || {
                RawReader::read(&path)
                    .map(|buf| {
                        let proxy = nv_process::Proxy::from_raw(&buf, nv_process::DEFAULT_PROXY_PIXELS);
                        (buf, proxy)
                    })
                    .map_err(|e| e.to_string())
            })
            .await;
            let Some(win) = weak.upgrade() else { return };
            if win.imp().load_gen.get() != generation {
                return; // another frame was picked meanwhile
            }
            match result {
                Ok(Ok((buf, proxy))) => {
                    // Set before loading: the frame's measuring region depends on them.
                    *win.imp().displayed_file.borrow_mut() = Some(src.path.clone());
                    win.imp().preview_ready.set(src.is_preview);
                    win.load_frame_buf_with_proxy(buf, proxy);
                    win.show_frame_rect(&src);
                }
                Ok(Err(e)) => win.show_toast(&format!("Could not load frame {idx}: {e}")),
                Err(_) => win.show_toast(&format!("Could not load frame {idx}")),
            }
        });
    }

    /// Put the frame's rectangle on the canvas and set the buttons for it.
    fn show_frame_rect(&self, src: &Source) {
        let rect = src
            .crop
            .filter(|_| src.is_preview)
            .and_then(|c| {
                let (w, h) = self.canvas_image_size()?;
                geom::crop_to_rect(c, w as u32, h as u32)
            });
        self.imp().frame_rect.set(rect);
        if let Some(a) = self.imp().frame_overlay.borrow().as_ref() {
            a.queue_draw();
        }
        self.imp().preview_ready.set(src.is_preview);
        self.update_scan_controls();
        self.update_info_label();
    }

    fn clear_canvas(&self) {
        let imp = self.imp();
        imp.load_gen.set(imp.load_gen.get() + 1);
        *imp.scan_buf.borrow_mut() = None;
        *imp.proxy.borrow_mut() = None;
        *imp.histogram.borrow_mut() = None;
        *imp.levels.borrow_mut() = None;
        if let Some(a) = imp.histogram_area.borrow().as_ref() {
            a.queue_draw();
        }
        *imp.displayed_file.borrow_mut() = None;
        imp.frame_rect.set(None);
        imp.preview_ready.set(false);
        if let Some(pic) = imp.canvas_picture.borrow().as_ref() {
            pic.set_paintable(None::<&gdk::Paintable>);
        }
        if let Some(a) = imp.frame_overlay.borrow().as_ref() {
            a.queue_draw();
        }
        self.update_scan_controls();
        self.update_info_label();
    }

    /// "1200 ppi · 1555 × 1090 px" under the canvas.
    pub(super) fn update_info_label(&self) {
        let imp = self.imp();
        let Some(label) = imp.info_label.borrow().clone() else { return };
        let text = match self.canvas_image_size() {
            None => String::new(),
            Some((w, h)) => {
                let dpi = self
                    .selected()
                    .and_then(|i| imp.roll.borrow().as_ref()?.frame(i)?.raw.as_ref().map(|r| r.scan.resolution_dpi))
                    .filter(|d| *d > 0);
                match dpi {
                    Some(d) if !imp.preview_ready.get() => format!("{d} ppi · {w:.0} × {h:.0} px"),
                    _ => format!("{w:.0} × {h:.0} px"),
                }
            }
        };
        label.set_text(&text);
    }

    pub(super) fn select_relative(&self, step: isize) {
        let Some(roll) = self.imp().roll.borrow().clone() else { return };
        let cur = self.selected().and_then(|i| roll.frames.iter().position(|f| f.index == i));
        let next = match cur {
            Some(p) => p as isize + step,
            None => 0,
        };
        if let Some(f) = usize::try_from(next).ok().and_then(|p| roll.frames.get(p)) {
            self.select_frame(f.index);
        }
    }

    // ── The list ──────────────────────────────────────────────────────────────

    pub(super) fn rebuild_frame_list(&self) {
        let Some(list) = self.imp().frame_list.borrow().clone() else { return };
        self.imp().suppress_select.set(true);
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }
        if let Some(roll) = self.imp().roll.borrow().as_ref() {
            let thumbs = self.imp().thumbs.borrow();
            for frame in &roll.frames {
                list.append(&make_frame_row(frame, thumbs.get(&frame.index)));
            }
        }
        self.imp().suppress_select.set(false);
        self.sync_list_selection();
    }

    fn sync_list_selection(&self) {
        let Some(list) = self.imp().frame_list.borrow().clone() else { return };
        let pos = self.selected().and_then(|sel| {
            self.imp().roll.borrow().as_ref()?.frames.iter().position(|f| f.index == sel)
        });
        self.imp().suppress_select.set(true);
        match pos.and_then(|p| list.row_at_index(p as i32)) {
            Some(row) => list.select_row(Some(&row)),
            None => list.unselect_all(),
        }
        self.imp().suppress_select.set(false);
    }

    /// Wire the list's selection and the sidebar and frame actions.
    pub(super) fn setup_frames(&self, list: &gtk::ListBox) {
        let weak = self.downgrade();
        list.connect_row_selected(move |_, row| {
            let Some(win) = weak.upgrade() else { return };
            if win.imp().suppress_select.get() {
                return;
            }
            let Some(idx) = row.and_then(|r| win.frame_index_at(r.index() as usize)) else { return };
            if win.selected() != Some(idx) {
                win.select_frame(idx);
            }
        });

        let actions: [(&str, fn(&Window)); 8] = [
            ("detect-frames", |w| w.detect_frames()),
            ("delete-frame", |w| w.delete_frame()),
            ("duplicate-frame", |w| w.duplicate_frame()),
            ("toggle-keep", |w| w.toggle_rating(1)),
            ("toggle-reject", |w| w.toggle_rating(-1)),
            ("next-frame", |w| w.select_relative(1)),
            ("prev-frame", |w| w.select_relative(-1)),
            ("advance-holder", |w| w.show_advance_banner()),
        ];
        for (name, f) in actions {
            let action = gio::SimpleAction::new(name, None);
            let weak = self.downgrade();
            action.connect_activate(move |_, _| {
                if let Some(win) = weak.upgrade() {
                    if !win.imp().busy.get() {
                        f(&win);
                    }
                }
            });
            self.add_action(&action);
        }
    }

    // ── Thumbnails ────────────────────────────────────────────────────────────

    /// Build sidebar thumbnails in the background, one frame at a time.
    pub(super) fn spawn_thumbnails(&self, indices: Vec<u32>) {
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            for idx in indices {
                let Some(src) = weak.upgrade().and_then(|w| w.frame_source(idx)) else { continue };
                let thumb = gio::spawn_blocking(move || {
                    let buf = RawReader::read(&src.path).ok()?;
                    let region = src
                        .crop
                        .filter(|_| src.is_preview)
                        .and_then(|c| geom::crop_to_rect(c, buf.width, buf.effective_height()));
                    Some(buf.thumbnail_rgba(THUMB_SIDE, region))
                })
                .await;
                let Some(win) = weak.upgrade() else { return };
                if let Ok(Some((w, h, px))) = thumb {
                    if w > 0 && h > 0 {
                        let texture = gdk::MemoryTexture::new(
                            w as i32,
                            h as i32,
                            gdk::MemoryFormat::R8g8b8a8,
                            &glib::Bytes::from_owned(px),
                            (w * 4) as usize,
                        );
                        win.imp().thumbs.borrow_mut().insert(idx, texture.upcast());
                        win.rebuild_frame_list();
                    }
                }
            }
        });
    }

    fn all_frame_indices(&self) -> Vec<u32> {
        self.imp().roll.borrow().as_ref().map(|r| r.frames.iter().map(|f| f.index).collect()).unwrap_or_default()
    }

    // ── Previews ──────────────────────────────────────────────────────────────

    /// The preview on the canvas is complete: give it a frame, write it into
    /// the roll so the roll can be reopened without the scanner.
    pub(super) fn adopt_preview(&self) {
        let new_frame = self.imp().advance_pending.replace(false);
        let Some(idx) = self.ensure_roll_and_frame(new_frame) else {
            self.load_params_for(None);
            return;
        };
        if new_frame {
            self.imp().frame_rect.set(None);
            if let Some(a) = self.imp().frame_overlay.borrow().as_ref() {
                a.queue_draw();
            }
        }
        self.store_frame_crop(idx);
        self.rebuild_frame_list();
        self.load_params_for(Some(idx));

        let Some(buf) = self.imp().scan_buf.borrow().clone() else { return };
        let Some(root) = self.imp().roll_root.borrow().clone() else { return };
        let path = RollStore::from_parts(root, Roll::new("")).preview_path(idx);
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();

        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let write_path = path.clone();
            let result = gio::spawn_blocking(move || RawWriter::write_scan(&buf, &write_path).map_err(|e| e.to_string())).await;
            let Some(win) = weak.upgrade() else { return };
            match result {
                Ok(Ok(())) => {
                    if let Some(f) = win.imp().roll.borrow_mut().as_mut().and_then(|r| r.frame_mut(idx)) {
                        f.preview = Some(name);
                        if matches!(f.state, FrameState::Empty | FrameState::Previewed) {
                            f.state = FrameState::Previewed;
                        }
                    }
                    *win.imp().displayed_file.borrow_mut() = Some(path);
                    win.rebuild_frame_list();
                    win.spawn_thumbnails(vec![idx]);
                    win.schedule_autosave();
                }
                Ok(Err(e)) => win.show_toast(&format!("Could not save the preview: {e}")),
                Err(_) => win.show_toast("Could not save the preview"),
            }
        });
    }

    // ── Detect, delete, duplicate, rate ───────────────────────────────────────

    /// Find the frames on the preview and make one roll frame for each.
    pub(super) fn detect_frames(&self) {
        let imp = self.imp();
        let Some(idx) = self.selected() else {
            self.show_toast("Preview the film first");
            return;
        };
        let preview = imp.roll.borrow().as_ref().and_then(|r| r.frame(idx)?.preview.clone());
        let showing_it = preview
            .as_ref()
            .and_then(|p| self.raw_file(p))
            .is_some_and(|p| imp.displayed_file.borrow().as_deref() == Some(p.as_path()));
        let (Some(preview), true, Some((iw, ih))) = (preview, showing_it, self.canvas_image_size()) else {
            self.show_toast("Preview the film first");
            return;
        };

        // Detection needs little resolution.
        let rects = {
            let buf = imp.scan_buf.borrow();
            let Some(buf) = buf.as_ref() else { return };
            let (w, h, rgba) = buf.thumbnail_rgba(1024, None);
            let luma: Vec<u8> = rgba
                .chunks_exact(4)
                .map(|p| ((p[0] as u16 + p[1] as u16 + p[2] as u16) / 3) as u8)
                .collect();
            nv_film::detect_frames(&luma, w as usize, h as usize, nv_film::DetectOptions::default())
        };
        let crop_of = |r: [f64; 4]| geom::rect_to_crop(r, iw as u32, ih as u32);

        let mut fresh = Vec::new();
        if let Some(roll) = imp.roll.borrow_mut().as_mut() {
            // Redoing detection replaces earlier unscanned cuts of this preview.
            roll.frames.retain(|f| f.index == idx || f.preview.as_deref() != Some(&preview) || f.raw.is_some());
            if let Some(f) = roll.frame_mut(idx) {
                f.geometry.crop = Some(crop_of(rects[0]));
            }
            let mut pos = roll.frames.iter().position(|f| f.index == idx).unwrap_or(0);
            for r in &rects[1..] {
                let next = roll.frames.iter().map(|f| f.index).max().unwrap_or(0) + 1;
                let mut f = Frame::new(next);
                f.preview = Some(preview.clone());
                f.state = FrameState::Previewed;
                f.geometry.crop = Some(crop_of(*r));
                pos += 1;
                roll.frames.insert(pos, f);
                fresh.push(next);
            }
        }

        imp.frame_rect.set(Some(rects[0]));
        if let Some(a) = imp.frame_overlay.borrow().as_ref() {
            a.queue_draw();
        }
        self.rebuild_frame_list();
        let mut thumbs = vec![idx];
        thumbs.extend(fresh);
        self.spawn_thumbnails(thumbs);
        self.schedule_autosave();
        self.show_toast(&match rects.len() {
            1 => "Found 1 frame".to_owned(),
            n => format!("Found {n} frames"),
        });
    }

    fn delete_frame(&self) {
        let Some(idx) = self.selected() else { return };
        let imp = self.imp();
        let (removed, neighbour, preview_shared) = {
            let mut roll = imp.roll.borrow_mut();
            let Some(roll) = roll.as_mut() else { return };
            let pos = roll.frames.iter().position(|f| f.index == idx);
            let Some(removed) = roll.delete_frame(idx) else { return };
            let shared = removed.preview.as_ref().is_some_and(|p| roll.frames.iter().any(|f| f.preview.as_ref() == Some(p)));
            let near = pos.and_then(|p| roll.frames.get(p.min(roll.frames.len().saturating_sub(1)))).map(|f| f.index);
            (removed, near, shared)
        };

        // The scan files go with the frame; a preview shared with other cuts stays.
        for name in removed.raw.iter().map(|r| &r.filename).chain(removed.preview.iter().filter(|_| !preview_shared)) {
            if let Some(p) = self.raw_file(name) {
                let _ = std::fs::remove_file(p);
            }
        }
        imp.thumbs.borrow_mut().remove(&idx);

        *imp.selected_frame.borrow_mut() = None;
        match neighbour {
            Some(n) => {
                self.rebuild_frame_list();
                self.select_frame(n);
            }
            None => {
                self.clear_canvas();
                self.rebuild_frame_list();
            }
        }
        self.schedule_autosave();
        self.show_toast(&format!("Deleted frame {idx}"));
    }

    fn duplicate_frame(&self) {
        let Some(idx) = self.selected() else { return };
        let new = self.imp().roll.borrow_mut().as_mut().and_then(|r| r.duplicate_frame(idx));
        if let Some(new) = new {
            self.rebuild_frame_list();
            self.select_frame(new);
            self.spawn_thumbnails(vec![new]);
            self.schedule_autosave();
        }
    }

    /// Toggle keep (`1`) or reject (`-1`) on the selected frame.
    fn toggle_rating(&self, rating: i8) {
        let Some(idx) = self.selected() else { return };
        if let Some(f) = self.imp().roll.borrow_mut().as_mut().and_then(|r| r.frame_mut(idx)) {
            f.rating = if f.rating == Some(rating) { Some(0) } else { Some(rating) };
        }
        self.rebuild_frame_list();
        self.schedule_autosave();
    }

    // ── Open Raw Scans ────────────────────────────────────────────────────────

    /// Copy raw scans into a new roll, one frame each, and open it.
    pub(super) fn import_raw_files(&self, sources: Vec<PathBuf>) {
        if sources.is_empty() || !self.create_roll_folder("Imported Scans", false) {
            return;
        }
        let Some(root) = self.imp().roll_root.borrow().clone() else { return };
        let store = RollStore::from_parts(root, Roll::new(""));
        let jobs: Vec<(u32, PathBuf, PathBuf)> = sources
            .into_iter()
            .enumerate()
            .map(|(i, src)| (i as u32 + 1, src, store.scan_path(i as u32 + 1)))
            .collect();

        self.show_toast("Importing…");
        self.set_busy(true);
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let done = gio::spawn_blocking(move || {
                jobs.into_iter()
                    .map(|(idx, src, dest)| {
                        let r = RawReader::read(&src)
                            .and_then(|b| {
                                RawWriter::write_scan(&b, &dest)?;
                                Ok((b.width, b.effective_height(), b.depth, b.channels))
                            })
                            .map_err(|e| e.to_string());
                        (idx, src, dest, r)
                    })
                    .collect::<Vec<_>>()
            })
            .await;
            let Some(win) = weak.upgrade() else { return };
            win.set_busy(false);
            let Ok(done) = done else {
                win.show_toast("Import failed");
                return;
            };

            let mut failures = Vec::new();
            if let Some(roll) = win.imp().roll.borrow_mut().as_mut() {
                for (idx, src, dest, result) in done {
                    let stem = src.file_stem().map(|s| s.to_string_lossy().into_owned());
                    match result {
                        Ok((w, h, depth, channels)) => {
                            let mut f = Frame::new(idx);
                            f.label = stem;
                            f.state = FrameState::Scanned;
                            f.raw = Some(nv_store::RawRef {
                                filename: dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                                scan: nv_store::model::ScanSettings {
                                    resolution_dpi: 0, // unknown for imported files
                                    depth,
                                    channels,
                                    scanned_at: now_iso(),
                                },
                                has_ir: false,
                                width_px: w,
                                height_px: h,
                            });
                            roll.frames.push(f);
                        }
                        Err(e) => failures.push(format!("{}: {e}", stem.unwrap_or_default())),
                    }
                }
            }
            if !failures.is_empty() {
                win.show_toast(&format!("Could not import {}", failures.join("; ")));
            }
            win.rebuild_frame_list();
            win.set_page("canvas");
            if let Some(first) = win.all_frame_indices().first().copied() {
                win.select_frame(first);
            }
            win.spawn_thumbnails(win.all_frame_indices());
            win.schedule_autosave();
        });
    }

    /// Open a roll that has just been read from disk.
    pub(super) fn show_opened_roll(&self) {
        self.imp().thumbs.borrow_mut().clear();
        self.rebuild_frame_list();
        self.set_page("canvas");
        self.clear_canvas();
        let all = self.all_frame_indices();
        if let Some(first) = all.first().copied() {
            self.select_frame(first);
        }
        self.spawn_thumbnails(all);
    }
}

fn make_frame_row(frame: &Frame, thumb: Option<&gdk::Texture>) -> gtk::ListBoxRow {
    let preview: gtk::Widget = match thumb {
        Some(t) => {
            let pic = gtk::Picture::for_paintable(t);
            pic.set_content_fit(gtk::ContentFit::Contain);
            pic.upcast()
        }
        None => gtk::Image::from_icon_name("image-x-generic-symbolic").upcast(),
    };
    preview.set_size_request(64, 48);

    let title = match &frame.label {
        Some(l) => format!("{} · {l}", frame.index),
        None => frame.index.to_string(),
    };
    let label = gtk::Label::builder().label(&title).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).build();

    let state_icon = match frame.state {
        FrameState::Empty => "circle-outline-thick-symbolic",
        FrameState::Previewed => "image-x-generic-symbolic",
        FrameState::Scanned => "emblem-ok-symbolic",
        FrameState::NeedsRescan => "view-refresh-symbolic",
        FrameState::Exported => "emblem-shared-symbolic",
    };
    let state = gtk::Image::from_icon_name(state_icon);
    state.set_tooltip_text(Some(match frame.state {
        FrameState::Empty => "No preview yet",
        FrameState::Previewed => "Previewed",
        FrameState::Scanned => "Scanned",
        FrameState::NeedsRescan => "Needs rescan",
        FrameState::Exported => "Exported",
    }));

    let row_box = gtk::Box::builder().spacing(8).margin_start(6).margin_end(6).margin_top(4).margin_bottom(4).build();
    row_box.append(&preview);
    row_box.append(&label);
    if frame.rating == Some(1) {
        let star = gtk::Image::from_icon_name("starred-symbolic");
        star.set_tooltip_text(Some("Kept"));
        row_box.append(&star);
    }
    row_box.append(&state);

    let row = gtk::ListBoxRow::builder().child(&row_box).build();
    if frame.rating == Some(-1) {
        row.add_css_class("dim-label");
        label.set_label(&format!("{title} (rejected)"));
    }
    row
}
