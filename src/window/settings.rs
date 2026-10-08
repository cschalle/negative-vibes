//! Which settings a frame renders with, and where an edit goes: the roll's
//! defaults ("Whole Roll") or the frame's own overrides ("This Frame").
//! Also per-frame undo history and roll analysis.

use adw::prelude::*;
use adw::subclass::prelude::ObjectSubclassIsExt;
use nv_film::builtin_profiles;
use nv_process::{analyse_frame, analysis_fields, default_params_for, roll_analysis, FrameAnalysis, ProcessParams, Settings};
use nv_store::RawReader;

use super::history::{History, Scope};
use super::{imp, Window};
use crate::canvas_geom as geom;

/// The look every roll starts from: generic colour negative, all defaults.
pub(super) fn base_params() -> ProcessParams {
    default_params_for(&builtin_profiles()[0])
}

impl Window {
    /// Defaults, then the roll's settings, then the frame's own.
    pub(super) fn effective_params(&self, idx: u32) -> ProcessParams {
        let mut p = base_params();
        if let Some(roll) = self.imp().roll.borrow().as_ref() {
            roll.settings.process.apply_to(&mut p);
            if let Some(f) = roll.frame(idx) {
                f.overrides.apply_to(&mut p);
            }
        }
        p
    }

    /// The frame's rectangle on the image on screen, as fractions, when that
    /// image is a preview that holds it.
    fn frame_region(&self, idx: u32) -> Option<[f32; 4]> {
        if !self.imp().preview_ready.get() {
            return None;
        }
        let (w, h) = self.scan_image_size()?;
        let crop = self.imp().roll.borrow().as_ref()?.frame(idx)?.geometry.crop?;
        let r = geom::crop_to_rect(crop, w as u32, h as u32)?;
        Some([r[0] as f32, r[1] as f32, r[2] as f32, r[3] as f32])
    }

    /// Make the frame on screen render with its own settings: sets the
    /// parameters, the inspector, and the undo history that belongs to it,
    /// then renders. `None` is an image with no frame (a loose file).
    pub(super) fn load_params_for(&self, idx: Option<u32>) {
        let imp = self.imp();
        let known = idx.filter(|i| imp.roll.borrow().as_ref().is_some_and(|r| r.frame(*i).is_some()));
        let mut params = match known {
            Some(i) => self.effective_params(i),
            None => imp.proc_params.borrow().clone().unwrap_or_else(base_params),
        };
        params.conversion.region = known.and_then(|i| self.frame_region(i));

        // No film base from the roll or the frame: read it off the rebate.
        // (Not stored: it follows the picture until someone sets one.)
        if known.is_none() || params.conversion.film_base.is_none() {
            if let Some(proxy) = imp.proxy.borrow().as_ref() {
                params.conversion.film_base = Some(nv_process::estimate_film_base_proxy(proxy));
            }
        }

        // Undo history is per frame: put the old one away, take this one's.
        {
            let mut all = imp.histories.borrow_mut();
            let previous = std::mem::take(&mut *imp.history.borrow_mut());
            if !previous.is_empty() {
                all.insert(imp.history_key.get(), previous);
            }
            let key = known.unwrap_or(0);
            let mut h = all.remove(&key).unwrap_or_default();
            if h.is_empty() {
                h = History::new(params.clone(), imp.scope.get());
            } else {
                // The roll may have changed under this frame since it was last shown.
                h.ensure_current(&params, Scope::Frame);
            }
            *imp.history.borrow_mut() = h;
            imp.history_key.set(key);
        }

        self.sync_widgets(&params);
        self.update_dust_availability();
        *imp.proc_params.borrow_mut() = Some(params);
        imp.film_base_ready.set(true);
        self.refresh_canvas();
    }

    /// Dust removal needs an infrared plane: say so when the scan on the
    /// canvas has none, and do not offer controls that would do nothing.
    pub(super) fn update_dust_availability(&self) {
        let has_ir = self.imp().proxy.borrow().as_ref().is_some_and(|p| p.ir.is_some());
        for r in [self.imp().dust_mode_row.borrow().as_ref().map(|r| r.upcast_ref::<gtk::Widget>().clone()), self.imp().dust_detection_row.borrow().as_ref().map(|r| r.upcast_ref::<gtk::Widget>().clone())].into_iter().flatten() {
            r.set_sensitive(has_ir);
        }
        if let Some(n) = self.imp().dust_note_row.borrow().as_ref() {
            n.set_visible(!has_ir);
        }
    }

