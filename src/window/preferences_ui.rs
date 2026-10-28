//! The Preferences window (Ctrl+,, F§14): what the app does and what a new
//! roll starts with. Every control is saved as it is changed.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::ObjectSubclassIsExt;
use nv_export::{ColourSpace, ExportFormat};

use super::Window;
use super::scan_settings::dpi_label;
use crate::prefs::Prefs;

type Save = Rc<dyn Fn(&dyn Fn(&mut Prefs))>;

fn switch(title: &str, subtitle: &str, on: bool, save: &Save, set: fn(&mut Prefs, bool)) -> adw::SwitchRow {
    let row = adw::SwitchRow::builder().title(title).subtitle(subtitle).active(on).build();
    let save = save.clone();
    row.connect_active_notify(move |r| {
        let v = r.is_active();
        save(&|p| set(p, v));
    });
    row
}

fn spin(title: &str, subtitle: &str, value: f64, (lo, hi, step): (f64, f64, f64), save: &Save, set: fn(&mut Prefs, f64)) -> adw::SpinRow {
    let row = adw::SpinRow::builder().title(title).subtitle(subtitle).adjustment(&gtk::Adjustment::new(value, lo, hi, step, step * 5.0, 0.0)).build();
    let save = save.clone();
    row.connect_value_notify(move |r| {
        let v = r.value();
        save(&|p| set(p, v));
    });
    row
}

/// A combo over `labels`, selecting `at`, calling `set` with the chosen index.
fn combo(title: &str, subtitle: &str, labels: &[String], at: usize, save: &Save, set: impl Fn(&mut Prefs, usize) + Clone + 'static) -> adw::ComboRow {
    let row = adw::ComboRow::builder().title(title).subtitle(subtitle).model(&gtk::StringList::new(&labels.iter().map(String::as_str).collect::<Vec<_>>())).build();
    row.set_selected(at as u32);
    let save = save.clone();
    row.connect_selected_notify(move |r| {
        let i = r.selected() as usize;
        let set = set.clone();
        save(&move |p| set(p, i));
    });
    row
}

