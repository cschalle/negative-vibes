//! Scan resolution and its live file size (F§9), and the Preferences window
//! that sets what a new roll starts with.

use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::ObjectSubclassIsExt;
use nv_export::{ColourSpace, ExportFormat};

use super::Window;
use crate::prefs::Prefs;

/// Resolutions offered when the scanner has not said what it can do.
/// Above this, the memory several passes need is more than a typical machine has.
const MAX_PASSES_DPI: u32 = 3600;

const FALLBACK_DPI: [u32; 6] = [300, 600, 1200, 2400, 3600, 7200];

fn dpi_label(dpi: u32) -> String {
    format!("{dpi} ppi")
}

/// "11.8 MB" or "482 MB": a size a person can read.
pub fn human_bytes(b: u64) -> String {
    let mb = b as f64 / (1024.0 * 1024.0);
    if mb >= 100.0 { format!("{mb:.0} MB") } else if mb >= 1.0 { format!("{mb:.1} MB") } else { format!("{:.0} kB", b as f64 / 1024.0) }
}

/// Pixels and bytes of a scan of `window_mm` (or the part `area` of it) at `dpi`,
/// 16-bit RGB: what the live size shows (F§9 "2400ppi 24 MB").
pub fn scan_size(window_mm: [f64; 2], area: Option<[f64; 4]>, dpi: u32) -> (u32, u32, u64) {
    let (fw, fh) = match area {
        Some(a) => ((a[2] - a[0]).clamp(0.0, 1.0), (a[3] - a[1]).clamp(0.0, 1.0)),
        None => (1.0, 1.0),
    };
    let px = |mm: f64, frac: f64| ((mm / 25.4 * dpi as f64 * frac).round() as u32).max(1);
    let (w, h) = (px(window_mm[0], fw), px(window_mm[1], fh));
    (w, h, w as u64 * h as u64 * 3 * 2)
}

impl Window {
    /// The resolution for the next final scan: the roll's, else the session's.
    pub fn scan_dpi(&self) -> u32 {
        self.imp().roll.borrow().as_ref().and_then(|r| r.settings.resolution_dpi).unwrap_or_else(|| self.imp().scan_dpi.get())
    }

    /// Whether the next final scan also captures infrared: the scanner can,
    /// and the roll (else the session) wants it.
    pub fn scan_with_ir(&self) -> bool {
        self.imp().device_ir.get() && self.imp().roll.borrow().as_ref().and_then(|r| r.settings.with_ir).unwrap_or_else(|| self.imp().with_ir.get())
    }

    /// How the next final scan is made up of passes: the roll's choice, else the
    /// session's. Passes need memory for the sums, so very high resolutions scan once.
    pub fn scan_passes(&self) -> nv_acquire::passes::PassPlan {
        let imp = self.imp();
        let (samples, long) = {
            let roll = imp.roll.borrow();
            let s = roll.as_ref().and_then(|r| r.settings.samples).unwrap_or_else(|| imp.samples.get());
            let l = match roll.as_ref() {
                Some(r) => r.settings.long_exposure,
                None => imp.long_exposure.get(),
            };
            (s, l)
        };
        // Headless hooks for the hardware checks, like NV_AUTOSCAN.
        let samples = std::env::var("NV_SAMPLES").ok().and_then(|v| v.parse().ok()).unwrap_or(samples);
        let long = std::env::var("NV_LONG_EXPOSURE").ok().and_then(|v| v.parse().ok()).or(long);
        let plan = nv_acquire::passes::PassPlan { samples, long_exposure: long.filter(|_| imp.device_exposure.get()), max_offset: 20 }.normalised();
        if self.scan_dpi() > MAX_PASSES_DPI {
            return nv_acquire::passes::PassPlan::SINGLE;
        }
        plan
    }

