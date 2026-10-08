//! Film profiles in the app: Create Film Profile… from a scan of the base
//! and a step wedge, import, export and delete (DESIGN §7.4).

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::ObjectSubclassIsExt;
use nv_film::builder;
use nv_film::{FilmKind, FilmProfile};

use super::Window;

const KINDS: [(&str, FilmKind); 3] = [("Colour negative", FilmKind::ColourNegative), ("Black and white negative", FilmKind::BwNegative), ("Slide", FilmKind::ColourReversal)];

impl Window {
    /// The profiles in the film profile row, in its order.
    pub(super) fn film_profile_list(&self) -> Vec<FilmProfile> {
        let cached = self.imp().film_profiles.borrow().clone();
        if cached.is_empty() { nv_film::all_profiles() } else { cached }
    }

    /// Read the library again and refill the row, keeping the selection.
    pub(super) fn refresh_film_profiles(&self) {
        let list = nv_film::all_profiles();
        let keep = self.imp().proc_params.borrow().as_ref().map(|p| p.profile.id.clone());
        *self.imp().film_profiles.borrow_mut() = list.clone();
        let Some(row) = self.imp().profile_row.borrow().clone() else { return };
        let names: Vec<&str> = list.iter().map(|p| p.name.as_str()).collect();
        self.imp().updating_ui.set(true);
        row.set_model(Some(&gtk::StringList::new(&names)));
        row.set_selected(keep.and_then(|k| list.iter().position(|p| p.id == k)).unwrap_or(0) as u32);
        self.imp().updating_ui.set(false);
    }

    pub(super) fn open_film_profile_dialog(&self) {
        let vendor = adw::EntryRow::builder().title("Maker").build();
        let name = adw::EntryRow::builder().title("Film Name").build();
        let iso = adw::SpinRow::builder().title("ISO").adjustment(&gtk::Adjustment::new(100.0, 0.0, 6400.0, 50.0, 100.0, 0.0)).build();
        let kind = adw::ComboRow::builder().title("Type").model(&gtk::StringList::new(&KINDS.map(|k| k.0))).build();
        let steps = adw::SpinRow::builder().title("Wedge Steps").subtitle("How many steps the wedge has in the cropped area").adjustment(&gtk::Adjustment::new(21.0, 6.0, 64.0, 1.0, 5.0, 0.0)).build();
        let status = gtk::Label::builder().wrap(true).xalign(0.0).css_classes(["dim-label"]).margin_top(6).build();

        let group = adw::PreferencesGroup::builder()
            .description("Scan the unexposed film base together with a step wedge exposed on the same film. Set the Film Base with the pipette on the unexposed film, and crop the frame round the wedge so it fills the cropped area end to end. The profile applies to scans made on this scanner.")
            .build();
        for r in [vendor.upcast_ref::<gtk::Widget>(), name.upcast_ref(), iso.upcast_ref(), kind.upcast_ref(), steps.upcast_ref()] {
            group.add(r);
        }
        let result = adw::PreferencesGroup::new();
        result.add(&status);
        let page = adw::PreferencesPage::new();
        page.add(&group);
        page.add(&result);

        let dialog = adw::Window::builder().transient_for(self).modal(true).title("Create Film Profile").default_width(520).default_height(600).build();
        let close = gtk::Button::builder().label("Close").build();
        let create = gtk::Button::builder().label("Create").css_classes(["suggested-action"]).build();
        let header = adw::HeaderBar::builder().show_end_title_buttons(false).show_start_title_buttons(false).build();
        header.pack_start(&close);
        header.pack_end(&create);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&header);
        content.append(&page);
        dialog.set_content(Some(&content));
        let d = dialog.clone();
        close.connect_clicked(move |_| d.close());

