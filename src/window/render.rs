//! Rendering the canvas from the editing proxy, off the UI thread, and the
//! histogram of what was rendered.

use std::sync::Arc;

use adw::prelude::*;
use adw::subclass::prelude::ObjectSubclassIsExt;
use gtk::gdk;
use nv_process::{DEFAULT_PROXY_PIXELS, Histogram, Proxy};

use super::Window;

impl Window {
    /// Decode the scan on the canvas into a proxy (small scans: right here).
    pub(super) fn rebuild_proxy(&self) {
        let proxy = self.imp().scan_buf.borrow().as_ref().map(|b| Arc::new(Proxy::from_raw(b, DEFAULT_PROXY_PIXELS)));
        *self.imp().proxy.borrow_mut() = proxy;
    }

    /// Render the proxy with the current parameters. Only one render runs at
    /// a time; changes that arrive meanwhile collapse into one more render
    /// with the latest parameters, so a dragged slider never queues up work.
    pub(super) fn request_render(&self) {
        let imp = self.imp();
        if imp.render_busy.get() {
            imp.render_dirty.set(true);
            return;
        }
        let (Some(proxy), Some(params)) = (imp.proxy.borrow().clone(), imp.proc_params.borrow().clone()) else {
            return;
        };
        imp.render_busy.set(true);
        imp.render_dirty.set(false);
        let cache = imp.render_cache.clone();

        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let done = gio::spawn_blocking(move || {
                let out = nv_process::render_cached(&proxy, &params, &cache);
                let hist = Histogram::from_rgb8(&out.rgb);
                (proxy.width, proxy.height, out.rgb, hist, out.levels)
            })
            .await;
            let Some(win) = weak.upgrade() else { return };
            win.imp().render_busy.set(false);
            if let Ok((w, h, rgb, hist, levels)) = done {
                win.show_rgb8(w, h, &rgb);
                *win.imp().histogram.borrow_mut() = Some(hist);
                *win.imp().levels.borrow_mut() = Some(levels);
                for area in [win.imp().levels_area.borrow(), win.imp().curve_area.borrow(), win.imp().wheel_area.borrow()] {
                    if let Some(a) = area.as_ref() {
                        a.queue_draw();
                    }
                }
                if let Some(area) = win.imp().histogram_area.borrow().as_ref() {
                    area.queue_draw();
                }
            }
            if win.imp().render_dirty.replace(false) {
                win.request_render();
            }
        });
    }

    /// Put 8-bit RGB pixels on the canvas.
    pub(super) fn show_rgb8(&self, w: usize, h: usize, rgb: &[u8]) {
        if w == 0 || h == 0 || rgb.len() < w * h * 3 {
            return;
        }
        *self.imp().last_rgb.borrow_mut() = (w, h, rgb[..w * h * 3].to_vec());
        // Show the picture through the frame's rotation and flips.
        let oriented;
        let (w, h, rgb) = {
            let o = self.orientation();
            if o.is_identity() {
                (w, h, rgb)
            } else {
                let (ow, oh, px) = o.apply(&rgb[..w * h * 3], w, h, 3);
                oriented = px;
                (ow, oh, &oriented[..])
            }
        };
        let straightened;
        let rgb = if self.angle().abs() >= 0.005 {
            straightened = nv_process::straighten::straighten_u8(&rgb[..w * h * 3], w, h, 3, self.angle());
            &straightened[..]
        } else {
            rgb
        };
        // R8G8B8 (3-byte packed) is not natively supported by most GPU
        // drivers, and GTK's conversion corrupts it on Wayland/Vulkan; R8G8B8A8
        // is universally supported.
        let rgba: Vec<u8> = rgb[..w * h * 3].as_chunks::<3>().0.iter().flat_map(|px| [px[0], px[1], px[2], 255u8]).collect();
        let texture = gdk::MemoryTexture::new(w as i32, h as i32, gdk::MemoryFormat::R8g8b8a8, &glib::Bytes::from_owned(rgba), w * 4);
        if let Some(pic) = self.imp().canvas_picture.borrow().as_ref() {
            pic.set_paintable(Some(texture.upcast_ref::<gdk::Paintable>()));
        }
    }

    /// The histogram strip at the top of the inspector.
    pub(super) fn build_histogram(&self) -> gtk::DrawingArea {
        let area = gtk::DrawingArea::builder().content_height(110).margin_start(12).margin_end(12).margin_bottom(6).build();
        area.add_css_class("card");

        let weak = self.downgrade();
        area.set_draw_func(move |area, cr, w, h| {
            let Some(win) = weak.upgrade() else { return };
            let hist = win.imp().histogram.borrow();
            let Some(hist) = hist.as_ref() else { return };
            let (w, h) = (w as f64, h as f64);
            let peak = hist.peak().max(1) as f64;
            let bar = w / 256.0;

            for (chan, rgb) in [(&hist.r, (0.9, 0.2, 0.2)), (&hist.g, (0.2, 0.7, 0.3)), (&hist.b, (0.25, 0.4, 0.95))] {
                cr.set_source_rgba(rgb.0, rgb.1, rgb.2, 0.45);
                cr.move_to(0.0, h);
                for (i, n) in chan.iter().enumerate() {
                    // A square root keeps a few tall bins from hiding the rest.
                    let y = h - (*n as f64 / peak).sqrt().min(1.0) * (h - 4.0);
                    cr.line_to(i as f64 * bar, y);
                    cr.line_to((i + 1) as f64 * bar, y);
                }
                cr.line_to(w, h);
                cr.close_path();
                let _ = cr.fill();
            }

            let c = area.color();
            cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, 0.8);
            cr.set_line_width(1.0);
            for (i, n) in hist.luma.iter().enumerate() {
                let y = h - (*n as f64 / peak).sqrt().min(1.0) * (h - 4.0);
                if i == 0 { cr.move_to(0.0, y) } else { cr.line_to(i as f64 * bar, y) }
            }
            let _ = cr.stroke();
        });
        area
    }
}
