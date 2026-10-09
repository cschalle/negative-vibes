//! The colour wheel (F§5.5): one dot per tonal region, dragged towards the
//! hue to add. Angle is hue, distance from the centre is amount.

use std::cell::Cell;
use std::f64::consts::TAU;
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::ObjectSubclassIsExt;

use super::Window;

const REGIONS: [&str; 3] = ["Shadows", "Midtones", "Highlights"];

/// Hue to display colour (full saturation), for painting the ring.
fn hue_rgb(angle: f64) -> (f64, f64, f64) {
    let d = nv_process::colour::hue_direction(angle as f32);
    // A zero-sum push shown as a colour: lift each channel from grey.
    let c = |v: f32| (0.5 + 0.5 * v as f64).clamp(0.0, 1.0);
    (c(d[0]), c(d[1]), c(d[2]))
}

impl Window {
    /// Put the selected range's three values in the rows.
    pub(super) fn fill_selective_rows(&self, set: &[[f32; 3]; 6]) {
        let imp = self.imp();
        let guard = imp.updating_ui.replace(true);
        if let Some((range, rows)) = imp.selective_rows.borrow().as_ref() {
            let v = set[(range.selected() as usize).min(5)];
            for (r, x) in rows.iter().zip(v) {
                r.set_value(x as f64 * 100.0);
            }
        }
        imp.updating_ui.set(guard);
    }

    /// The range chosen changed, or the settings did: show the range's values.
    pub(super) fn show_selective_range(&self) {
        let set = self.imp().proc_params.borrow().as_ref().map(|p| p.colour.selective).unwrap_or([[0.0; 3]; 6]);
        self.fill_selective_rows(&set);
    }

    pub(super) fn build_colour_wheel(&self) -> gtk::Box {
        let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).build();

        let tabs = gtk::Box::builder().css_classes(["linked"]).homogeneous(true).build();
        let mut first: Option<gtk::ToggleButton> = None;
        for (i, name) in REGIONS.iter().enumerate() {
            let b = gtk::ToggleButton::builder().label(*name).active(i == 1).build();
            if let Some(f) = &first {
                b.set_group(Some(f));
            } else {
                first = Some(b.clone());
            }
            let weak = self.downgrade();
            b.connect_toggled(move |b| {
                if let (true, Some(win)) = (b.is_active(), weak.upgrade()) {
                    win.imp().wheel_region.set(i);
                    if let Some(a) = win.imp().wheel_area.borrow().as_ref() {
                        a.queue_draw();
                    }
                }
            });
            tabs.append(&b);
        }
        super::chrome::let_buttons_shrink(&tabs);
        root.append(&tabs);

        let area = gtk::DrawingArea::builder().content_width(220).content_height(220).halign(gtk::Align::Center).build();

        let weak = self.downgrade();
        area.set_draw_func(move |area, cr, w, h| {
            let Some(win) = weak.upgrade() else { return };
            let (w, h) = (w as f64, h as f64);
            let (cx, cy, r) = (w / 2.0, h / 2.0, w.min(h) / 2.0 - 6.0);
            // The hue ring, in thin wedges.
            let steps = 120;
            for i in 0..steps {
                let (a0, a1) = (TAU * i as f64 / steps as f64, TAU * (i + 1) as f64 / steps as f64 + 0.01);
                let (red, green, blue) = hue_rgb((a0 + a1) / 2.0);
                cr.move_to(cx, cy);
                cr.arc(cx, cy, r, a0, a1);
                cr.close_path();
                cr.set_source_rgba(red, green, blue, 0.85);
                let _ = cr.fill();
            }
            // Fade to the middle, so the centre reads as "nothing".
            let fade = gtk::cairo::RadialGradient::new(cx, cy, 0.0, cx, cy, r);
            let bg = area.color();
            let bgl = if bg.red() + bg.green() + bg.blue() > 1.5 { 0.14 } else { 0.97 };
            fade.add_color_stop_rgba(0.0, bgl, bgl, bgl, 1.0);
            fade.add_color_stop_rgba(1.0, bgl, bgl, bgl, 0.0);
            let _ = cr.set_source(&fade);
            cr.arc(cx, cy, r, 0.0, TAU);
            let _ = cr.fill();

            let wheel = win.imp().proc_params.borrow().as_ref().map(|p| p.colour.wheel).unwrap_or_default();
            let active = win.imp().wheel_region.get();
            for (i, v) in wheel.iter().enumerate() {
                // The angle is a hue, measured as on the ring (y up on screen is negative).
                let (x, y) = (cx + v[0] as f64 * r, cy + v[1] as f64 * r);
                cr.arc(x, y, if i == active { 7.0 } else { 4.0 }, 0.0, TAU);
                cr.set_source_rgba(1.0, 1.0, 1.0, if i == active { 1.0 } else { 0.55 });
                let _ = cr.fill_preserve();
                cr.set_source_rgba(0.0, 0.0, 0.0, 0.7);
                cr.set_line_width(1.5);
                let _ = cr.stroke();
            }
        });