    /// The frame's rectangle moved: measurements follow it.
    pub(super) fn refresh_region(&self) {
        let Some(idx) = *self.imp().selected_frame.borrow() else { return };
        let region = self.frame_region(idx);
        let changed = match self.imp().proc_params.borrow_mut().as_mut() {
            Some(p) if p.conversion.region != region => {
                p.conversion.region = region;
                true
            }
            _ => false,
        };
        if changed {
            self.refresh_canvas();
        }
    }

    /// Record an edit from `old` to `new` where the scope says: the roll's
    /// defaults, or this frame's overrides.
    pub(super) fn record_edit(&self, old: &ProcessParams, new: &ProcessParams) {
        self.record_edit_in(old, new, self.imp().scope.get());
    }

    /// [`record_edit`](Self::record_edit) in a given scope: an undo goes back
    /// to where the edit it undoes was made.
    pub(super) fn record_edit_in(&self, old: &ProcessParams, new: &ProcessParams, scope: Scope) {
        let diff = Settings::diff(old, new);
        let Some(idx) = *self.imp().selected_frame.borrow() else { return };
        if diff.is_empty() {
            return;
        }
        if let Some(roll) = self.imp().roll.borrow_mut().as_mut() {
            match scope {
                Scope::Roll => {
                    roll.settings.process.merge(&diff);
                    // This frame follows the roll now, for what it just set.
                    if let Some(f) = roll.frame_mut(idx) {
                        f.overrides.clear(&diff);
                    }
                }
                Scope::Frame => {
                    if let Some(f) = roll.frame_mut(idx) {
                        f.overrides.merge(&diff);
                    }
                }
            }
        }
        self.schedule_autosave();
    }

    pub(super) fn set_scope(&self, scope: imp::Scope) {
        self.imp().scope.set(scope);
    }

    // ── Roll analysis ─────────────────────────────────────────────────────────

    /// Measure every frame and fix the median as the roll's conversion.
    pub(super) fn analyse_roll(&self) {
        let imp = self.imp();
        if imp.busy.get() || imp.analysing.get() {
            return;
        }
        let jobs: Vec<(u32, std::path::PathBuf, bool, Option<[u32; 4]>, ProcessParams)> = {
            let roll = imp.roll.borrow();
            let Some(roll) = roll.as_ref() else {
                self.show_toast("Open a roll first");
                return;
            };
            roll.frames
                .iter()
                .filter_map(|f| {
                    let src = self.frame_source(f.index)?;
                    Some((f.index, src.path, src.is_preview, src.crop, self.effective_params(f.index)))
                })
                .collect()
        };
        if jobs.is_empty() {
            self.show_toast("No previewed frames to analyse");
            return;
        }

        imp.analysing.set(true);
        self.show_toast(&match jobs.len() {
            1 | 2 => format!("Analysing {} frame… three or more give a steadier result", jobs.len()),
            n => format!("Analysing {n} frames…"),
        });
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let mut results: Vec<FrameAnalysis> = Vec::new();
            for (_, path, is_preview, crop, params) in jobs {
                let one = gio::spawn_blocking(move || {
                    let buf = RawReader::read(&path).ok()?;
                    let region = crop
                        .filter(|_| is_preview)
                        .and_then(|c| geom::crop_to_rect(c, buf.width, buf.effective_height()))
                        .map(|r| [r[0] as f32, r[1] as f32, r[2] as f32, r[3] as f32]);
                    let proxy = nv_process::Proxy::from_raw(&buf, nv_process::DEFAULT_PROXY_PIXELS);
                    analyse_frame(&proxy, region, &params)
                })
                .await;
                if let Ok(Some(a)) = one {
                    results.push(a);
                }
            }
            let Some(win) = weak.upgrade() else { return };
            win.imp().analysing.set(false);
            let Some(roll_wide) = roll_analysis(&results) else {
                win.show_toast("Could not analyse the roll");
                return;
            };
            win.set_roll_analysis(Some(roll_wide.settings()));
            win.show_toast(&format!("Roll analysed from {} frames", results.len()));
        });
    }

    /// Set (or, with `None`, clear) the roll-wide conversion that analysis fixes.
    pub(super) fn set_roll_analysis(&self, fixed: Option<Settings>) {
        if let Some(roll) = self.imp().roll.borrow_mut().as_mut() {
            match &fixed {
                Some(s) => roll.settings.process.merge(s),
                None => roll.settings.process.clear(&analysis_fields()),
            }
            // Frames follow the roll's conversion, not a stale one of their own.
            for f in &mut roll.frames {
                f.overrides.clear(&analysis_fields());
            }
        }
        let sel = *self.imp().selected_frame.borrow();
        if sel.is_some() && self.imp().proxy.borrow().is_some() {
            self.load_params_for(sel);
        }
        self.schedule_autosave();
    }
}
