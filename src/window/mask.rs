//! Brush masks for dust removal (F§6 "Mask"): paint where it must leave the
//! picture alone (Protect: stars, eyelashes, anything fine that looks like
//! dust), where it must repair (Heal), and take strokes back (Unmask). The
//! strokes belong to the frame and are kept as fractions of the scan, so
//! they hold at any zoom, in the loupe, and in an export.

use adw::prelude::*;
use adw::subclass::prelude::ObjectSubclassIsExt;
use nv_process::dust::erase_touching;
use nv_process::Stroke;

use super::imp::CanvasTool;
use super::Window;
use crate::canvas_geom as geom;

pub(super) fn is_mask_tool(t: CanvasTool) -> bool {
    matches!(t, CanvasTool::Protect | CanvasTool::Heal | CanvasTool::Unmask)
}

/// Colours the strokes are drawn in: green for protected, red for healed.
const PROTECT: (f64, f64, f64) = (0.15, 0.75, 0.30);
const HEAL: (f64, f64, f64) = (0.90, 0.20, 0.20);
const ERASE: (f64, f64, f64) = (0.95, 0.95, 0.95);

impl Window {
    /// The brush radius, as a fraction of the scan's longer side.
    fn brush_radius(&self) -> f32 {
        let pct = self.imp().brush_row.borrow().as_ref().map_or(0.8, |r| r.value()) as f32;
        pct / 100.0
    }

    /// A point on the picture as shown (fractions of it) as a point on the scan.
    fn display_to_scan(&self, fx: f64, fy: f64) -> [f32; 2] {
        let p = self.orientation().rect_to_source([fx, fy, fx, fy]);
        [p[0] as f32, p[1] as f32]
    }

    pub(super) fn begin_stroke(&self, fx: f64, fy: f64) {
        *self.imp().stroke_points.borrow_mut() = vec![self.display_to_scan(fx, fy)];
        self.queue_overlay();
    }

    pub(super) fn extend_stroke(&self, fx: f64, fy: f64) {
        let p = self.display_to_scan(fx, fy);
        let mut pts = self.imp().stroke_points.borrow_mut();
        // A point only when the brush has moved a little: long drags stay light.
        if pts.last().is_none_or(|l| (l[0] - p[0]).hypot(l[1] - p[1]) > self.brush_radius() * 0.3) {
            pts.push(p);
        }
        drop(pts);
        self.queue_overlay();
    }

    fn queue_overlay(&self) {
        if let Some(a) = self.imp().frame_overlay.borrow().as_ref() {
            a.queue_draw();
        }
    }

    /// The stroke is finished: it becomes part of the frame's dust settings,
    /// one undoable step.
    pub(super) fn end_stroke(&self) {
        let points = std::mem::take(&mut *self.imp().stroke_points.borrow_mut());
        if points.is_empty() {
            return;
        }
        let Some(mut params) = self.imp().proc_params.borrow().clone() else { return };
        let stroke = Stroke { points, radius: self.brush_radius() };
        match self.imp().canvas_tool.get() {
            CanvasTool::Protect => params.dust.exclude.push(stroke),
            CanvasTool::Heal => params.dust.include.push(stroke),
            CanvasTool::Unmask => {
                erase_touching(&mut params.dust.exclude, &stroke);
                erase_touching(&mut params.dust.include, &stroke);
            }
            _ => return,
        }
        if params.dust.mode == nv_process::DustMode::Off {
            self.show_toast("Dust removal is off: turn it on in the Detail page to see the masks work");
        }
        self.commit_params(params);
        self.queue_overlay();
    }

    pub(super) fn clear_masks(&self) {
        let Some(mut params) = self.imp().proc_params.borrow().clone() else { return };
        if params.dust.exclude.is_empty() && params.dust.include.is_empty() {
            return;
        }
        params.dust.exclude.clear();
        params.dust.include.clear();
        self.commit_params(params);
        self.queue_overlay();
    }

    /// Draw the frame's strokes, and the one being painted, on the canvas.
    pub(super) fn draw_strokes(&self, cr: &gtk::cairo::Context, (cx, cy, cw, ch): (f64, f64, f64, f64), (iw, ih): (f64, f64)) {
        let o = self.orientation();
        let (exclude, include) = self.imp().proc_params.borrow().as_ref().map(|p| (p.dust.exclude.clone(), p.dust.include.clone())).unwrap_or_default();
        // The scan's long side in canvas pixels: the brush radius is a fraction of it.
        let long = iw.max(ih) * cw / iw;
        let to_canvas = |p: &[f32; 2]| -> (f64, f64) {
            let d = o.rect_from_source([p[0] as f64, p[1] as f64, p[0] as f64, p[1] as f64]);
            (cx + d[0] * cw, cy + d[1] * ch)
        };
        let paint = |strokes: &[Stroke], (r, g, b): (f64, f64, f64), alpha: f64| {
            for s in strokes {
                let pts: Vec<(f64, f64)> = s.points.iter().map(to_canvas).collect();
                let width = (s.radius as f64 * long * 2.0).max(2.0);
                cr.set_source_rgba(r, g, b, alpha);
                cr.set_line_width(width);
                cr.set_line_cap(gtk::cairo::LineCap::Round);
                cr.set_line_join(gtk::cairo::LineJoin::Round);
                if let Some(first) = pts.first() {
                    cr.move_to(first.0, first.1);
                    if pts.len() == 1 {
                        cr.line_to(first.0, first.1);
                    }
                    for p in &pts[1..] {
                        cr.line_to(p.0, p.1);
                    }
                    let _ = cr.stroke();
                }
            }
        };
        paint(&exclude, PROTECT, 0.40);
        paint(&include, HEAL, 0.40);
        let live = self.imp().stroke_points.borrow().clone();
        if !live.is_empty() {
            let colour = match self.imp().canvas_tool.get() {
                CanvasTool::Protect => PROTECT,
                CanvasTool::Heal => HEAL,
                _ => ERASE,
            };
            paint(&[Stroke { points: live, radius: self.brush_radius() }], colour, 0.55);
        }
    }

    /// A mouse position on the canvas as fractions of the picture, if it is on it.
    pub(super) fn canvas_fraction(&self, widget: &gtk::Widget, x: f64, y: f64) -> Option<(f64, f64)> {
        let (iw, ih) = self.canvas_image_size()?;
        let c = geom::content_rect(widget.width() as f64, widget.height() as f64, iw, ih);
        geom::point_to_frac(x, y, c)
    }
}