        let weak = self.downgrade();
        let status2 = status.clone();
        create.connect_clicked(move |_| {
            let Some(win) = weak.upgrade() else { return };
            let imp = win.imp();
            let (Some(proxy), Some(params)) = (imp.proxy.borrow().clone(), imp.proc_params.borrow().clone()) else {
                status2.set_label("Open the scan of the wedge first.");
                return;
            };
            if proxy.is_empty() {
                status2.set_label("Open the scan of the wedge first.");
                return;
            }
            let (w, h) = (proxy.width, proxy.height);
            let r = params.conversion.region.unwrap_or([0.0, 0.0, 1.0, 1.0]);
            let rect = [(r[0] as f64 * w as f64) as usize, (r[1] as f64 * h as f64) as usize, (r[2] as f64 * w as f64) as usize, (r[3] as f64 * h as f64) as usize];
            let base = params.conversion.film_base.unwrap_or_else(|| nv_process::estimate_film_base_proxy(&proxy));
            let measured = builder::Measurement { base, steps: builder::measure_wedge(&proxy.pixels, w, h, rect, steps.value() as usize) };
            let info = builder::Info {
                vendor: vendor.text().to_string(),
                name: name.text().to_string(),
                iso: iso.value() as u32,
                kind: KINDS[kind.selected() as usize].1.clone(),
                device: match &*win.imp().scan_state.borrow() {
                    super::imp::ScanUiState::Ready { device_name } => device_name.clone(),
                    _ => "unknown scanner".into(),
                },
                date: today(),
            };
            match builder::build(&info, &measured).map_err(|e| e.to_string()).and_then(|(p, rep)| nv_film::library::save(&p).map(|_| (p, rep)).map_err(|e| e.to_string())) {
                Ok((p, rep)) => {
                    status2.set_label(&format!(
                        "Saved “{} {}”. Used {} steps; red and blue follow green with correlation {:.3} and {:.3}. Gamma {:.2} / {:.2} / {:.2}.",
                        p.vendor, p.name, rep.steps_used, rep.fit[0], rep.fit[1], p.gamma[0], p.gamma[1], p.gamma[2]
                    ));
                    win.refresh_film_profiles();
                }
                Err(e) => status2.set_label(&format!("Could not create the profile: {e}")),
            }
        });
        dialog.present();
    }

    pub(super) fn import_film_profile(&self) {
        let weak = self.downgrade();
        gtk::FileDialog::builder().title("Import Film Profile").modal(true).build().open(Some(self), None::<&gio::Cancellable>, move |r| {
            let (Some(win), Some(path)) = (weak.upgrade(), r.ok().and_then(|f| f.path())) else { return };
            match nv_film::library::import(&path) {
                Ok(p) => {
                    win.refresh_film_profiles();
                    win.show_toast(&format!("Imported {} {}", p.vendor, p.name));
                }
                Err(e) => win.show_toast(&format!("Not imported: {e}")),
            }
        });
    }

    pub(super) fn export_film_profile(&self) {
        let Some(id) = self.imp().proc_params.borrow().as_ref().map(|p| p.profile.id.clone()) else { return };
        let weak = self.downgrade();
        gtk::FileDialog::builder().title("Export Film Profile").initial_name(format!("{id}.json")).modal(true).build().save(Some(self), None::<&gio::Cancellable>, move |r| {
            let (Some(win), Some(path)) = (weak.upgrade(), r.ok().and_then(|f| f.path())) else { return };
            match nv_film::library::export(&id, &path) {
                Ok(()) => win.show_toast("Profile exported"),
                Err(e) => win.show_toast(&format!("Not exported: {e}")),
            }
        });
    }

    pub(super) fn delete_film_profile(&self) {
        let Some(id) = self.imp().proc_params.borrow().as_ref().map(|p| p.profile.id.clone()) else { return };
        match nv_film::library::delete(&id) {
            Ok(()) => {
                self.refresh_film_profiles();
                self.show_toast("Profile deleted");
            }
            Err(e) => self.show_toast(&format!("Not deleted: {e}")),
        }
    }
}

/// Today as `YYYY-MM-DD`, in local time.
fn today() -> String {
    glib::DateTime::now_local().ok().and_then(|d| d.format("%Y-%m-%d").ok()).map(|s| s.to_string()).unwrap_or_default()
}

#[allow(unused)]
type Shared<T> = Rc<RefCell<T>>;
