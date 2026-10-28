//! Scanning more than one frame: Scan Selected, Scan All, Scan Raw Only, and
//! Stop. A batch is a list of frames, each scanned in turn inside its own
//! rectangle of the preview. On a manual-advance scanner there is only ever
//! one frame in the window, so a batch there is a loop: scan, ask the user to
//! move the holder, preview and scan again, until they stop (DESIGN.md §5.7).

use adw::prelude::*;
use adw::subclass::prelude::ObjectSubclassIsExt;

use super::Window;
use crate::canvas_geom as geom;

impl Window {
    /// Register the scan actions: the Scan button's menu and its shortcuts.
    pub(super) fn setup_batch(&self) {
        let actions: [(&str, fn(&Window)); 4] =
            [("scan-selected", |w| w.scan_selected(false)), ("scan-all", |w| w.scan_all()), ("scan-raw-only", |w| w.scan_selected(true)), ("stop", |w| w.stop())];
        for (name, f) in actions {
            let action = gio::SimpleAction::new(name, None);
            let weak = self.downgrade();
            action.connect_activate(move |_, _| {
                if let Some(win) = weak.upgrade() {
                    f(&win);
                }
            });
            self.add_action(&action);
        }
    }

    /// Scan the frame on the canvas, inside the rectangle drawn on it.
    /// `raw_only` keeps the raw scan and skips the export that would follow.
    pub(super) fn scan_selected(&self, raw_only: bool) {
        let imp = self.imp();
        if imp.busy.get() {
            return;
        }
        self.outside_guide();
        if !imp.preview_ready.get() || !self.device_ready() {
            self.show_toast("Preview the film first");
            return;
        }
        imp.batch.borrow_mut().clear();
        imp.batch_loop.set(false);
        imp.batch_raw_only.set(raw_only);
        self.start_final_scan();
    }

    /// Scan every previewed frame that has no scan yet; on a manual-advance
    /// scanner, scan this one and keep going as the user advances the holder.
    pub(super) fn scan_all(&self) {
        let imp = self.imp();
        if imp.busy.get() {
            return;
        }
        self.outside_guide();
        if !self.device_ready() {
            self.show_toast("No scanner is ready");
            return;
        }
        imp.batch_raw_only.set(false);
        if imp.manual_advance.get() {
            if !imp.preview_ready.get() {
                self.show_toast("Preview the film first");
                return;
            }
            imp.batch_loop.set(true);
            imp.batch_done.set(0);
            self.start_final_scan();
            return;
        }

        let frames = imp.roll.borrow().as_ref().map(|r| r.frames_to_scan()).unwrap_or_default();
        if frames.is_empty() {
            self.show_toast("No previewed frames left to scan");
            return;
        }
        imp.batch_loop.set(false);
        imp.batch_total.set(frames.len() as u32);
        imp.batch_done.set(0);
        *imp.batch.borrow_mut() = frames.into();
        self.scan_next_in_batch();
    }

    /// Scan the next frame of the batch inside its own rectangle.
    fn scan_next_in_batch(&self) {
        let imp = self.imp();
        let Some(idx) = imp.batch.borrow_mut().pop_front() else { return };

        // The crop is in preview pixels; the scanner wants fractions of its window.
        let area = imp.roll.borrow().as_ref().and_then(|r| r.frame(idx)).and_then(|f| {
            let [w, h] = f.preview_size?;
            let r = geom::crop_to_rect(f.geometry.crop?, w, h)?;
            Some(r)
        });
        let n = imp.batch_done.get() + 1;
        self.show_toast(&format!("Scanning frame {idx} ({n} of {})", imp.batch_total.get()));
        *imp.selected_frame.borrow_mut() = Some(idx);
        self.rebuild_frame_list();
        self.start_final_scan_of(idx, area);
    }

    /// A frame has been scanned and saved: the next step of the batch, if any.
    /// Called once the scanner is free again.
    pub(super) fn continue_batch(&self) {
        let imp = self.imp();
        imp.batch_done.set(imp.batch_done.get() + 1);
        if !imp.batch.borrow().is_empty() {
            self.scan_next_in_batch();
        } else if imp.batch_total.get() > 0 {
            let n = imp.batch_total.replace(0);
            imp.batch_done.set(0);
            self.show_toast(&format!("Scanned {n} frames"));
            // Back to showing the frame that is selected, with its own rectangle.
            if let Some(sel) = *imp.selected_frame.borrow() {
                self.select_frame(sel);
            }
        }
        // A manual-advance loop waits for the user: the banner is already up.
    }

    /// The batch is over, because of an error or a stop.
    pub(super) fn end_batch(&self) {
        let imp = self.imp();
        imp.batch.borrow_mut().clear();
        imp.batch_loop.set(false);
        imp.batch_total.set(0);
        imp.auto_scan_next.set(false);
    }

    /// Stop whatever the scanner is doing, and the batch with it.
    pub(super) fn stop(&self) {
        self.end_batch();
        self.imp().raw_only.set(false);
        if let Some(app) = self.application() {
            app.activate_action("cancel-scan", None);
        }
    }
}
