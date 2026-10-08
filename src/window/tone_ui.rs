//! Levels and curves: widgets that are dragged, with live preview while the
//! mouse is down and one undo step when it is released.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::ObjectSubclassIsExt;
use nv_process::{Curve, ProcessParams, ToneParams};

use super::Window;

/// Pixels within which a click grabs a handle or a point.
const GRAB: f64 = 12.0;
/// The perceptual encoding the widgets draw in.
const GAMMA: f64 = 2.2;

fn channel_curve(t: &mut ToneParams, ch: u8) -> &mut Curve {
    match ch {
        1 => &mut t.curves.r,
        2 => &mut t.curves.g,
        3 => &mut t.curves.b,
        _ => &mut t.curves.master,
    }
}

impl Window {
    // ── Live edits ────────────────────────────────────────────────────────────

    /// A drag starts: remember where the edit began.
    fn begin_live_edit(&self) {
        *self.imp().live_old.borrow_mut() = self.imp().proc_params.borrow().clone();
    }

    /// Change the parameters while the mouse is down: show the result, but
    /// leave undo and the roll for when it is released.
    fn update_live(&self, f: impl FnOnce(&mut ProcessParams)) {
        let Some(mut p) = self.imp().proc_params.borrow().clone() else { return };
        f(&mut p);
        self.sync_widgets(&p);
        *self.imp().proc_params.borrow_mut() = Some(p);
        // The handles follow the mouse now, not when the render lands.
        for area in [self.imp().levels_area.borrow(), self.imp().curve_area.borrow()] {
            if let Some(a) = area.as_ref() { a.queue_draw(); }
        }
        self.refresh_canvas();
    }

    /// The drag ends: one undo step from where it began.
    fn end_live_edit(&self) {
        let (Some(old), Some(new)) = (self.imp().live_old.borrow_mut().take(), self.imp().proc_params.borrow().clone()) else { return };
        if old == new {
            return;
        }
        {
            let mut stack = self.imp().undo_stack.borrow_mut();
            let at = self.imp().undo_idx.get();
            stack.truncate(at + 1);
            stack.push(new.clone());
            self.imp().undo_idx.set(stack.len() - 1);
        }
        self.record_edit(&old, &new);
    }

    // ── Levels ────────────────────────────────────────────────────────────────

