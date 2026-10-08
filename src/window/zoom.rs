//! Canvas zoom and pan.
//!
//! "Fit" lets the picture letterbox itself inside the viewport. Any other
//! level sizes the overlay (picture plus frame overlay) to the image times
//! the zoom, so the scrolled window provides the panning and the frame
//! overlay's geometry keeps working: its widget is always exactly the image.

use std::time::Duration;

use adw::prelude::*;
use adw::subclass::prelude::ObjectSubclassIsExt;
use gtk::gdk;

use super::Window;
use crate::canvas_geom as geom;

const STEP: f64 = 1.25;

impl Window {
    /// Scale at which the whole image fits the viewport.
    fn fit_scale(&self) -> Option<f64> {
        let (iw, ih) = self.canvas_image_size()?;
        let scroll = self.imp().canvas_scroll.borrow();
        let scroll = scroll.as_ref()?;
        let (vw, vh) = (scroll.width() as f64, scroll.height() as f64);
        (vw > 0.0 && vh > 0.0).then(|| (vw / iw).min(vh / ih))
    }

    /// Lay the canvas out for the current zoom level and image size.
    pub(super) fn apply_zoom(&self) {
        let imp = self.imp();
        let (overlay, pic) = (imp.canvas_overlay.borrow(), imp.canvas_picture.borrow());
        let (Some(overlay), Some(pic)) = (overlay.as_ref(), pic.as_ref()) else { return };

        let label = match (imp.zoom.get(), self.canvas_image_size()) {
            (Some(z), Some((w, h))) => {
                overlay.set_size_request((w * z).round() as i32, (h * z).round() as i32);
                overlay.set_halign(gtk::Align::Center);
                overlay.set_valign(gtk::Align::Center);
                pic.set_content_fit(gtk::ContentFit::Fill);
                format!("{}%", (z * 100.0).round())
            }
            _ => {
                overlay.set_size_request(-1, -1);
                overlay.set_halign(gtk::Align::Fill);
                overlay.set_valign(gtk::Align::Fill);
                pic.set_content_fit(gtk::ContentFit::Contain);
                "Fit".to_owned()
            }
        };
        if let Some(l) = imp.zoom_label.borrow().as_ref() {
            l.set_label(&label);
        }
        self.schedule_hq();
        if let Some(a) = imp.navigator_area.borrow().as_ref() {
            a.queue_draw();
        }
    }

    /// Set the zoom (`None` = fit), keeping the middle of the view in place.
    pub(super) fn set_zoom(&self, zoom: Option<f64>) {
        let imp = self.imp();
        let scroll = imp.canvas_scroll.borrow().clone();
        let centre = scroll.as_ref().map(|s| {
            let (h, v) = (s.hadjustment(), s.vadjustment());
            (
                (h.value() + h.page_size() / 2.0) / h.upper().max(1.0),
                (v.value() + v.page_size() / 2.0) / v.upper().max(1.0),
            )
        });

        imp.zoom.set(zoom);
        self.apply_zoom();

        if let (Some(scroll), Some((cx, cy)), Some(_)) = (scroll, centre, zoom) {
            // The adjustments only learn their new range after the next layout.
            glib::timeout_add_local_once(Duration::from_millis(40), move || {
                let (h, v) = (scroll.hadjustment(), scroll.vadjustment());
                h.set_value(cx * h.upper() - h.page_size() / 2.0);
                v.set_value(cy * v.upper() - v.page_size() / 2.0);
            });
        }
    }

    fn zoom_by(&self, factor: f64) {
        let Some(fit) = self.fit_scale() else { return };
        self.set_zoom(geom::step_zoom(self.imp().zoom.get(), fit, factor));
    }

    /// Register `win.zoom-*` actions and the mouse gestures that go with them.
    pub(super) fn setup_zoom(&self, scroll: &gtk::ScrolledWindow) {
        // The full-quality view follows what is on screen.
        for adj in [scroll.hadjustment(), scroll.vadjustment()] {
            let weak = self.downgrade();
            adj.connect_value_changed(move |_| { if let Some(win) = weak.upgrade() { win.schedule_hq(); } });
        }
        let actions: [(&str, fn(&Window)); 4] = [
            ("zoom-in", |w| w.zoom_by(STEP)),
            ("zoom-out", |w| w.zoom_by(1.0 / STEP)),
            ("zoom-fit", |w| w.set_zoom(None)),
            ("zoom-100", |w| w.set_zoom(Some(1.0))),
        ];
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

        // Ctrl + wheel zooms; the plain wheel keeps scrolling.
        let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        wheel.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = self.downgrade();
        wheel.connect_scroll(move |ctrl, _, dy| {
            if !ctrl.current_event_state().contains(gdk::ModifierType::CONTROL_MASK) {
                return glib::Propagation::Proceed;
            }
            if let Some(win) = weak.upgrade() {
                win.zoom_by(if dy < 0.0 { STEP } else { 1.0 / STEP });
            }
            glib::Propagation::Stop
        });
        scroll.add_controller(wheel);

        // Middle-button drag pans (the primary button draws the frame).
        let pan = gtk::GestureDrag::builder().button(gdk::BUTTON_MIDDLE).build();
        let origin = std::rc::Rc::new(std::cell::Cell::new((0.0, 0.0)));
        let (o, s) = (origin.clone(), scroll.clone());
        pan.connect_drag_begin(move |_, _, _| o.set((s.hadjustment().value(), s.vadjustment().value())));
        let s = scroll.clone();
        pan.connect_drag_update(move |_, dx, dy| {
            s.hadjustment().set_value(origin.get().0 - dx);
            s.vadjustment().set_value(origin.get().1 - dy);
        });
        scroll.add_controller(pan);
    }
}
