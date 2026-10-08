//! The 1:1 loupe (DESIGN.md §7.3, F§6 "1:1 loupe preview"): a window onto
//! the picture at the scan's own resolution, around the pointer, rendered
//! with every filter at full quality. The whole-frame preview works on a
//! reduced copy and leaves noise reduction out; this is where sharpening,
//! grain reduction and dust repair are judged.

use std::time::Duration;

use adw::prelude::*;
use adw::subclass::prelude::ObjectSubclassIsExt;
use gtk::gdk;
use nv_process::{tiles::freeze, ProcessParams, Proxy};
use nv_store::RawReader;

use super::Window;
use crate::canvas_geom as geom;

/// Side of the loupe in pixels of the finished picture.
const SIZE: u32 = 360;
/// Context read around the block, for what looks at neighbours (dust repair
/// 24, noise reduction 5, sharpening up to three radii).
const MARGIN: u32 = 40;
/// Wait for the pointer to rest before rendering.
const SETTLE: Duration = Duration::from_millis(120);

impl Window {
    /// The loupe's frame, an overlay in the corner of the canvas.
    pub(super) fn build_loupe(&self) -> gtk::Frame {
        let picture = gtk::Picture::builder().content_fit(gtk::ContentFit::ScaleDown).can_shrink(false).width_request(SIZE as i32).height_request(SIZE as i32).build();
        let frame = gtk::Frame::builder()
            .child(&picture)
            .halign(gtk::Align::End)
            .valign(gtk::Align::End)
            .margin_end(12)
            .margin_bottom(12)
            .visible(false)
            .css_classes(["card"])
            .build();
        frame.set_tooltip_text(Some("1:1 at the scan's resolution, with every filter at full quality"));
        *self.imp().loupe_picture.borrow_mut() = Some(picture);
        frame
    }

    /// Show or hide the loupe.
    pub(super) fn set_loupe(&self, on: bool) {
        self.imp().loupe_on.set(on);
        if let Some(f) = self.imp().loupe_frame.borrow().as_ref() {
            f.set_visible(on);
        }
        if on {
            self.schedule_loupe();
        }
    }

    /// The pointer moved over the picture: where the loupe looks.
    pub(super) fn setup_loupe_pointer(&self, area: &gtk::DrawingArea) {
        let motion = gtk::EventControllerMotion::new();
        let weak = self.downgrade();
        motion.connect_motion(move |ctrl, x, y| {
            let (Some(win), Some(widget)) = (weak.upgrade(), ctrl.widget()) else { return };
            if !win.imp().loupe_on.get() {
                return;
            }
            let Some((iw, ih)) = win.canvas_image_size() else { return };
            let c = geom::content_rect(widget.width() as f64, widget.height() as f64, iw, ih);
            if let Some(p) = geom::point_to_frac(x, y, c) {
                win.imp().loupe_pointer.set(Some(p));
                win.schedule_loupe();
            }
        });
        area.add_controller(motion);
    }

    /// Render the loupe soon, once the pointer (or the settings) have settled.
    pub(super) fn schedule_loupe(&self) {
        let imp = self.imp();
        if !imp.loupe_on.get() {
            return;
        }
        if let Some(id) = imp.loupe_timer.take() {
            id.remove();
        }
        let weak = self.downgrade();
        let id = glib::timeout_add_local_once(SETTLE, move || {
            if let Some(win) = weak.upgrade() {
                drop(win.imp().loupe_timer.take());
                win.update_loupe();
            }
        });
        *imp.loupe_timer.borrow_mut() = Some(id);
    }

    fn update_loupe(&self) {
        let imp = self.imp();
        let (Some(proxy), Some(params), Some((sw, sh))) =
            (imp.proxy.borrow().clone(), imp.proc_params.borrow().clone(), self.scan_image_size().map(|(w, h)| (w as u32, h as u32)))
        else {
            return;
        };
        // The pointer is on the picture as shown; the scan may be turned.
        let (fx, fy) = imp.loupe_pointer.get().unwrap_or((0.5, 0.5));
        let o = self.orientation();
        let p = o.rect_to_source([fx, fy, fx, fy]);
        let (cx, cy) = ((p[0] * sw as f64) as i64, (p[1] * sh as f64) as i64);

        // A whole block, even at the edge: slide it inwards rather than shrink it.
        let (bw, bh) = (SIZE.min(sw), SIZE.min(sh));
        let x0 = (cx - (SIZE / 2) as i64).clamp(0, (sw - bw) as i64) as u32;
        let y0 = (cy - (SIZE / 2) as i64).clamp(0, (sh - bh) as i64) as u32;
        let (rx0, ry0) = (x0.saturating_sub(MARGIN), y0.saturating_sub(MARGIN));
        let (rw, rh) = ((x0 + bw + MARGIN).min(sw) - rx0, (y0 + bh + MARGIN).min(sh) - ry0);

        // A preview is in memory; a big scan is read from disk, just this block.
        let in_memory = imp.scan_buf.borrow().as_ref().map(|b| b.crop(rx0, ry0, rw, rh));
        let path = imp.displayed_file.borrow().clone();
        let cached = imp.loupe_frozen.borrow().clone();

        imp.loupe_gen.set(imp.loupe_gen.get() + 1);
        let generation = imp.loupe_gen.get();
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let result = gio::spawn_blocking(move || render_block(&proxy, &params, cached, in_memory, path, (rx0, ry0, rw, rh), (x0, y0, bw, bh), o)).await;
            let Some(win) = weak.upgrade() else { return };
            if win.imp().loupe_gen.get() != generation {
                return; // the pointer moved on
            }
            match result {
                Ok(Some((frozen_key, w, h, rgb))) => {
                    *win.imp().loupe_frozen.borrow_mut() = Some(frozen_key);
                    win.show_loupe(w, h, &rgb);
                }
                _ => {}
            }
        });
    }

    fn show_loupe(&self, w: usize, h: usize, rgb: &[u8]) {
        let Some(pic) = self.imp().loupe_picture.borrow().clone() else { return };
        let rgba: Vec<u8> = rgb.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect();
        let texture = gdk::MemoryTexture::new(w as i32, h as i32, gdk::MemoryFormat::R8g8b8a8, &glib::Bytes::from_owned(rgba), w * 4);
        pic.set_paintable(Some(texture.upcast_ref::<gdk::Paintable>()));
    }
}