    /// The picture's tonal range with the black and white points on it.
    pub(super) fn build_levels(&self) -> gtk::DrawingArea {
        let area = gtk::DrawingArea::builder().content_height(96).margin_top(6).margin_bottom(6).build();
        area.add_css_class("card");

        let weak = self.downgrade();
        area.set_draw_func(move |area, cr, w, h| {
            let Some(win) = weak.upgrade() else { return };
            let (w, h) = (w as f64, h as f64 - 14.0);
            let fg = area.color();
            let tone = win.imp().proc_params.borrow().as_ref().map(|p| p.tone.clone()).unwrap_or_default();

            if let Some(levels) = win.imp().levels.borrow().as_ref() {
                let peak = levels[1..255].iter().copied().max().unwrap_or(1).max(1) as f64;
                let bar = w / 256.0;
                cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.45);
                for (i, n) in levels.iter().enumerate() {
                    let bh = (*n as f64 / peak).sqrt().min(1.0) * (h - 4.0);
                    cr.rectangle(i as f64 * bar, h - bh, bar.max(1.0), bh);
                }
                let _ = cr.fill();
            }

            // What the points cut off.
            let (bx, wx) = (enc(tone.black_point) * w, enc(tone.white_point) * w);
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.35);
            cr.rectangle(0.0, 0.0, bx, h);
            cr.rectangle(wx, 0.0, w - wx, h);
            let _ = cr.fill();

            for (x, dark) in [(bx, true), (wx, false)] {
                cr.move_to(x, h);
                cr.line_to(x - 6.0, h + 12.0);
                cr.line_to(x + 6.0, h + 12.0);
                cr.close_path();
                if dark { cr.set_source_rgb(0.1, 0.1, 0.1) } else { cr.set_source_rgb(0.95, 0.95, 0.95) }
                let _ = cr.fill_preserve();
                cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.8);
                cr.set_line_width(1.0);
                let _ = cr.stroke();
            }
        });

        // Drag a point: which one is decided where the press lands.
        let grab: Rc<Cell<Option<bool>>> = Rc::new(Cell::new(None)); // Some(true) = black
        let drag = gtk::GestureDrag::builder().button(gdk_primary()).build();
        let (weak, g) = (self.downgrade(), grab.clone());
        drag.connect_drag_begin(move |gesture, x, _| {
            let (Some(win), Some(widget)) = (weak.upgrade(), gesture.widget()) else { return };
            let tone = win.imp().proc_params.borrow().as_ref().map(|p| p.tone.clone()).unwrap_or_default();
            let w = widget.width() as f64;
            let (db, dw) = ((x - enc(tone.black_point) * w).abs(), (x - enc(tone.white_point) * w).abs());
            // Overlapping points: take the one the press is on the outer side of.
            g.set(if db.min(dw) > GRAB { None } else { Some(db < dw || (db == dw && x < w / 2.0)) });
            if g.get().is_some() {
                win.begin_live_edit();
            }
        });
        let (weak, g) = (self.downgrade(), grab.clone());
        drag.connect_drag_update(move |gesture, dx, _| {
            let (Some(win), Some(widget), Some(which), Some((sx, _))) = (weak.upgrade(), gesture.widget(), g.get(), gesture.start_point()) else { return };
            let v = dec(((sx + dx) / widget.width() as f64).clamp(0.0, 1.0));
            win.update_live(|p| {
                if which {
                    p.tone.black_point = v.clamp(0.0, p.tone.white_point as f64 - 0.01) as f32;
                } else {
                    p.tone.white_point = v.clamp(p.tone.black_point as f64 + 0.01, 1.0) as f32;
                }
            });
        });
        let (weak, g) = (self.downgrade(), grab);
        drag.connect_drag_end(move |_, _, _| {
            if let (Some(win), Some(_)) = (weak.upgrade(), g.take()) {
                win.end_live_edit();
            }
        });
        area.add_controller(drag);
        area
    }

    // ── Curves ────────────────────────────────────────────────────────────────

    pub(super) fn build_curves(&self) -> gtk::Box {
        let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).build();

        let tabs = gtk::Box::builder().css_classes(["linked"]).homogeneous(true).build();
        let mut first: Option<gtk::ToggleButton> = None;
        for (ch, name) in [(0u8, "Master"), (1, "Red"), (2, "Green"), (3, "Blue")] {
            let b = gtk::ToggleButton::builder().label(name).active(ch == 0).build();
            if let Some(f) = &first { b.set_group(Some(f)); } else { first = Some(b.clone()); }
            let weak = self.downgrade();
            b.connect_toggled(move |b| {
                if let (true, Some(win)) = (b.is_active(), weak.upgrade()) {
                    win.imp().curve_channel.set(ch);
                    if let Some(a) = win.imp().curve_area.borrow().as_ref() { a.queue_draw(); }
                }
            });
            tabs.append(&b);
        }
        root.append(&tabs);

        let area = gtk::DrawingArea::builder().content_width(240).content_height(240).halign(gtk::Align::Center).build();
        area.add_css_class("card");
        area.set_tooltip_text(Some("Click to add a point, drag to move it, right-click to remove it"));

        let weak = self.downgrade();
        area.set_draw_func(move |area, cr, w, h| {
            let Some(win) = weak.upgrade() else { return };
            let (w, h) = (w as f64, h as f64);
            let fg = area.color();
            let (fr, fgc, fb) = (fg.red() as f64, fg.green() as f64, fg.blue() as f64);
            let ch = win.imp().curve_channel.get();

            cr.set_source_rgba(fr, fgc, fb, 0.12);
            cr.set_line_width(1.0);
            for i in 1..4 {
                let (x, y) = (w * i as f64 / 4.0, h * i as f64 / 4.0);
                cr.move_to(x, 0.0); cr.line_to(x, h);
                cr.move_to(0.0, y); cr.line_to(w, y);
            }
            let _ = cr.stroke();
            cr.set_source_rgba(fr, fgc, fb, 0.25);
            cr.move_to(0.0, h); cr.line_to(w, 0.0);
            let _ = cr.stroke();

            let mut tone = win.imp().proc_params.borrow().as_ref().map(|p| p.tone.clone()).unwrap_or_default();
            let curve = channel_curve(&mut tone, ch).clone();
            let (r, g, b) = match ch { 1 => (0.9, 0.2, 0.2), 2 => (0.2, 0.65, 0.3), 3 => (0.25, 0.4, 0.95), _ => (fr, fgc, fb) };
            cr.set_source_rgb(r, g, b);
            cr.set_line_width(2.0);
            let lut = curve.lut(256);
            for (i, y) in lut.iter().enumerate() {
                let (px, py) = (i as f64 / 255.0 * w, h - *y as f64 * h);
                if i == 0 { cr.move_to(px, py) } else { cr.line_to(px, py) }
            }
            let _ = cr.stroke();
            for p in &curve.0 {
                cr.arc(p[0] as f64 * w, h - p[1] as f64 * h, 4.5, 0.0, std::f64::consts::TAU);
                cr.set_source_rgb(r, g, b);
                let _ = cr.fill_preserve();
                cr.set_source_rgba(0.0, 0.0, 0.0, 0.6);
                cr.set_line_width(1.0);
                let _ = cr.stroke();
            }
        });

        // Press on a point grabs it; press elsewhere adds one there and grabs it.
        let grab: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
        let drag = gtk::GestureDrag::builder().button(gdk_primary()).build();
        let (weak, g) = (self.downgrade(), grab.clone());
        drag.connect_drag_begin(move |gesture, x, y| {
            let (Some(win), Some(widget)) = (weak.upgrade(), gesture.widget()) else { return };
            let (w, h) = (widget.width() as f64, widget.height() as f64);
            let ch = win.imp().curve_channel.get();
            let mut tone = win.imp().proc_params.borrow().as_ref().map(|p| p.tone.clone()).unwrap_or_default();
            let pts = channel_curve(&mut tone, ch).0.clone();

            let near = pts.iter().position(|p| ((p[0] as f64 * w - x).powi(2) + ((1.0 - p[1] as f64) * h - y).powi(2)).sqrt() <= GRAB);
            win.begin_live_edit();
            let grabbed = match near {
                Some(i) => i,
                None => {
                    let (nx, ny) = ((x / w).clamp(0.0, 1.0) as f32, (1.0 - y / h).clamp(0.0, 1.0) as f32);
                    let at = pts.iter().position(|p| p[0] > nx).unwrap_or(pts.len());
                    // Not on top of an end or a neighbour: there is no room for a point.
                    if at == 0 || at == pts.len() || nx - pts[at - 1][0] < 0.02 || pts[at][0] - nx < 0.02 {
                        win.imp().live_old.borrow_mut().take();
                        g.set(None);
                        return;
                    }
                    win.update_live(|p| channel_curve(&mut p.tone, ch).0.insert(at, [nx, ny]));
                    at
                }
            };
            g.set(Some(grabbed));
        });
        let (weak, g) = (self.downgrade(), grab.clone());
        drag.connect_drag_update(move |gesture, dx, dy| {
            let (Some(win), Some(widget), Some(i), Some((sx, sy))) = (weak.upgrade(), gesture.widget(), g.get(), gesture.start_point()) else { return };
            let (w, h) = (widget.width() as f64, widget.height() as f64);
            let ch = win.imp().curve_channel.get();
            let ny = (1.0 - (sy + dy) / h).clamp(0.0, 1.0) as f32;
            let nx = ((sx + dx) / w).clamp(0.0, 1.0) as f32;
            win.update_live(|p| {
                let pts = &mut channel_curve(&mut p.tone, ch).0;
                if i >= pts.len() { return; }
                // The ends keep their x; inner points stay between their neighbours.
                let x = if i == 0 || i + 1 == pts.len() { pts[i][0] } else { nx.clamp(pts[i - 1][0] + 0.01, pts[i + 1][0] - 0.01) };
                pts[i] = [x, ny];
            });
            if let Some(a) = win.imp().curve_area.borrow().as_ref() { a.queue_draw(); }
        });
        let (weak, g) = (self.downgrade(), grab);
        drag.connect_drag_end(move |_, _, _| {
            if let (Some(win), Some(_)) = (weak.upgrade(), g.take()) {
                win.end_live_edit();
            }
        });
        area.add_controller(drag);

        // Right-click removes an inner point.
        let remove = gtk::GestureClick::builder().button(3).build();
        let weak = self.downgrade();
        remove.connect_pressed(move |gesture, _, x, y| {
            let (Some(win), Some(widget)) = (weak.upgrade(), gesture.widget()) else { return };
            let (w, h) = (widget.width() as f64, widget.height() as f64);
            let ch = win.imp().curve_channel.get();
            let mut tone = win.imp().proc_params.borrow().as_ref().map(|p| p.tone.clone()).unwrap_or_default();
            let pts = channel_curve(&mut tone, ch).0.clone();
            let hit = pts.iter().enumerate().position(|(i, p)| {
                i > 0 && i + 1 < pts.len() && ((p[0] as f64 * w - x).powi(2) + ((1.0 - p[1] as f64) * h - y).powi(2)).sqrt() <= GRAB
            });
            if let Some(i) = hit {
                win.begin_live_edit();
                win.update_live(|p| { channel_curve(&mut p.tone, ch).0.remove(i); });
                win.end_live_edit();
                widget.queue_draw();
            }
        });
        area.add_controller(remove);

        let reset = gtk::Button::builder().label("Reset Curves").halign(gtk::Align::End).build();
        let weak = self.downgrade();
        reset.connect_clicked(move |_| {
            if let Some(win) = weak.upgrade() {
                win.begin_live_edit();
                win.update_live(|p| p.tone.curves = Default::default());
                win.end_live_edit();
                if let Some(a) = win.imp().curve_area.borrow().as_ref() { a.queue_draw(); }
            }
        });

        *self.imp().curve_area.borrow_mut() = Some(area.clone());
        root.append(&area);
        root.append(&reset);
        root
    }
}

fn gdk_primary() -> u32 {
    gtk::gdk::BUTTON_PRIMARY
}

/// Linear light to the position on the levels axis.
fn enc(v: f32) -> f64 {
    (v.max(0.0) as f64).powf(1.0 / GAMMA)
}

/// The position on the levels axis to linear light.
fn dec(x: f64) -> f64 {
    x.powf(GAMMA)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_levels_axis_round_trips() {
        for v in [0.0f32, 0.05, 0.18, 0.5, 1.0] {
            assert!((dec(enc(v)) - v as f64).abs() < 1e-6);
        }
        // Middle grey sits near the middle of the axis, not at a fifth of it.
        assert!((enc(0.18) - 0.46).abs() < 0.01);
    }
}
