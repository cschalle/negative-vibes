//! The navigator (F§8): a small map of the whole picture with the part on
//! screen outlined. Click or drag on it to move the view.

use adw::prelude::*;
use adw::subclass::prelude::ObjectSubclassIsExt;

use super::Window;

const SIDE: usize = 200;

impl Window {
    pub(super) fn build_navigator(&self) -> gtk::MenuButton {
        let area = gtk::DrawingArea::builder().content_width(SIDE as i32).content_height(SIDE as i32).build();
        let weak = self.downgrade();
        area.set_draw_func(move |_, cr, w, h| {
            let Some(win) = weak.upgrade() else { return };
            let Some((tw, th, rgb)) = win.navigator_thumbnail() else { return };
            let (w, h) = (w as f64, h as f64);
            let scale = (w / tw as f64).min(h / th as f64);
            let (ox, oy) = ((w - tw as f64 * scale) / 2.0, (h - th as f64 * scale) / 2.0);
            let mut argb = Vec::with_capacity(tw * th * 4);
            for p in rgb.chunks_exact(3) {
                argb.extend_from_slice(&[p[2], p[1], p[0], 255]);
            }
            if let Ok(surface) = gtk::cairo::ImageSurface::create_for_data(argb, gtk::cairo::Format::ARgb32, tw as i32, th as i32, (tw * 4) as i32) {
                cr.save().ok();
                cr.translate(ox, oy);
                cr.scale(scale, scale);
                let _ = cr.set_source_surface(&surface, 0.0, 0.0);
                let _ = cr.paint();
                cr.restore().ok();
            }
            let [x0, y0, x1, y1] = win.viewport_fractions();
            cr.rectangle(ox + x0 * tw as f64 * scale, oy + y0 * th as f64 * scale, (x1 - x0) * tw as f64 * scale, (y1 - y0) * th as f64 * scale);
            cr.set_source_rgba(1.0, 0.85, 0.2, 0.25);
            let _ = cr.fill_preserve();
            cr.set_source_rgb(1.0, 0.85, 0.2);
            cr.set_line_width(2.0);
            let _ = cr.stroke();
        });

        // Press or drag: centre the view there.
        let drag = gtk::GestureDrag::builder().button(1).build();
        let go = |win: &Window, widget: &gtk::Widget, x: f64, y: f64| {
            let Some((tw, th, _)) = win.navigator_thumbnail() else { return };
            let (w, h) = (widget.width() as f64, widget.height() as f64);
            let scale = (w / tw as f64).min(h / th as f64);
            let (ox, oy) = ((w - tw as f64 * scale) / 2.0, (h - th as f64 * scale) / 2.0);
            let (fx, fy) = (((x - ox) / (tw as f64 * scale)).clamp(0.0, 1.0), ((y - oy) / (th as f64 * scale)).clamp(0.0, 1.0));
            win.centre_view_on(fx, fy);
            widget.queue_draw();
        };
        let weak = self.downgrade();
        drag.connect_drag_begin(move |g, x, y| {
            if let (Some(win), Some(w)) = (weak.upgrade(), g.widget()) { go(&win, &w, x, y); }
        });
        let weak = self.downgrade();
        drag.connect_drag_update(move |g, dx, dy| {
            if let (Some(win), Some(w), Some((sx, sy))) = (weak.upgrade(), g.widget(), g.start_point()) {
                let (tw, th, _) = match win.navigator_thumbnail() { Some(t) => t, None => return };
                let (ww, hh) = (w.width() as f64, w.height() as f64);
                let scale = (ww / tw as f64).min(hh / th as f64);
                let (ox, oy) = ((ww - tw as f64 * scale) / 2.0, (hh - th as f64 * scale) / 2.0);
                win.centre_view_on(((sx + dx - ox) / (tw as f64 * scale)).clamp(0.0, 1.0), ((sy + dy - oy) / (th as f64 * scale)).clamp(0.0, 1.0));
                w.queue_draw();
            }
        });
        area.add_controller(drag);

        let popover = gtk::Popover::builder().child(&area).build();
        // Redraw each time it opens: the picture may have changed.
        let a = area.clone();
        popover.connect_show(move |_| a.queue_draw());
        let button = gtk::MenuButton::builder().label("Map").tooltip_text("Navigator: where you are in a zoomed picture").margin_start(6).popover(&popover).build();
        *self.imp().navigator_area.borrow_mut() = Some(area);
        button
    }

    /// The rendered picture, oriented, reduced to fit a `SIDE` box.
    fn navigator_thumbnail(&self) -> Option<(usize, usize, Vec<u8>)> {
        let last = self.imp().last_rgb.borrow();
        let (w, h) = (last.0, last.1);
        if w == 0 || h == 0 {
            return None;
        }
        let step = (w.max(h) + SIDE - 1) / SIDE;
        let (tw, th) = (w / step, h / step);
        if tw == 0 || th == 0 {
            return None;
        }
        let mut small = Vec::with_capacity(tw * th * 3);
        for y in 0..th {
            for x in 0..tw {
                let i = ((y * step) * w + x * step) * 3;
                small.extend_from_slice(&last.2[i..i + 3]);
            }
        }
        let o = self.orientation();
        Some(if o.is_identity() { (tw, th, small) } else { let (ow, oh, px) = o.apply(&small, tw, th, 3); (ow, oh, px) })
    }

    /// The part of the picture on screen, as fractions `[x0, y0, x1, y1]`.
    fn viewport_fractions(&self) -> [f64; 4] {
        let imp = self.imp();
        let (Some(scroll), Some((iw, ih)), Some(z)) = (imp.canvas_scroll.borrow().clone(), self.canvas_image_size(), imp.zoom.get()) else {
            return [0.0, 0.0, 1.0, 1.0];
        };
        let (hadj, vadj) = (scroll.hadjustment(), scroll.vadjustment());
        let (fw, fh) = (iw * z, ih * z);
        let f = |v: f64, page: f64, total: f64| ((v / total).clamp(0.0, 1.0), ((v + page) / total).clamp(0.0, 1.0));
        let (x0, x1) = f(hadj.value(), hadj.page_size(), fw);
        let (y0, y1) = f(vadj.value(), vadj.page_size(), fh);
        [x0, y0, x1, y1]
    }

    fn centre_view_on(&self, fx: f64, fy: f64) {
        let imp = self.imp();
        let (Some(scroll), Some((iw, ih)), Some(z)) = (imp.canvas_scroll.borrow().clone(), self.canvas_image_size(), imp.zoom.get()) else { return };
        let (hadj, vadj) = (scroll.hadjustment(), scroll.vadjustment());
        hadj.set_value(fx * iw * z - hadj.page_size() / 2.0);
        vadj.set_value(fy * ih * z - vadj.page_size() / 2.0);
    }
}
