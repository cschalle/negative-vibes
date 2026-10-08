//! Rotating and flipping the picture of the selected frame. The scan is
//! never touched: the frame keeps an orientation, and the canvas, the
//! thumbnails and exports show the picture through it.

use adw::prelude::*;
use adw::subclass::prelude::ObjectSubclassIsExt;
use nv_process::{DisplayOp, Orientation};

use super::Window;

impl Window {
    /// The orientation of frame `idx`.
    pub(super) fn orientation_of(&self, idx: u32) -> Orientation {
        self.imp()
            .roll
            .borrow()
            .as_ref()
            .and_then(|r| r.frame(idx))
            .map(|f| Orientation { rotation: f.geometry.rotation % 4, flip_h: f.geometry.flip_h, flip_v: f.geometry.flip_v })
            .unwrap_or_default()
    }

    /// The orientation of the picture on the canvas: the selected frame's, or
    /// the one a loose file (opened without a roll) was given.
    pub(super) fn orientation(&self) -> Orientation {
        match *self.imp().selected_frame.borrow() {
            Some(idx) if self.imp().roll.borrow().as_ref().is_some_and(|r| r.frame(idx).is_some()) => self.orientation_of(idx),
            _ => self.imp().loose_orientation.get(),
        }
    }

    /// The fine straightening of frame `idx`, in degrees clockwise.
    pub(super) fn angle_of(&self, idx: u32) -> f32 {
        self.imp().roll.borrow().as_ref().and_then(|r| r.frame(idx)).map_or(0.0, |f| f.geometry.angle)
    }

    /// The straightening of the picture on the canvas.
    pub(super) fn angle(&self) -> f32 {
        match *self.imp().selected_frame.borrow() {
            Some(idx) if self.imp().roll.borrow().as_ref().is_some_and(|r| r.frame(idx).is_some()) => self.angle_of(idx),
            _ => self.imp().loose_angle.get(),
        }
    }

    pub(super) fn set_angle(&self, degrees: f32) {
        let imp = self.imp();
        let degrees = degrees.clamp(-nv_process::straighten::MAX_ANGLE, nv_process::straighten::MAX_ANGLE);
        match *imp.selected_frame.borrow() {
            Some(idx) if imp.roll.borrow().as_ref().is_some_and(|r| r.frame(idx).is_some()) => {
                if let Some(f) = imp.roll.borrow_mut().as_mut().and_then(|r| r.frame_mut(idx)) {
                    f.geometry.angle = degrees;
                }
                self.schedule_autosave();
                self.spawn_thumbnails(vec![idx]);
            }
            _ => imp.loose_angle.set(degrees),
        }
        self.refresh_canvas();
        self.schedule_hq();
    }

    /// A point of the picture as shown, as a point of the oriented scan.
    pub(super) fn unstraighten(&self, fx: f64, fy: f64) -> (f64, f64) {
        match self.canvas_image_size() {
            Some(size) => nv_process::straighten::display_to_picture(self.angle(), size, fx, fy),
            None => (fx, fy),
        }
    }

    /// The inverse of [`unstraighten`](Self::unstraighten).
    pub(super) fn straighten_point(&self, px: f64, py: f64) -> (f64, f64) {
        match self.canvas_image_size() {
            Some(size) => nv_process::straighten::picture_to_display(self.angle(), size, px, py),
            None => (px, py),
        }
    }

    /// Whether there is a picture on the canvas.
    pub(super) fn has_image(&self) -> bool {
        self.imp().scan_buf.borrow().is_some() || self.imp().scan_dims.get().is_some()
    }

    /// Size of the scan on the canvas, in scan pixels.
    pub(super) fn scan_image_size(&self) -> Option<(f64, f64)> {
        if let Some(buf) = self.imp().scan_buf.borrow().as_ref() {
            let h = buf.effective_height();
            return (buf.width > 0 && h > 0).then(|| (buf.width as f64, h as f64));
        }
        self.imp().scan_dims.get().map(|(w, h)| (w as f64, h as f64))
    }

    fn turn(&self, op: DisplayOp) {
        let imp = self.imp();
        let next = self.orientation().then(op).canonical();
        match *imp.selected_frame.borrow() {
            Some(idx) if imp.roll.borrow().as_ref().is_some_and(|r| r.frame(idx).is_some()) => {
                if let Some(f) = imp.roll.borrow_mut().as_mut().and_then(|r| r.frame_mut(idx)) {
                    f.geometry.rotation = next.rotation;
                    f.geometry.flip_h = next.flip_h;
                    f.geometry.flip_v = next.flip_v;
                }
                self.schedule_autosave();
                self.spawn_thumbnails(vec![idx]);
            }
            _ => imp.loose_orientation.set(next),
        }
        // The picture changes shape: redraw, resize the canvas and the rectangle.
        self.refresh_canvas();
        self.apply_zoom();
        if let Some(a) = imp.frame_overlay.borrow().as_ref() {
            a.queue_draw();
        }
        self.update_info_label();
    }

    pub(super) fn setup_orientation(&self) {
        let actions: [(&str, DisplayOp); 4] = [
            ("rotate-left", DisplayOp::RotateCcw),
            ("rotate-right", DisplayOp::RotateCw),
            ("flip-horizontal", DisplayOp::FlipHorizontal),
            ("flip-vertical", DisplayOp::FlipVertical),
        ];
        for (name, op) in actions {
            let action = gio::SimpleAction::new(name, None);
            let weak = self.downgrade();
            action.connect_activate(move |_, _| {
                if let Some(win) = weak.upgrade() {
                    win.turn(op);
                }
            });
            self.add_action(&action);
        }
    }
}
