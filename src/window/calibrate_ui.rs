//! Calibrate with IT8 Target (F§11): a scan of the target and its measured
//! values become an input profile for the scanner, built by ArgyllCMS. Slides
//! then use it for their colour.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::ObjectSubclassIsExt;

use super::Window;

fn file_row(title: &str, subtitle: &str) -> (adw::ActionRow, gtk::Button) {
    let row = adw::ActionRow::builder().title(title).subtitle(subtitle).build();
    let b = gtk::Button::builder().label("Choose…").valign(gtk::Align::Center).build();
    row.add_suffix(&b);
    (row, b)
}

/// The maker's ZIP of IT8 data, when it is already in the Downloads folder: the newest `.zip` with "it8" in its name.
fn downloaded_data() -> Option<PathBuf> {
    let dir = glib::user_special_dir(glib::UserDirectory::Downloads)?;
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_ascii_lowercase();
            n.contains("it8") && n.ends_with(".zip")
        })
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .max()
        .map(|(_, p)| p)
}

impl Window {
    pub(super) fn open_calibrate_dialog(&self) {
        self.open_calibrate_dialog_with(None);
    }

    /// The dialog, with `scan` already chosen (a scan of the target just made), else the open frame's.
    pub(super) fn open_calibrate_dialog_with(&self, scan: Option<PathBuf>) {
        if !nv_calibrate::available() {
            self.show_toast("Calibration needs ArgyllCMS (scanin, colprof); install it first");
            return;
        }
        let just_scanned = scan.is_some();
        let scan: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(scan.or_else(|| self.imp().displayed_file.borrow().clone())));
        let reference: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(None));
        let chart: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(nv_calibrate::default_chart()));

        let scan_subtitle = match scan.borrow().as_ref() {
            Some(p) if just_scanned => format!("Scanned just now: {}", glib::markup_escape_text(&p.display().to_string())),
            Some(p) => format!("The open frame: {}", glib::markup_escape_text(&p.display().to_string())),
            None => "Put the target slide in the scanner and press Scan Now, or choose a scan you made earlier".to_owned(),
        };
        let (scan_row, scan_btn) = file_row("Scan of the Target", &scan_subtitle);
        scan_btn.set_label("Choose File…");
        let scan_now = gtk::Button::builder().label("Scan Now").valign(gtk::Align::Center).css_classes(["suggested-action"]).build();
        scan_row.add_suffix(&scan_now);
        scan_now.set_sensitive(self.device_ready() && !self.imp().busy.get());
        if !scan_now.is_sensitive() {
            scan_now.set_tooltip_text(Some("No scanner is ready"));
        }
        let (ref_row, ref_btn) = file_row(
            "Target Data",
            "Not part of this application: the maker of the target publishes it. For a GWB Colour Card slide, press Get Data and download their ZIP of IT8 files, then choose it here (or the single .txt file for your batch).",
        );
        ref_btn.set_label("Choose…");
        let get_data = gtk::Button::builder().label("Get Data…").valign(gtk::Align::Center).tooltip_text("Opens the maker's page in your browser").build();
        ref_row.add_suffix(&get_data);
        let batch_row = adw::EntryRow::builder().title("Batch Code Printed on the Slide (like T100926)").visible(false).build();
        let suggest = gtk::Button::builder().label("Most Likely").valign(gtk::Align::Center).sensitive(false).tooltip_text("Works out which batch the scan matches best").build();
        batch_row.add_suffix(&suggest);
        let suggestion: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        let batches: Rc<RefCell<Vec<nv_calibrate::batch::Candidate>>> = Rc::new(RefCell::new(Vec::new()));
        // The scan, shown so that the batch printed on the target can be read off it.
        let preview_pic = gtk::Picture::builder().content_fit(gtk::ContentFit::Contain).height_request(300).can_shrink(true).build();
        let preview_buf: Rc<RefCell<Option<(u32, u32, Vec<u8>)>>> = Rc::new(RefCell::new(None));
        let rotate = gtk::Button::builder().icon_name("object-rotate-right-symbolic").tooltip_text("Turn the picture").valign(gtk::Align::Center).build();
        rotate.add_css_class("flat");
        let preview_group = adw::PreferencesGroup::builder()
            .title("The Scan")
            .description("The batch code is printed on the slide, small. Turn the picture until you can read it.")
            .visible(false)
            .build();
        preview_group.set_header_suffix(Some(&rotate));
        preview_group.add(&preview_pic);
        let show_preview: Rc<dyn Fn()> = {
            let (pic, buf) = (preview_pic.clone(), preview_buf.clone());
            Rc::new(move || {
                if let Some((w, h, px)) = buf.borrow().as_ref() {
                    let tex = gtk::gdk::MemoryTexture::new(*w as i32, *h as i32, gtk::gdk::MemoryFormat::R8g8b8, &glib::Bytes::from(px.as_slice()), *w as usize * 3);
                    pic.set_paintable(Some(&tex));
                }
            })
        };
        let load_preview: Rc<dyn Fn(&std::path::Path)> = {
            let (buf, show, group) = (preview_buf.clone(), show_preview.clone(), preview_group.clone());
            Rc::new(move |p: &std::path::Path| {
                group.set_visible(true);
                let (p, buf, show) = (p.to_path_buf(), buf.clone(), show.clone());
                glib::MainContext::default().spawn_local(async move {
                    if let Ok(Ok(img)) = gio::spawn_blocking(move || nv_calibrate::preview_rgb8(&p, 1600)).await {
                        *buf.borrow_mut() = Some(img);
                        show();
                    }
                });
            })
        };
        {
            let (buf, show) = (preview_buf.clone(), show_preview.clone());
            rotate.connect_clicked(move |_| {
                let turned = buf.borrow().as_ref().map(|(w, h, px)| nv_calibrate::rotate_quarter(*w, *h, px));
                if turned.is_some() {
                    *buf.borrow_mut() = turned;
                    show();
                }
            });
        }
        let (chart_row, chart_btn) =
            file_row("Chart Layout", &chart.borrow().as_ref().map_or("Choose a .cht file".into(), |p| glib::markup_escape_text(&p.display().to_string()).to_string()));
        let name = adw::EntryRow::builder().title("Profile Name").text("scanner-it8").build();
        let status = gtk::Label::builder().wrap(true).xalign(0.0).css_classes(["dim-label"]).margin_top(6).build();

        let dialog = adw::Window::builder().transient_for(self).modal(true).title("Calibrate with IT8 Target").default_width(560).default_height(760).build();
        type OnPick = Rc<dyn Fn(&std::path::Path)>;
        let pick = |row: adw::ActionRow,
                    btn: gtk::Button,
                    slot: Rc<RefCell<Option<PathBuf>>>,
                    dialog: adw::Window,
                    title: &'static str,
                    kind: Option<(&'static str, &'static [&'static str])>,
                    on_pick: Option<OnPick>| {
            btn.connect_clicked(move |_| {
                let (row, slot, on_pick) = (row.clone(), slot.clone(), on_pick.clone());
                let mut chooser = gtk::FileDialog::builder().title(title).modal(true);
                if let Some((label, patterns)) = kind {
                    let filter = gtk::FileFilter::new();
                    filter.set_name(Some(label));
                    for pat in patterns {
                        filter.add_pattern(pat);
                    }
                    let all = gtk::FileFilter::new();
                    all.set_name(Some("All files"));
                    all.add_pattern("*");
                    let list = gio::ListStore::new::<gtk::FileFilter>();
                    list.append(&filter);
                    list.append(&all);
                    chooser = chooser.filters(&list).default_filter(&filter);
                }
                chooser.build().open(Some(&dialog), None::<&gio::Cancellable>, move |r| {
                    if let Some(p) = r.ok().and_then(|f| f.path()) {
                        row.set_subtitle(&glib::markup_escape_text(&p.display().to_string()));
                        *slot.borrow_mut() = Some(p.clone());
                        if let Some(f) = &on_pick {
                            f(&p);
                        }
                    }
                });
            });
        };
        // Naming the profile after the data file keeps profiles for different targets apart.
        let is_zip = |p: &std::path::Path| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip"));

        // Which batch the scan matches best, worked out in the background once the scan and the ZIP are both known.
        let refresh_suggestion: Rc<dyn Fn()> = {
            let (scan, reference, chart, batches, suggest, suggestion) = (scan.clone(), reference.clone(), chart.clone(), batches.clone(), suggest.clone(), suggestion.clone());
            Rc::new(move || {
                *suggestion.borrow_mut() = None;
                suggest.set_sensitive(false);
                let (Some(scan), Some(reference), Some(chart)) = (scan.borrow().clone(), reference.borrow().clone(), chart.borrow().clone()) else { return };
                let candidates = batches.borrow().clone();
                if !is_zip(&reference) || candidates.is_empty() {
                    return;
                }
                let (suggest, suggestion) = (suggest.clone(), suggestion.clone());
                glib::MainContext::default().spawn_local(async move {
                    let job = nv_calibrate::Job { scan, chart, reference, out_dir: nv_calibrate::profiles_dir(), name: "batch-search".into(), corners: None };
                    if let Ok(Ok(ranked)) = gio::spawn_blocking(move || nv_calibrate::batch::rank(&job, &candidates)).await
                        && let Some(best) = ranked.first()
                    {
                        suggest.set_label(&format!("Use {}", best.name));
                        suggest.set_sensitive(true);
                        *suggestion.borrow_mut() = Some(best.name.clone());
                    }
                });
            })
        };
        {
            let (batch_row, suggestion) = (batch_row.clone(), suggestion.clone());
            suggest.connect_clicked(move |_| {
                if let Some(n) = suggestion.borrow().as_ref() {
                    batch_row.set_text(n);
                }
            });
        }
        // What was typed is checked against the ZIP as it is typed.
        {
            let (batches, status, name) = (batches.clone(), status.clone(), name.clone());
            batch_row.connect_changed(move |row| {
                let typed = row.text();
                row.remove_css_class("error");
                if typed.trim().is_empty() {
                    return;
                }
                match nv_calibrate::batch::find_batch(&batches.borrow(), &typed) {
                    Some(c) => {
                        status.set_label(&format!("Found batch {} in that file.", c.name));
                        let current = name.text();
                        if current == "scanner-it8" || current.starts_with("it8-") {
                            name.set_text(&format!("it8-{}", c.name));
                        }
                    }
                    None => {
                        row.add_css_class("error");
                        status.set_label(&format!("There is no batch {} in that file. Check the code printed on the slide.", typed.trim()));
                    }
                }
            });
        }
        let on_reference: OnPick = {
            let (name, batch_row, batches, status, refresh) = (name.clone(), batch_row.clone(), batches.clone(), status.clone(), refresh_suggestion.clone());
            Rc::new(move |p| {
                batch_row.set_visible(is_zip(p));
                if is_zip(p) {
                    match std::fs::read(p).map_err(|e| e.to_string()).and_then(|b| nv_calibrate::batch::candidates_from_zip(&b).map_err(|e| e.to_string())) {
                        Ok(found) => {
                            status.set_label(&format!("{} batches in that file. Type the code printed on the slide: it is in the picture above.", found.len()));
                            *batches.borrow_mut() = found;
                            refresh();
                        }
                        Err(e) => {
                            batch_row.set_visible(false);
                            status.set_label(&format!("That file could not be used: {e}"));
                        }
                    }
                } else if let Some(stem) = p.file_stem().and_then(|s| s.to_str()).filter(|s| !s.is_empty() && !s.contains(['/', '\\'])) {
                    let current = name.text();
                    if current == "scanner-it8" || current.starts_with("it8-") {
                        name.set_text(&format!("it8-{stem}"));
                    }
                }
            })
        };
        let on_scan: OnPick = {
            let (load, refresh) = (load_preview.clone(), refresh_suggestion.clone());
            Rc::new(move |p| {
                load(p);
                refresh();
            })
        };
        if let Some(p) = scan.borrow().as_ref() {
            load_preview(p);
        }
        // The ZIP people are told to download is often already in the Downloads folder.
        if let Some(found) = downloaded_data() {
            ref_row.set_subtitle(&glib::markup_escape_text(&found.display().to_string()));
            *reference.borrow_mut() = Some(found.clone());
            on_reference(&found);
        }
        pick(scan_row.clone(), scan_btn, scan.clone(), dialog.clone(), "Scan of the Target", None, Some(on_scan));
        pick(
            ref_row.clone(),
            ref_btn,
            reference.clone(),
            dialog.clone(),
            "Target Data",
            Some(("Target data (.zip, .txt, .it8)", &["*.zip", "*.ZIP", "*.txt", "*.TXT", "*.it8", "*.IT8"])),
            Some(on_reference),
        );
        pick(chart_row.clone(), chart_btn, chart.clone(), dialog.clone(), "Chart Layout", Some(("Chart layouts (.cht)", &["*.cht"])), None);

        {
            let (weak, dialog) = (self.downgrade(), dialog.clone());
            scan_now.connect_clicked(move |_| {
                if let Some(win) = weak.upgrade() {
                    dialog.close();
                    win.begin_calibration_scan();
                }
            });
        }

        {
            let dialog = dialog.clone();
            get_data.connect_clicked(move |_| {
                gtk::UriLauncher::new("https://gwbcolourcard.uk/it8.html").launch(Some(&dialog), None::<&gio::Cancellable>, |_| {});
            });
        }

        let scan_group = adw::PreferencesGroup::builder()
            .description("Put the IT8 slide in the scanner and press Scan Now, or choose a scan of it you made earlier. The profile is used for slides, and is applied to this frame when it is built.")
            .build();
        scan_group.add(&scan_row);
        let data_group = adw::PreferencesGroup::new();
        for r in [ref_row.upcast_ref::<gtk::Widget>(), batch_row.upcast_ref(), chart_row.upcast_ref(), name.upcast_ref()] {
            data_group.add(r);
        }
        let page = adw::PreferencesPage::new();
        page.add(&scan_group);
        page.add(&preview_group);
        page.add(&data_group);
        let result_group = adw::PreferencesGroup::new();
        result_group.add(&status);
        page.add(&result_group);

        let cancel = gtk::Button::builder().label("Close").build();
        let build = gtk::Button::builder().label("Build Profile").css_classes(["suggested-action"]).build();
        let header = adw::HeaderBar::builder().show_end_title_buttons(false).show_start_title_buttons(false).build();
        header.pack_start(&cancel);
        header.pack_end(&build);
        // What the dialog is doing or has done, where it cannot be missed: the status text lives at the
        // bottom of a page that scrolls, so it is repeated in a banner under the header.
        let banner = adw::Banner::builder().revealed(false).build();
        status.set_visible(false);
        {
            let banner = banner.clone();
            status.connect_label_notify(move |l| {
                banner.set_title(&l.label());
                banner.set_revealed(!l.label().is_empty());
            });
        }
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&header);
        content.append(&banner);
        content.append(&page);
        page.set_vexpand(true);
        dialog.set_content(Some(&content));

        let d = dialog.clone();
        cancel.connect_clicked(move |_| d.close());

        let weak = self.downgrade();
        let (build_btn, status2, batch_row, done_btn) = (build.clone(), status.clone(), batch_row.clone(), cancel.clone());
        build.connect_clicked(move |_| {
            let (Some(scan), Some(reference), Some(chart)) = (scan.borrow().clone(), reference.borrow().clone(), chart.borrow().clone()) else {
                status2.set_label("Choose the scan, the target data and the chart layout first.");
                return;
            };
            let zip = reference.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip"));
            let batch = if zip {
                let typed = batch_row.text().trim().to_owned();
                if typed.is_empty() {
                    status2.set_label("Type the batch code printed on the slide first: the ZIP has the data for every batch.");
                    return;
                }
                Some(typed)
            } else {
                None
            };
            let job = nv_calibrate::Job { scan, chart, reference, out_dir: nv_calibrate::profiles_dir(), name: name.text().to_string(), corners: None };
            build_btn.set_sensitive(false);
            build_btn.set_label("Building…");
            status2.set_label("Finding the patches and building the profile…");
            let (weak, status, build_btn, done_btn) = (weak.clone(), status2.clone(), build_btn.clone(), done_btn.clone());
            glib::MainContext::default().spawn_local(async move {
                let result = gio::spawn_blocking(move || match batch {
                    Some(b) => std::fs::read(&job.reference).map_err(nv_calibrate::Error::from).and_then(|zip| nv_calibrate::batch::calibrate_from_zip(&job, &zip, Some(&b))).map(|(o, _)| o),
                    None => nv_calibrate::calibrate(&job),
                })
                .await;
                build_btn.set_sensitive(true);
                build_btn.set_label("Build Profile");
                match result {
                    Ok(Ok(out)) => {
                        let fmt = |v: Option<f64>| v.map_or("unknown".into(), |e| format!("{e:.1}"));
                        let verdict = match out.quality() {
                            nv_calibrate::Quality::Good => "A good fit.",
                            nv_calibrate::Quality::Fair => "A fair fit. Check the data file is the one for this slide's batch.",
                            nv_calibrate::Quality::Poor => "A poor fit: the data file probably is not the one for this slide, or the scan is not of the whole target.",
                            nv_calibrate::Quality::Unknown => "",
                        };
                        status.set_label(&format!(
                            "Built {} and applied it to this frame.\n{verdict}\nAverage error {} ΔE, worst patch {} (CIE76, lower is better; about 3 or under is good for a profile of this kind).",
                            out.input.name,
                            fmt(out.average_error),
                            fmt(out.peak_error),
                        ));
                        if let Some(win) = weak.upgrade() {
                            win.use_scanner_profile(&out.input.name);
                        }
                        build_btn.set_label("Build Again");
                        done_btn.set_label("Done");
                        done_btn.add_css_class("suggested-action");
                    }
                    Ok(Err(e)) => {
                        let hint = e.hint().map(|h| format!("\n{h}")).unwrap_or_default();
                        status.set_label(&format!("Calibration failed: {e}{hint}"));
                    }
                    Err(_) => status.set_label("Calibration stopped unexpectedly."),
                }
            });
        });
        super::chrome::present_labelled(&dialog);
    }

    /// Preview the slide in the scanner and scan it, raw; the dialog opens again when the scan is saved.
    fn begin_calibration_scan(&self) {
        if self.imp().busy.get() || !self.device_ready() {
            self.show_toast("The scanner is not ready");
            return;
        }
        self.imp().calibrate_after_scan.set(true);
        self.show_toast("Previewing the target, then scanning it");
        self.start_preview();
    }

    /// Select a profile for the open frame, loading it again if it is the one
    /// already in use (it may have just been rebuilt).
    pub(super) fn use_scanner_profile(&self, name: &str) {
        if let Some(p) = self.imp().proc_params.borrow_mut().as_mut()
            && p.input.as_ref().is_some_and(|i| i.name == name)
        {
            p.input = None;
        }
        self.refresh_profile_list();
        let row = self.imp().scanner_profile_row.borrow().clone();
        if let Some(row) = row {
            let at =
                (0..row.model().map_or(0, |m| m.n_items())).find(|&i| row.model().and_then(|m| m.item(i)).and_downcast::<gtk::StringObject>().is_some_and(|s| s.string() == name));
            if let Some(at) = at {
                row.set_selected(at);
            }
        }
        self.on_params_changed();
    }

    /// Fill the scanner-profile row with the profiles built so far.
    pub(super) fn refresh_profile_list(&self) {
        let Some(row) = self.imp().scanner_profile_row.borrow().clone() else { return };
        let keep = self.imp().proc_params.borrow().as_ref().and_then(|p| p.input.as_ref().map(|i| i.name.clone()));
        let mut names = vec!["None".to_owned()];
        names.extend(nv_calibrate::list_profiles());
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        self.imp().updating_ui.set(true);
        row.set_model(Some(&gtk::StringList::new(&refs)));
        row.set_selected(keep.and_then(|k| names.iter().position(|n| *n == k)).unwrap_or(0) as u32);
        self.imp().updating_ui.set(false);
    }
}