/// What the worker thread does: read the block, render it with the frozen
/// parameters, and cut out the loupe's block, turned as the picture is.
/// Returns the frozen parameters too, so they are not measured again while
/// nothing changes.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn render_block(
    proxy: &std::sync::Arc<Proxy>,
    params: &ProcessParams,
    cached: Option<(u64, ProcessParams, ProcessParams)>,
    in_memory: Option<nv_image::FrameBuf>,
    path: Option<std::path::PathBuf>,
    (rx0, ry0, rw, rh): (u32, u32, u32, u32),
    (x0, y0, bw, bh): (u32, u32, u32, u32),
    orientation: nv_process::Orientation,
) -> Option<((u64, ProcessParams, ProcessParams), usize, usize, Vec<u8>)> {
    let region = match in_memory {
        Some(r) => r,
        None => RawReader::region(&path?, rx0, ry0, rw, rh).ok()?,
    };
    if region.width == 0 {
        return None;
    }
    let frozen = match cached {
        Some((id, p, f)) if id == proxy.id && p == *params => f,
        _ => freeze(proxy, params),
    };
    let block = Proxy::from_raw_region(&region, 0, 0, region.width as usize, region.effective_height() as usize);
    let toned = nv_process::render_toned(&block, &frozen);

    // Cut the block out of the region that was rendered with context around it.
    let (ox, oy) = ((x0 - rx0) as usize, (y0 - ry0) as usize);
    let (cw, ch) = (bw.min(region.width - ox as u32) as usize, bh.min(region.effective_height() - oy as u32) as usize);
    let mut cut = Vec::with_capacity(cw * ch * 3);
    for y in 0..ch {
        let row = ((oy + y) * region.width as usize + ox) * 3;
        cut.extend_from_slice(&toned[row..row + cw * 3]);
    }
    let rgb = nv_process::encode_srgb8(&cut);
    let (w, h, rgb) = orientation.apply(&rgb, cw, ch, 3);
    Some(((proxy.id, params.clone(), frozen), w, h, rgb))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nv_film::builtin_profiles;
    use nv_process::{default_params_for, DustMode, NoiseLevel, Orientation, UsmParams};
    use std::sync::Arc;

    fn tulip() -> nv_image::FrameBuf {
        RawReader::read(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/plustek8200i/tulip-600.pnm")).unwrap()
    }

    /// The loupe's block is the same block of the whole picture, rendered at
    /// full quality: same colours, sharpening and noise reduction as an
    /// export would give there, with the right orientation, and only the
    /// block (not the margin it was rendered with) comes back.
    #[test]
    fn the_loupe_shows_what_the_whole_picture_shows_at_that_place() {
        let raw = tulip();
        let proxy = Arc::new(Proxy::from_raw(&raw, nv_process::DEFAULT_PROXY_PIXELS));
        let mut params = default_params_for(&builtin_profiles()[0]);
        params.dust.mode = DustMode::Off; // dust measures each region for itself
        params.detail.usm = UsmParams { enabled: true, amount: 200.0, radius: 1.6, threshold: 0 };
        params.detail.noise = NoiseLevel::Light;
        let frozen = freeze(&proxy, &params);

        let (x0, y0, bw, bh) = (300u32, 200u32, 180u32, 120u32);
        let (rx0, ry0) = (x0 - MARGIN, y0 - MARGIN);
        let (rw, rh) = (bw + 2 * MARGIN, bh + 2 * MARGIN);
        for o in [Orientation::IDENTITY, Orientation { rotation: 1, flip_h: true, flip_v: false }] {
            let (_, w, h, got) = render_block(&proxy, &params, None, Some(raw.crop(rx0, ry0, rw, rh)), None, (rx0, ry0, rw, rh), (x0, y0, bw, bh), o).expect("a block");
            assert_eq!(o.output_size(bw as usize, bh as usize), (w, h), "{o:?}");

            // The same block, cut from the whole picture rendered at once.
            let whole = nv_process::render_toned(&Proxy::from_raw(&raw, usize::MAX), &frozen);
            let mut cut = Vec::new();
            for y in y0..y0 + bh {
                cut.extend_from_slice(&whole[((y * raw.width + x0) * 3) as usize..][..bw as usize * 3]);
            }
            let (_, _, want) = o.apply(&nv_process::encode_srgb8(&cut), bw as usize, bh as usize, 3);
            let worst = got.iter().zip(&want).map(|(a, b)| (*a as i32 - *b as i32).abs()).max().unwrap();
            assert!(worst <= 1, "{o:?}: the loupe differs from the picture by {worst} levels");
        }
    }

    #[test]
    fn a_block_outside_the_scan_gives_nothing() {
        let raw = tulip();
        let proxy = Arc::new(Proxy::from_raw(&raw, nv_process::DEFAULT_PROXY_PIXELS));
        let params = default_params_for(&builtin_profiles()[0]);
        assert!(render_block(&proxy, &params, None, Some(raw.crop(5000, 5000, 100, 100)), None, (5000, 5000, 100, 100), (5000, 5000, 100, 100), Orientation::IDENTITY).is_none());
    }
}