impl Window {
    pub(super) fn open_preferences(&self) {
        let prefs = self.imp().prefs.borrow().clone();
        let save: Save = {
            let weak = self.downgrade();
            Rc::new(move |f: &dyn Fn(&mut Prefs)| {
                if let Some(win) = weak.upgrade() {
                    let mut p = win.imp().prefs.borrow_mut();
                    f(&mut p);
                    *p = p.clone().normalised();
                    if let Err(e) = p.save() {
                        eprintln!("nv: could not save preferences: {e}");
                    }
                }
            })
        };
        let win = adw::PreferencesWindow::builder().transient_for(self).modal(true).title("Preferences").default_width(560).default_height(640).build();
        let group = |title: &str, description: &str| adw::PreferencesGroup::builder().title(title).description(description).build();

        // ── General ──────────────────────────────────────────────────────────
        let general = adw::PreferencesPage::builder().title("General").icon_name("preferences-system-symbolic").build();
        let start = group("Starting", "");
        start.add(&switch("Show the Scan a Roll Guide", "A step-by-step way to scan a roll; also in the main menu", prefs.guided_on_start, &save, |p, v| p.guided_on_start = v));
        start.add(&switch("Preview When the Scanner Is Ready", "Start a preview as soon as the scanner is found", prefs.auto_preview, &save, |p, v| p.auto_preview = v));
        let after = group("After Scanning", "");
        after.add(&switch("Export After Each Scan", "Make the picture as soon as the scan is saved", prefs.export.after_scan, &save, |p, v| p.export.after_scan = v));
        after.add(&switch("Open the Exported Picture", "In the default application", prefs.open_after_export, &save, |p, v| p.open_after_export = v));
        after.add(&switch("Play a Sound When a Scan Is Finished", "The system alert sound, once when a batch ends", prefs.completion_sound, &save, |p, v| p.completion_sound = v));
        let marks = group("Dust Marks", "The colour Mark mode paints over what it found");
        let mark_row = adw::ActionRow::builder().title("Mark Colour").build();
        let mark_btn = gtk::ColorDialogButton::builder().dialog(&gtk::ColorDialog::new()).valign(gtk::Align::Center).build();
        mark_btn.set_rgba(&gtk::gdk::RGBA::new(prefs.mark_colour[0], prefs.mark_colour[1], prefs.mark_colour[2], 1.0));
        mark_row.add_suffix(&mark_btn);
        {
            let (save, weak) = (save.clone(), self.downgrade());
            mark_btn.connect_rgba_notify(move |b| {
                let c = b.rgba();
                save(&|p| p.mark_colour = [c.red(), c.green(), c.blue()]);
                if let Some(win) = weak.upgrade() {
                    win.apply_mark_colour();
                }
            });
        }
        marks.add(&mark_row);
        let reset = group("", "");
        let reset_btn = gtk::Button::builder().label("Restore Default Preferences").halign(gtk::Align::Start).css_classes(["destructive-action"]).build();
        reset.add(&reset_btn);
        {
            let (weak, win) = (self.downgrade(), win.clone());
            reset_btn.connect_clicked(move |_| {
                let Some(app_win) = weak.upgrade() else { return };
                let dialog = adw::MessageDialog::builder()
                    .transient_for(&win)
                    .heading("Restore Default Preferences?")
                    .body("Every preference goes back to its original value. Rolls and their settings are not touched.")
                    .build();
                dialog.add_responses(&[("cancel", "Cancel"), ("reset", "Restore")]);
                dialog.set_response_appearance("reset", adw::ResponseAppearance::Destructive);
                let (weak, win) = (app_win.downgrade(), win.clone());
                dialog.connect_response(None, move |_, r| {
                    if r != "reset" {
                        return;
                    }
                    if let Some(app_win) = weak.upgrade() {
                        let fresh = Prefs::default();
                        if let Err(e) = fresh.save() {
                            eprintln!("nv: could not save preferences: {e}");
                        }
                        *app_win.imp().prefs.borrow_mut() = fresh;
                        app_win.apply_mark_colour();
                        app_win.show_toast("Preferences restored");
                    }
                    win.close();
                });
                dialog.present();
            });
        }
        for g in [&start, &after, &marks, &reset] {
            general.add(g);
        }

        // ── Scanning ─────────────────────────────────────────────────────────
        let scanning = adw::PreferencesPage::builder().title("Scanning").icon_name("scanner-symbolic").build();
        let steps = self.resolutions();
        let labels: Vec<String> = steps.iter().map(|d| dpi_label(*d)).collect();
        let res = group("Resolution", "What a new roll starts with");
        let at = |want: u32| steps.iter().rposition(|d| *d <= want).unwrap_or(0);
        let (s1, s2) = (steps.clone(), steps.clone());
        res.add(&combo("Preview Resolution", "A quick, small look at the whole film", &labels, at(prefs.preview_dpi), &save, move |p, i| {
            p.preview_dpi = s1.get(i).copied().unwrap_or(300)
        }));
        res.add(&combo("Scan Resolution", "For the final scan", &labels, at(prefs.scan_dpi), &save, move |p, i| p.scan_dpi = s2.get(i).copied().unwrap_or(1200)));
        let capture = group("Capture", "What a new roll starts with");
        capture.add(&switch("Capture Infrared", "For dust removal; silver black and white film blocks infrared", prefs.scan_ir, &save, |p, v| p.scan_ir = v));
        capture.add(&spin("Samples", "Passes averaged to lower the noise; each is a full scan", prefs.samples as f64, (1.0, 8.0, 1.0), &save, |p, v| p.samples = v as u8));
        let me = adw::SwitchRow::builder()
            .title("Multi-Exposure")
            .subtitle("Add a longer exposure for cleaner shadows, on scanners that can")
            .active(prefs.long_exposure.is_some())
            .build();
        let long = adw::SpinRow::builder()
            .title("Long Exposure")
            .subtitle("Times the normal exposure")
            .digits(1)
            .adjustment(&gtk::Adjustment::new(prefs.long_exposure.unwrap_or(2.5) as f64, 1.5, 4.0, 0.1, 0.5, 0.0))
            .sensitive(prefs.long_exposure.is_some())
            .build();
        {
            let (save1, long1) = (save.clone(), long.clone());
            me.connect_active_notify(move |r| {
                let on = r.is_active();
                long1.set_sensitive(on);
                let v = long1.value() as f32;
                save1(&|p| p.long_exposure = on.then_some(v));
            });
            let (save2, me2) = (save.clone(), me.clone());
            long.connect_value_notify(move |r| {
                let v = r.value() as f32;
                let on = me2.is_active();
                save2(&|p| p.long_exposure = on.then_some(v));
            });
        }
        capture.add(&me);
        capture.add(&long);
        let align = group("Alignment", "How far out of line may be put right, in pixels");
        align.add(&spin("Infrared", "Between the infrared and the colour of one scan", prefs.ir_align_limit as f64, (0.0, 64.0, 1.0), &save, |p, v| p.ir_align_limit = v as u32));
        align.add(&spin("Passes", "Between the passes of a multi-sample or multi-exposure scan", prefs.pass_align_limit as f64, (0.0, 64.0, 1.0), &save, |p, v| {
            p.pass_align_limit = v as u32
        }));
        for g in [&res, &capture, &align] {
            scanning.add(g);
        }

        // ── Output ───────────────────────────────────────────────────────────
        let output = adw::PreferencesPage::builder().title("Output").icon_name("document-save-symbolic").build();
        let files = group("Files", "What a new roll starts with");
        let formats = [ExportFormat::Tiff, ExportFormat::Jpeg, ExportFormat::Png, ExportFormat::Dng];
        let fi = formats.iter().position(|f| *f == prefs.export.format).unwrap_or(0);
        files.add(&combo("Format", "", &["TIFF", "JPEG", "PNG", "DNG (the raw scan)"].map(String::from), fi, &save, move |p, i| p.export.format = formats[i.min(3)]));
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
        files.add(&template);
        let folder_row = adw::ActionRow::builder()
            .title("Folder")
            .subtitle(prefs.export.folder.as_ref().map_or("~/Pictures/Scans/&lt;roll&gt; (default)".to_owned(), |f| glib::markup_escape_text(&f.display().to_string()).to_string()))
            .build();
        let choose = gtk::Button::builder().label("Choose…").valign(gtk::Align::Center).build();
        let default_btn = gtk::Button::builder().label("Default").valign(gtk::Align::Center).css_classes(["flat"]).build();
        folder_row.add_suffix(&default_btn);
        folder_row.add_suffix(&choose);
        {
            let (save_c, folder_c, win_c) = (save.clone(), folder_row.clone(), win.clone());
            choose.connect_clicked(move |_| {
                let (save, folder_row) = (save_c.clone(), folder_c.clone());
                gtk::FileDialog::builder().title("Output Folder").modal(true).build().select_folder(Some(&win_c), None::<&gio::Cancellable>, move |r| {
                    if let Some(path) = r.ok().and_then(|f| f.path()) {
                        folder_row.set_subtitle(&glib::markup_escape_text(&path.display().to_string()));
                        save(&|p| p.export.folder = Some(path.clone()));
                    }
                });
            });
            let (save_d, folder_d) = (save.clone(), folder_row.clone());
            default_btn.connect_clicked(move |_| {
                save_d(&|p| p.export.folder = None);
                folder_d.set_subtitle("~/Pictures/Scans/&lt;roll&gt; (default)");
            });
        }
        files.add(&folder_row);
        let colour = group("Colour", "");
        let spaces = [ColourSpace::Srgb, ColourSpace::AdobeRgb];
        let ci = spaces.iter().position(|c| *c == prefs.export.colour_space).unwrap_or(0);
        colour.add(&combo("Colour Space", "For JPEG, PNG and TIFF", &["sRGB", "Adobe RGB (1998)"].map(String::from), ci, &save, move |p, i| {
            p.export.colour_space = spaces[i.min(1)]
        }));
        colour.add(&switch("Embed the Colour Profile", "So other programs show the colours as intended", prefs.export.embed_profile, &save, |p, v| p.export.embed_profile = v));
        colour.add(&switch("Embed Metadata", "The details from Metadata (Ctrl+I)", prefs.export.embed_metadata, &save, |p, v| p.export.embed_metadata = v));
        output.add(&files);
        output.add(&colour);

        // ── Colour ───────────────────────────────────────────────────────────
        let cms = adw::PreferencesPage::builder().title("Colour").icon_name("applications-graphics-symbolic").build();
        let scanner = group("Scanner Profile", "From an IT8 target, applied to slides. New rolls start with the one chosen here.");
        let mut names = vec!["None".to_owned()];
        names.extend(nv_calibrate::list_profiles());
        let at = prefs.scanner_profile.as_ref().and_then(|n| names.iter().position(|x| x == n)).unwrap_or(0);
        let chosen = names.clone();
        scanner.add(&combo("Default Profile", "", &names, at, &save, move |p, i| p.scanner_profile = (i > 0).then(|| chosen[i].clone())));
        let cal = adw::ActionRow::builder().title("Calibrate with IT8 Target…").subtitle("Scan a target and build a profile for this scanner").activatable(true).build();
        {
            let (weak, win) = (self.downgrade(), win.clone());
            cal.connect_activated(move |_| {
                win.close();
                if let Some(w) = weak.upgrade() {
                    w.open_calibrate_dialog();
                }
            });
        }
        scanner.add(&cal);
        cms.add(&scanner);

        // ── Film profiles ────────────────────────────────────────────────────
        let film = adw::PreferencesPage::builder().title("Film Profiles").icon_name("camera-photo-symbolic").build();
        let list = group("Your Film Profiles", "The shipped profiles cannot be changed or removed");
        let rows: Rc<RefCell<Vec<adw::ActionRow>>> = Rc::new(RefCell::new(Vec::new()));
        let fill: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
        {
            let (list, rows, fill_slot, weak, win) = (list.clone(), rows.clone(), fill.clone(), self.downgrade(), win.clone());
            let refill: Rc<dyn Fn()> = Rc::new(move || {
                for r in rows.borrow_mut().drain(..) {
                    list.remove(&r);
                }
                let user = nv_film::library::user_profiles_in(&nv_film::library::user_dir());
                if user.is_empty() {
                    let empty = adw::ActionRow::builder().title("None Yet").subtitle("Create one from a scan of film base and a step wedge, or import a file").build();
                    list.add(&empty);
                    rows.borrow_mut().push(empty);
                }
                for p in user {
                    let row = adw::ActionRow::builder()
                        .title(format!("{} {}", p.vendor, p.name))
                        .subtitle(p.measured_on.as_ref().map_or("Imported".to_owned(), |m| format!("Measured on {}, {}", m.device, m.date)))
                        .build();
                    let export = gtk::Button::builder().icon_name("document-save-symbolic").tooltip_text("Export").valign(gtk::Align::Center).css_classes(["flat"]).build();
                    let delete = gtk::Button::builder().icon_name("user-trash-symbolic").tooltip_text("Delete").valign(gtk::Align::Center).css_classes(["flat"]).build();
                    row.add_suffix(&export);
                    row.add_suffix(&delete);
                    {
                        let (id, win) = (p.id.clone(), win.clone());
                        export.connect_clicked(move |_| {
                            let id = id.clone();
                            gtk::FileDialog::builder().title("Export Film Profile").initial_name(format!("{id}.json")).modal(true).build().save(
                                Some(&win),
                                None::<&gio::Cancellable>,
                                move |r| {
                                    if let Some(path) = r.ok().and_then(|f| f.path())
                                        && let Err(e) = nv_film::library::export(&id, &path)
                                    {
                                        eprintln!("nv: could not export the profile: {e}");
                                    }
                                },
                            );
                        });
                    }
                    {
                        let (id, weak, slot) = (p.id.clone(), weak.clone(), fill_slot.clone());
                        delete.connect_clicked(move |_| {
                            if nv_film::library::delete(&id).is_ok() {
                                if let Some(w) = weak.upgrade() {
                                    w.refresh_film_profiles();
                                }
                                let again = slot.borrow().clone();
                                if let Some(f) = again {
                                    f();
                                }
                            }
                        });
                    }
                    list.add(&row);
                    rows.borrow_mut().push(row);
                }
            });
            *fill.borrow_mut() = Some(refill.clone());
            refill();
        }
        let actions = group("", "");
        let import = adw::ActionRow::builder().title("Import a Film Profile…").activatable(true).build();
        let create = adw::ActionRow::builder().title("Create a Film Profile…").subtitle("From the open scan of film base and a step wedge").activatable(true).build();
        {
            let (weak_i, win_i, slot_i) = (self.downgrade(), win.clone(), fill.clone());
            import.connect_activated(move |_| {
                let (weak, slot) = (weak_i.clone(), slot_i.clone());
                gtk::FileDialog::builder().title("Import Film Profile").modal(true).build().open(Some(&win_i), None::<&gio::Cancellable>, move |r| {
                    let Some(path) = r.ok().and_then(|f| f.path()) else { return };
                    match nv_film::library::import(&path) {
                        Ok(_) => {
                            if let Some(w) = weak.upgrade() {
                                w.refresh_film_profiles();
                            }
                            let again = slot.borrow().clone();
                            if let Some(f) = again {
                                f();
                            }
                        }
                        Err(e) => eprintln!("nv: could not import the profile: {e}"),
                    }
                });
            });
            let (weak_c, win_c) = (self.downgrade(), win.clone());
            create.connect_activated(move |_| {
                win_c.close();
                if let Some(w) = weak_c.upgrade() {
                    w.open_film_profile_dialog();
                }
            });
        }
        actions.add(&import);
        actions.add(&create);
        film.add(&list);
        film.add(&actions);

        // ── Google Photos ────────────────────────────────────────────────────
        let google = adw::PreferencesPage::builder().title("Google Photos").icon_name("web-browser-symbolic").build();
        let account = group(
            "Account",
            "Pictures can be sent to your Google Photos library as they are exported. Negative Vibes can only add pictures and make albums for them: it cannot see or change anything else in your library.",
        );
        let status = adw::ActionRow::builder().title("Google Account").subtitle("Checking…").build();
        let button = gtk::Button::builder().label("Sign In…").valign(gtk::Align::Center).sensitive(false).build();
        status.add_suffix(&button);
        account.add(&status);
        // The row says what is true now; the button does what fits.
        let refresh_account: Rc<dyn Fn()> = {
            let (weak, status, button) = (self.downgrade(), status.clone(), button.clone());
            Rc::new(move || {
                let Some(app) = weak.upgrade() else { return };
                let has_client = crate::google::oauth_config(&app.imp().prefs.borrow()).is_set();
                button.set_sensitive(true);
                if app.google_signing_in() {
                    status.set_subtitle("Waiting for you to finish in the browser…");
                    button.set_label("Cancel");
                } else if app.google_signed_in() {
                    status.set_subtitle("Signed in");
                    button.set_label("Sign Out");
                } else if !has_client {
                    status.set_subtitle("Not available in this copy. Exports are still saved to your folder");
                    button.set_label("Sign In…");
                    button.set_sensitive(false);
                } else {
                    status.set_subtitle("Not signed in");
                    button.set_label("Sign In…");
                }
            })
        };
        {
            let refresh = refresh_account.clone();
            self.with_google_token(move |_| refresh());
        }
        {
            let (weak, refresh) = (self.downgrade(), refresh_account.clone());
            button.connect_clicked(move |_| {
                let Some(app) = weak.upgrade() else { return };
                if app.google_signing_in() {
                    app.google_cancel_sign_in();
                } else if app.google_signed_in() {
                    app.google_sign_out();
                    refresh();
                } else {
                    let (weak, refresh2) = (app.downgrade(), refresh.clone());
                    app.google_sign_in(Rc::new(move |r| {
                        refresh2();
                        if let (Err(e), Some(app)) = (r, weak.upgrade())
                            && !e.is_empty()
                        {
                            app.show_toast(&format!("Could not sign in: {e}"));
                        }
                    }));
                    refresh();
                }
            });
        }
        let own = adw::ExpanderRow::builder()
            .title("Use Your Own Google Client")
            .subtitle("Leave empty to use the one built into this copy. For forks, or your own Google Cloud project: an OAuth client of type Desktop app with the Photos Library API enabled")
            .show_enable_switch(false)
            .build();
        own.set_expanded(!prefs.google_client_id.is_empty());
        let client_id = adw::EntryRow::builder().title("Client ID").text(&prefs.google_client_id).build();
        let client_secret = adw::PasswordEntryRow::builder().title("Client Secret").text(&prefs.google_client_secret).build();
        for (row, set) in [
            (client_id.upcast_ref::<adw::EntryRow>(), (|p: &mut Prefs, v: String| p.google_client_id = v) as fn(&mut Prefs, String)),
            (client_secret.upcast_ref(), |p, v| p.google_client_secret = v),
        ] {
            let (save, refresh) = (save.clone(), refresh_account.clone());
            row.connect_changed(move |e| {
                let v = e.text().to_string();
                save(&|p| set(p, v.clone()));
                refresh();
            });
        }
        own.add_row(&client_id);
        own.add_row(&client_secret);
        let advanced = group("Advanced", "");
        advanced.add(&own);
        google.add(&account);
        google.add(&advanced);

        for p in [&general, &scanning, &output, &cms, &film, &google] {
            win.add(p);
        }
        super::chrome::present_labelled(&win);
    }
}
