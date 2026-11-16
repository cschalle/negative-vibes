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

impl Window {
    pub(super) fn open_calibrate_dialog(&self) {
        if !nv_calibrate::available() {
            self.show_toast("Calibration needs ArgyllCMS (scanin, colprof); install it first");
            return;
        }
        let scan: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(self.imp().displayed_file.borrow().clone()));
        let reference: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(None));
        let chart: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(nv_calibrate::default_chart()));

        let (scan_row, scan_btn) =
            file_row("Scan of the Target", &scan.borrow().as_ref().map_or("Choose the raw scan".into(), |p| glib::markup_escape_text(&p.display().to_string()).to_string()));
        let (ref_row, ref_btn) = file_row("Target Data", "The measured values file that came with the target");
        let (chart_row, chart_btn) =
            file_row("Chart Layout", &chart.borrow().as_ref().map_or("Choose a .cht file".into(), |p| glib::markup_escape_text(&p.display().to_string()).to_string()));
        let name = adw::EntryRow::builder().title("Profile Name").text("scanner-it8").build();
        let status = gtk::Label::builder().wrap(true).xalign(0.0).css_classes(["dim-label"]).margin_top(6).build();

        let dialog = adw::Window::builder().transient_for(self).modal(true).title("Calibrate with IT8 Target").default_width(520).default_height(560).build();
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
                        if let Some(f) = &on_pick {
                            f(&p);
                        }
                        *slot.borrow_mut() = Some(p);
                    }
                });
            });
        };
        // Naming the profile after the data file keeps profiles for different targets apart.
        let name_for_ref = name.clone();
        let name_from_data: OnPick = Rc::new(move |p| {
            let current = name_for_ref.text();
            if current == "scanner-it8" || current.starts_with("it8-") {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()).filter(|s| !s.is_empty() && !s.contains(['/', '\\'])) {
                    name_for_ref.set_text(&format!("it8-{stem}"));
                }
            }
        });
        pick(scan_row.clone(), scan_btn, scan.clone(), dialog.clone(), "Scan of the Target", None, None);
        pick(
            ref_row.clone(),
            ref_btn,
            reference.clone(),
            dialog.clone(),
            "Target Data",
            Some(("Target data (.txt, .it8)", &["*.txt", "*.TXT", "*.it8", "*.IT8"])),
            Some(name_from_data),
        );
        pick(chart_row.clone(), chart_btn, chart.clone(), dialog.clone(), "Chart Layout", Some(("Chart layouts (.cht)", &["*.cht"])), None);

        let group = adw::PreferencesGroup::builder().description("Scan the IT8 slide target (the open frame is used if it is that scan), then give the data file for its batch. The profile is used for slides, and is applied to this frame when it is built.").build();
        for r in [scan_row.upcast_ref::<gtk::Widget>(), ref_row.upcast_ref(), chart_row.upcast_ref(), name.upcast_ref()] {
            group.add(r);
        }
        let page = adw::PreferencesPage::new();
        page.add(&group);
        let result_group = adw::PreferencesGroup::new();
        result_group.add(&status);
        page.add(&result_group);

        let cancel = gtk::Button::builder().label("Close").build();
        let build = gtk::Button::builder().label("Build Profile").css_classes(["suggested-action"]).build();
        let header = adw::HeaderBar::builder().show_end_title_buttons(false).show_start_title_buttons(false).build();
        header.pack_start(&cancel);
        header.pack_end(&build);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&header);
        content.append(&page);
        dialog.set_content(Some(&content));

        let d = dialog.clone();
        cancel.connect_clicked(move |_| d.close());

        let weak = self.downgrade();
        let (build_btn, status2) = (build.clone(), status.clone());
        build.connect_clicked(move |_| {
            let (Some(scan), Some(reference), Some(chart)) = (scan.borrow().clone(), reference.borrow().clone(), chart.borrow().clone()) else {
                status2.set_label("Choose the scan, the target data and the chart layout first.");
                return;
            };
            let job = nv_calibrate::Job { scan, chart, reference, out_dir: nv_calibrate::profiles_dir(), name: name.text().to_string(), corners: None };
            build_btn.set_sensitive(false);
            status2.set_label("Finding the patches and building the profile…");
            let (weak, status, build_btn) = (weak.clone(), status2.clone(), build_btn.clone());
            glib::MainContext::default().spawn_local(async move {
                let result = gio::spawn_blocking(move || nv_calibrate::calibrate(&job)).await;
                build_btn.set_sensitive(true);
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

    /// Select a profile for the open frame, loading it again if it is the one
    /// already in use (it may have just been rebuilt).
    pub(super) fn use_scanner_profile(&self, name: &str) {
        if let Some(p) = self.imp().proc_params.borrow_mut().as_mut() {
            if p.input.as_ref().is_some_and(|i| i.name == name) {
                p.input = None;
            }
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
