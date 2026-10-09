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

        let (scan_row, scan_btn) = file_row("Scan of the Target", &scan.borrow().as_ref().map_or("Choose the raw scan".into(), |p| glib::markup_escape_text(&p.display().to_string()).to_string()));
        let (ref_row, ref_btn) = file_row("Target Data", "The measured values file that came with the target");
        let (chart_row, chart_btn) = file_row("Chart Layout", &chart.borrow().as_ref().map_or("Choose a .cht file".into(), |p| glib::markup_escape_text(&p.display().to_string()).to_string()));
        let name = adw::EntryRow::builder().title("Profile Name").text("scanner-it8").build();
        let status = gtk::Label::builder().wrap(true).xalign(0.0).css_classes(["dim-label"]).margin_top(6).build();

        let dialog = adw::Window::builder().transient_for(self).modal(true).title("Calibrate with IT8 Target").default_width(520).default_height(560).build();
        let pick = |row: adw::ActionRow, btn: gtk::Button, slot: Rc<RefCell<Option<PathBuf>>>, dialog: adw::Window, title: &'static str| {
            btn.connect_clicked(move |_| {
                let (row, slot) = (row.clone(), slot.clone());
                gtk::FileDialog::builder().title(title).modal(true).build().open(Some(&dialog), None::<&gio::Cancellable>, move |r| {
                    if let Some(p) = r.ok().and_then(|f| f.path()) {
                        row.set_subtitle(&glib::markup_escape_text(&p.display().to_string()));
                        *slot.borrow_mut() = Some(p);
                    }
                });
            });
        };
        pick(scan_row.clone(), scan_btn, scan.clone(), dialog.clone(), "Scan of the Target");
        pick(ref_row.clone(), ref_btn, reference.clone(), dialog.clone(), "Target Data");
        pick(chart_row.clone(), chart_btn, chart.clone(), dialog.clone(), "Chart Layout");

        let group = adw::PreferencesGroup::builder()
            .description("Scan the IT8 slide target with no corrections, then give its data file. The profile is used for slides.")
            .build();
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
                        status.set_label(&format!(
                            "Built {}.\nAverage error {} (ΔE), worst patch {}.",
                            out.profile.display(),
                            out.average_error.map_or("unknown".into(), |e| format!("{e:.2}")),
                            out.peak_error.map_or("unknown".into(), |e| format!("{e:.2}")),
                        ));
                        if let Some(win) = weak.upgrade() {
                            win.refresh_profile_list();
                        }
                    }
                    Ok(Err(e)) => status.set_label(&format!("Calibration failed: {e}")),
                    Err(_) => status.set_label("Calibration stopped unexpectedly."),
                }
            });
        });
        dialog.present();
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