    /// The Samples and Multi-Exposure rows follow the roll and the scanner.
    pub(super) fn update_passes_rows(&self) {
        let imp = self.imp();
        let Some((samples, me, long)) = imp.passes_rows.borrow().clone() else { return };
        let plan = self.scan_passes();
        imp.updating_ui.set(true);
        let roll = imp.roll.borrow();
        samples.set_value(roll.as_ref().and_then(|r| r.settings.samples).unwrap_or_else(|| imp.samples.get()) as f64);
        let l = match roll.as_ref() {
            Some(r) => r.settings.long_exposure,
            None => imp.long_exposure.get(),
        };
        me.set_active(l.is_some());
        if let Some(l) = l {
            long.set_value(l as f64);
        }
        drop(roll);
        imp.updating_ui.set(false);
        me.set_visible(imp.device_exposure.get());
        long.set_visible(imp.device_exposure.get() && me.is_active());
        let too_big = self.scan_dpi() > MAX_PASSES_DPI;
        samples.set_sensitive(!too_big);
        me.set_sensitive(!too_big);
        samples.set_subtitle(&if too_big {
            format!("Several passes are limited to {MAX_PASSES_DPI} dpi and below; this resolution scans once")
        } else if plan.total_scans() > 1 {
            format!("{} scans of the frame in all, lined up and merged", plan.total_scans())
        } else {
            "Scans averaged to lower the noise (4 samples halve it); each is a full scan".to_owned()
        });
    }

    pub(super) fn resolutions(&self) -> Vec<u32> {
        let r = self.imp().device_resolutions.borrow().clone();
        if r.is_empty() { FALLBACK_DPI.to_vec() } else { r }
    }