        // A press or drag puts the active region's dot under the pointer.
        let live: Rc<Cell<bool>> = Rc::new(Cell::new(false));
        let drag = gtk::GestureDrag::builder().button(1).build();
        let place = |win: &Window, widget: &gtk::Widget, x: f64, y: f64| {
            let (w, h) = (widget.width() as f64, widget.height() as f64);
            let r = w.min(h) / 2.0 - 6.0;
            let (mut dx, mut dy) = ((x - w / 2.0) / r, (y - h / 2.0) / r);
            let len = dx.hypot(dy);
            if len > 1.0 {
                dx /= len;
                dy /= len;
            }
            // Snap the last few pixels to the centre so "no change" is easy to hit.
            if len < 0.03 {
                dx = 0.0;
                dy = 0.0;
            }
            let region = win.imp().wheel_region.get();
            win.update_live(|p| p.colour.wheel[region] = [dx as f32, dy as f32]);
            widget.queue_draw();
        };
        let (weak, l) = (self.downgrade(), live.clone());
        drag.connect_drag_begin(move |gesture, x, y| {
            let (Some(win), Some(widget)) = (weak.upgrade(), gesture.widget()) else { return };
            win.begin_live_edit();
            l.set(true);
            place(&win, &widget, x, y);
        });
        let weak = self.downgrade();
        drag.connect_drag_update(move |gesture, dx, dy| {
            let (Some(win), Some(widget), Some((sx, sy))) = (weak.upgrade(), gesture.widget(), gesture.start_point()) else { return };
            let (w, h) = (widget.width() as f64, widget.height() as f64);
            let r = w.min(h) / 2.0 - 6.0;
            let (mut vx, mut vy) = ((sx + dx - w / 2.0) / r, (sy + dy - h / 2.0) / r);
            let len = vx.hypot(vy);
            if len > 1.0 {
                vx /= len;
                vy /= len;
            }
            if len < 0.03 {
                vx = 0.0;
                vy = 0.0;
            }
            let region = win.imp().wheel_region.get();
            win.update_live(|p| p.colour.wheel[region] = [vx as f32, vy as f32]);
            widget.queue_draw();
        });
        let (weak, l) = (self.downgrade(), live);
        drag.connect_drag_end(move |_, _, _| {
            if let (Some(win), true) = (weak.upgrade(), l.replace(false)) {
                win.end_live_edit();
            }
        });
        area.add_controller(drag);

        // From the keyboard: arrows move the dot (Shift for bigger steps), Home or Delete centres it.
        area.set_focusable(true);
        area.set_tooltip_text(Some("Drag the dot towards the colour to add, or use the arrow keys; Home puts it back in the centre"));
        let keys = gtk::EventControllerKey::new();
        let weak = self.downgrade();
        keys.connect_key_pressed(move |ctrl, key, _, state| {
            use gtk::gdk::Key;
            let Some(win) = weak.upgrade() else { return glib::Propagation::Proceed };
            let step = if state.contains(gtk::gdk::ModifierType::SHIFT_MASK) { 0.2 } else { 0.05 };
            let (dx, dy, reset) = match key {
                Key::Left => (-step, 0.0, false),
                Key::Right => (step, 0.0, false),
                Key::Up => (0.0, -step, false),
                Key::Down => (0.0, step, false),
                Key::Home | Key::Delete | Key::BackSpace => (0.0, 0.0, true),
                _ => return glib::Propagation::Proceed,
            };
            let region = win.imp().wheel_region.get();
            let at = win.imp().proc_params.borrow().as_ref().map(|p| p.colour.wheel[region]).unwrap_or_default();
            let (mut x, mut y) = if reset { (0.0, 0.0) } else { (at[0] + dx as f32, at[1] + dy as f32) };
            let len = x.hypot(y);
            if len > 1.0 {
                x /= len;
                y /= len;
            }
            win.begin_live_edit();
            win.update_live(|p| p.colour.wheel[region] = [x, y]);
            win.end_live_edit();
            if let Some(w) = ctrl.widget() {
                w.queue_draw();
            }
            glib::Propagation::Stop
        });
        area.add_controller(keys);

        let reset = gtk::Button::builder().label("Reset Colour Balance").halign(gtk::Align::End).build();
        let weak = self.downgrade();
        reset.connect_clicked(move |_| {
            if let Some(win) = weak.upgrade() {
                win.begin_live_edit();
                win.update_live(|p| p.colour.wheel = [[0.0; 2]; 3]);
                win.end_live_edit();
                if let Some(a) = win.imp().wheel_area.borrow().as_ref() {
                    a.queue_draw();
                }
            }
        });

        *self.imp().wheel_area.borrow_mut() = Some(area.clone());
        root.append(&area);
        root.append(&reset);
        root
    }
}