    /// The inspector page: resolution, and what it costs in pixels and bytes.
    pub(super) fn build_scan_page(&self) -> gtk::ScrolledWindow {
        let page = adw::PreferencesPage::new();
        let group = adw::PreferencesGroup::builder().title("Resolution").description("Used for the final scan. The preview is always quick and small.").build();
        let row = adw::ComboRow::builder().title("Scan Resolution").build();
        let size = adw::ActionRow::builder().title("File Size").subtitle("").build();
        size.add_css_class("property");
        group.add(&row);
        let ir = adw::SwitchRow::builder()
            .title("Capture Infrared")
            .subtitle("For dust and scratch removal. Adds a second pass; silver black and white film blocks infrared.")
            .active(true)
            .visible(false)
            .build();
        group.add(&ir);
        let samples = adw::SpinRow::builder().title("Samples").adjustment(&gtk::Adjustment::new(1.0, 1.0, nv_acquire::passes::MAX_SAMPLES as f64, 1.0, 1.0, 0.0)).build();
        let me = adw::SwitchRow::builder().title("Multi-Exposure").subtitle("Adds a longer exposure for cleaner shadows in dense film").visible(false).build();
        let long = adw::SpinRow::builder().title("Long Exposure").subtitle("Times the normal exposure").digits(1).visible(false).adjustment(&gtk::Adjustment::new(2.5, 1.5, 4.0, 0.1, 0.5, 0.0)).build();
        group.add(&samples);
        group.add(&me);
        group.add(&long);
        *self.imp().passes_rows.borrow_mut() = Some((samples.clone(), me.clone(), long.clone()));
        {
            let weak = self.downgrade();
            let apply = move || {
                let Some(win) = weak.upgrade() else { return };
                let imp = win.imp();
                if imp.updating_ui.get() { return; }
                let Some((samples, me, long)) = imp.passes_rows.borrow().clone() else { return };
                let (s, l) = (samples.value() as u8, me.is_active().then(|| long.value() as f32));
                if let Some(roll) = imp.roll.borrow_mut().as_mut() {
                    roll.settings.samples = Some(s);
                    roll.settings.long_exposure = l;
                }
                imp.samples.set(s);
                imp.long_exposure.set(l);
                win.schedule_autosave();
                win.update_passes_rows();
                win.update_scan_size();
            };
            let a = apply.clone();
            samples.connect_value_notify(move |_| a());
            let a = apply.clone();
            me.connect_active_notify(move |_| a());
            long.connect_value_notify(move |_| apply());
        }
        group.add(&size);
        page.add(&group);
        *self.imp().ir_row.borrow_mut() = Some(ir.clone());
        let weak = self.downgrade();
        ir.connect_active_notify(move |r| {
            let Some(win) = weak.upgrade() else { return };
            if win.imp().updating_ui.get() {
                return;
            }
            let on = r.is_active();
            if let Some(roll) = win.imp().roll.borrow_mut().as_mut() {
                roll.settings.with_ir = Some(on);
            }
            win.imp().with_ir.set(on);
            win.schedule_autosave();
            win.update_scan_size();
        });
        *self.imp().scan_res_row.borrow_mut() = Some(row.clone());
        *self.imp().scan_size_row.borrow_mut() = Some(size);

        let weak = self.downgrade();
        row.connect_selected_notify(move |row| {
            let Some(win) = weak.upgrade() else { return };
            if win.imp().updating_ui.get() {
                return;
            }
            if let Some(dpi) = win.resolutions().get(row.selected() as usize).copied() {
                win.set_scan_dpi(dpi);
            }
        });
        self.fill_resolutions();

        gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&page).build()
    }

    /// Put the device's resolutions in the combo, keeping the chosen one selected.
    pub(super) fn fill_resolutions(&self) {
        let Some(row) = self.imp().scan_res_row.borrow().clone() else { return };
        let list = self.resolutions();
        let want = self.scan_dpi();
        self.imp().updating_ui.set(true);
        let labels: Vec<String> = list.iter().map(|d| dpi_label(*d)).collect();
        row.set_model(Some(&gtk::StringList::new(&labels.iter().map(String::as_str).collect::<Vec<_>>())));
        // The nearest offered step at or below the wish, else the first.
        let at = list.iter().rposition(|d| *d <= want).unwrap_or(0);
        row.set_selected(at as u32);
        self.imp().updating_ui.set(false);
        self.update_scan_size();
    }

    /// The roll (and the session) scans at `dpi` from now on.
    pub(super) fn set_scan_dpi(&self, dpi: u32) {
        self.imp().scan_dpi.set(dpi);
        if let Some(roll) = self.imp().roll.borrow_mut().as_mut() {
            roll.settings.resolution_dpi = Some(dpi);
        }
        self.schedule_autosave();
        self.update_scan_size();
    }

    /// "1714 × 1150 px, 11.3 MB at 16 bit" for the rectangle on the preview.
    pub(super) fn update_scan_size(&self) {
        self.update_passes_rows();
        if let Some(r) = self.imp().ir_row.borrow().as_ref() {
            self.imp().updating_ui.set(true);
            r.set_active(self.imp().roll.borrow().as_ref().and_then(|x| x.settings.with_ir).unwrap_or_else(|| self.imp().with_ir.get()));
            self.imp().updating_ui.set(false);
        }
        let Some(row) = self.imp().scan_size_row.borrow().clone() else { return };
        let Some(window) = self.imp().film_window_mm.get() else {
            row.set_subtitle("Connect a scanner to see the size");
            return;
        };
        let (w, h, bytes) = scan_size(window, self.frame_rect(), self.scan_dpi());
        if self.scan_with_ir() {
            // The infrared plane is one more sample per pixel.
            row.set_subtitle(&format!("{w} × {h} px, {} as 16-bit colour and infrared", human_bytes(bytes / 3 * 4)));
        } else {
            row.set_subtitle(&format!("{w} × {h} px, {} as 16-bit colour", human_bytes(bytes)));
        }
    }

    // ── Preferences ───────────────────────────────────────────────────────────

    pub(super) fn setup_preferences(&self) {
        *self.imp().prefs.borrow_mut() = Prefs::load();
        self.imp().scan_dpi.set(self.imp().prefs.borrow().scan_dpi);
        self.imp().with_ir.set(true);

        let action = gio::SimpleAction::new("preferences", None);
        let weak = self.downgrade();
        action.connect_activate(move |_, _| {
            if let Some(win) = weak.upgrade() {
                win.open_preferences();
            }
        });
        self.add_action(&action);
    }

    /// Put the preferred mark colour on the frame on screen and redraw it.
    fn apply_mark_colour(&self) {
        let m = self.imp().prefs.borrow().mark_colour;
        let linear = [0, 1, 2].map(|c| nv_export::ColourSpace::Srgb.decode(m[c]));
        if let Some(p) = self.imp().proc_params.borrow_mut().as_mut() {
            p.dust.mark = linear;
        }
        self.refresh_canvas();
    }

    fn open_preferences(&self) {
        let prefs = self.imp().prefs.borrow().clone();
        let save = {
            let weak = self.downgrade();
            Rc::new(move |f: &dyn Fn(&mut Prefs)| {
                if let Some(win) = weak.upgrade() {
                    let mut p = win.imp().prefs.borrow_mut();
                    f(&mut p);
                    if let Err(e) = p.save() {
                        eprintln!("nv: could not save preferences: {e}");
                    }
                }
            })
        };

        let win = adw::PreferencesWindow::builder().transient_for(self).modal(true).title("Preferences").build();
        let page = adw::PreferencesPage::builder().title("General").icon_name("preferences-system-symbolic").build();

        let scanning = adw::PreferencesGroup::builder().title("Scanning").description("What a new roll starts with").build();
        let guide = adw::SwitchRow::builder().title("Show the Scan a Roll Guide at Start").subtitle("A step-by-step way to scan a roll; also in the main menu").active(prefs.guided_on_start).build();
        {
            let save = save.clone();
            guide.connect_active_notify(move |r| {
                let on = r.is_active();
                save(&|p| p.guided_on_start = on);
            });
        }
        scanning.add(&guide);
        let list = self.resolutions();
        let labels: Vec<String> = list.iter().map(|d| dpi_label(*d)).collect();
        let res = adw::ComboRow::builder().title("Scan Resolution").model(&gtk::StringList::new(&labels.iter().map(String::as_str).collect::<Vec<_>>())).build();
        res.set_selected(list.iter().rposition(|d| *d <= prefs.scan_dpi).unwrap_or(0) as u32);
        {
            let (save, list) = (save.clone(), list.clone());
            res.connect_selected_notify(move |r| {
                if let Some(dpi) = list.get(r.selected() as usize).copied() {
                    save(&|p| p.scan_dpi = dpi);
                }
            });
        }
        scanning.add(&res);

        let output = adw::PreferencesGroup::builder().title("Output").description("What a new roll starts with").build();
        let formats = [ExportFormat::Tiff, ExportFormat::Jpeg, ExportFormat::Png, ExportFormat::Dng];
        let format = adw::ComboRow::builder().title("Format").model(&gtk::StringList::new(&["TIFF", "JPEG", "PNG", "DNG (the raw scan)"])).build();
        format.set_selected(formats.iter().position(|f| *f == prefs.export.format).unwrap_or(0) as u32);
        {
            let save = save.clone();
            format.connect_selected_notify(move |r| {
                let f = formats[(r.selected() as usize).min(3)];
                save(&|p| p.export.format = f);
            });
        }
        let spaces = [ColourSpace::Srgb, ColourSpace::AdobeRgb];
        let space = adw::ComboRow::builder().title("Colour Space").model(&gtk::StringList::new(&["sRGB", "Adobe RGB (1998)"])).build();
        space.set_selected(spaces.iter().position(|c| *c == prefs.export.colour_space).unwrap_or(0) as u32);
        {
            let save = save.clone();
            space.connect_selected_notify(move |r| {
                let c = spaces[(r.selected() as usize).min(1)];
                save(&|p| p.export.colour_space = c);
            });
        }
        let template = adw::EntryRow::builder().title("File Name").text(&prefs.export.template).build();
        {
            let save = save.clone();
            template.connect_changed(move |e| {
                let t = e.text().to_string();
                if nv_export::render_template(&t, &nv_export::NameContext { roll: "Roll".into(), frame: 1, date: "2026-01-01".into(), ..Default::default() }).is_ok() {
                    e.remove_css_class("error");
                    save(&|p| p.export.template = t.clone());
                } else {
                    e.add_css_class("error"); // not saved until it is a valid template
                }
            });
        }
        let folder_row = adw::ActionRow::builder().title("Folder").subtitle(prefs.export.folder.as_ref().map_or("~/Pictures/Scans/<roll> (default)".to_owned(), |f| f.display().to_string())).build();
        let choose = gtk::Button::builder().label("Choose…").valign(gtk::Align::Center).build();
        let default_btn = gtk::Button::builder().label("Default").valign(gtk::Align::Center).css_classes(["flat"]).build();
        folder_row.add_suffix(&default_btn);
        folder_row.add_suffix(&choose);
        {
            let (save, folder_row, win) = (save.clone(), folder_row.clone(), win.clone());
            choose.connect_clicked(move |_| {
                let (save, folder_row) = (save.clone(), folder_row.clone());
                gtk::FileDialog::builder().title("Output Folder").modal(true).build().select_folder(Some(&win), None::<&gio::Cancellable>, move |r| {
                    if let Some(path) = r.ok().and_then(|f| f.path()) {
                        folder_row.set_subtitle(&path.display().to_string());
                        save(&|p| p.export.folder = Some(path.clone()));
                    }
                });
            });
        }
        {
            let (save, folder_row) = (save.clone(), folder_row.clone());
            default_btn.connect_clicked(move |_| {
                folder_row.set_subtitle("~/Pictures/Scans/<roll> (default)");
                save(&|p| p.export.folder = None);
            });
        }
        let after = adw::SwitchRow::builder().title("Export After Each Scan").active(prefs.export.after_scan).build();
        {
            let save = save.clone();
            after.connect_active_notify(move |s| {
                let on = s.is_active();
                save(&|p| p.export.after_scan = on);
            });
        }
        let marks = adw::PreferencesGroup::builder().title("Dust Marks").description("The colour Mark mode paints over what it found").build();
        let mark_row = adw::ActionRow::builder().title("Mark Colour").build();
        let mark_btn = gtk::ColorDialogButton::builder().dialog(&gtk::ColorDialog::new()).valign(gtk::Align::Center).build();
        mark_btn.set_rgba(&gtk::gdk::RGBA::new(prefs.mark_colour[0], prefs.mark_colour[1], prefs.mark_colour[2], 1.0));
        mark_row.add_suffix(&mark_btn);
        {
            let (save, weak) = (save.clone(), self.downgrade());
            mark_btn.connect_rgba_notify(move |b| {
                let c = b.rgba();
                save(&|p| p.mark_colour = [c.red(), c.green(), c.blue()]);
                // The frame on screen repaints in the new colour.
                if let Some(win) = weak.upgrade() {
                    win.apply_mark_colour();
                }
            });
        }
        marks.add(&mark_row);

        output.add(&format);
        output.add(&space);
        output.add(&folder_row);
        output.add(&template);
        output.add(&after);

        page.add(&scanning);
        page.add(&output);
        page.add(&marks);
        win.add(&page);
        win.present();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_size_follows_the_window_the_rectangle_and_the_resolution() {
        // The 8200i's window at 1200 ppi: 36.5 mm = 1724 px, 27.2 mm = 1285 px.
        let (w, h, bytes) = scan_size([36.5, 27.2], None, 1200);
        assert_eq!((w, h), (1724, 1285));
        assert_eq!(bytes, 1724 * 1285 * 6);
        // Half the width and height of the window: a quarter of the pixels.
        let (hw, hh, hb) = scan_size([36.5, 27.2], Some([0.0, 0.0, 0.5, 0.5]), 1200);
        assert_eq!((hw, hh), (862, 643));
        assert!((hb as f64 / bytes as f64 - 0.25).abs() < 0.01);
        // 7200 ppi is the 482 MB of DESIGN.md §1.
        let (_, _, big) = scan_size([36.5, 27.2], None, 7200);
        assert!((human_bytes(big)).starts_with("4"), "{}", human_bytes(big));
    }

    #[test]
    fn sizes_read_like_sizes() {
        assert_eq!(human_bytes(512 * 1024 - 1), "512 kB");
        assert_eq!(human_bytes(11_800_000), "11.3 MB");
        assert_eq!(human_bytes(482 * 1024 * 1024), "482 MB");
        assert_eq!(human_bytes(2048), "2 kB");
    }
}
