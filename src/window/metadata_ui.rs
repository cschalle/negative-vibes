//! The Metadata dialog (Ctrl+I): the fields of F§10 for the roll, with
//! presets. They are written into every exported file as XMP.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::ObjectSubclassIsExt;
use nv_export::metadata::{self, Metadata};

use super::Window;

fn entry(title: &str, text: &str) -> adw::EntryRow {
    adw::EntryRow::builder().title(title).text(text).build()
}

impl Window {
    pub(super) fn open_metadata_dialog(&self) {
        let current = self.export_settings().metadata;

        let creator = entry("Photographer", &current.creator);
        let title = entry("Title", &current.title);
        let headline = entry("Headline", &current.headline);
        let caption = entry("Caption", &current.caption);
        let keywords = entry("Keywords (separated by commas)", &current.keywords.join(", "));
        let copyright = entry("Copyright", &current.copyright);
        let credit = entry("Credit", &current.credit);
        let source = entry("Source", &current.source);
        let instructions = entry("Special Instructions", &current.instructions);
        let city = entry("City", &current.city);
        let province = entry("State or Province", &current.province);
        let country = entry("Country", &current.country);
        let date = entry("Date Taken (2024, 2024-06 or 2024-06-15)", &current.date_taken);
        let camera = entry("Camera", &current.camera);
        let lens = entry("Lens", &current.lens);
        let film = entry("Film (blank: from the film profile)", &current.film_stock);

        let read: Rc<dyn Fn() -> Metadata> = {
            let f = (creator.clone(), title.clone(), headline.clone(), caption.clone(), keywords.clone(), copyright.clone(), credit.clone(), source.clone(), instructions.clone(), city.clone(), province.clone(), country.clone(), date.clone(), camera.clone(), lens.clone(), film.clone());
            Rc::new(move || Metadata {
                creator: f.0.text().to_string(),
                title: f.1.text().to_string(),
                headline: f.2.text().to_string(),
                caption: f.3.text().to_string(),
                keywords: Metadata::parse_keywords(&f.4.text()),
                copyright: f.5.text().to_string(),
                credit: f.6.text().to_string(),
                source: f.7.text().to_string(),
                instructions: f.8.text().to_string(),
                city: f.9.text().to_string(),
                province: f.10.text().to_string(),
                country: f.11.text().to_string(),
                date_taken: f.12.text().to_string(),
                camera: f.13.text().to_string(),
                lens: f.14.text().to_string(),
                film_stock: f.15.text().to_string(),
            })
        };
        let fill: Rc<dyn Fn(&Metadata)> = {
            let f = (creator.clone(), title.clone(), headline.clone(), caption.clone(), keywords.clone(), copyright.clone(), credit.clone(), source.clone(), instructions.clone(), city.clone(), province.clone(), country.clone(), date.clone(), camera.clone(), lens.clone(), film.clone());
            Rc::new(move |m: &Metadata| {
                f.0.set_text(&m.creator);
                f.1.set_text(&m.title);
                f.2.set_text(&m.headline);
                f.3.set_text(&m.caption);
                f.4.set_text(&m.keywords.join(", "));
                f.5.set_text(&m.copyright);
                f.6.set_text(&m.credit);
                f.7.set_text(&m.source);
                f.8.set_text(&m.instructions);
                f.9.set_text(&m.city);
                f.10.set_text(&m.province);
                f.11.set_text(&m.country);
                f.12.set_text(&m.date_taken);
                f.13.set_text(&m.camera);
                f.14.set_text(&m.lens);
                f.15.set_text(&m.film_stock);
            })
        };

        // Presets.
        let presets: Rc<RefCell<Vec<(String, Metadata)>>> = Rc::new(RefCell::new(metadata::list_presets()));
        let preset_row = adw::ComboRow::builder().title("Preset").build();
        let refill = {
            let (presets, preset_row) = (presets.clone(), preset_row.clone());
            Rc::new(move || {
                *presets.borrow_mut() = metadata::list_presets();
                let mut names = vec!["Custom".to_owned()];
                names.extend(presets.borrow().iter().map(|p| p.0.clone()));
                let refs: Vec<&str> = names.iter().map(String::as_str).collect();
                preset_row.set_model(Some(&gtk::StringList::new(&refs)));
            })
        };
        refill();
        {
            let (presets, fill) = (presets.clone(), fill.clone());
            preset_row.connect_selected_notify(move |r| {
                if let Some(i) = (r.selected() as usize).checked_sub(1) {
                    if let Some((_, m)) = presets.borrow().get(i) {
                        fill(m);
                    }
                }
            });
        }
        let preset_name = entry("Save These Fields as Preset", "");
        let save_preset = gtk::Button::builder().label("Save").valign(gtk::Align::Center).build();
        let delete_preset = gtk::Button::builder().label("Delete Selected").valign(gtk::Align::Center).css_classes(["destructive-action"]).build();
        preset_name.add_suffix(&save_preset);
        let status = gtk::Label::builder().wrap(true).xalign(0.0).css_classes(["dim-label"]).build();

        let preset_group = adw::PreferencesGroup::new();
        preset_group.add(&preset_row);
        preset_group.add(&preset_name);
        preset_group.add(&delete_preset);

        let people = adw::PreferencesGroup::builder().title("Picture").build();
        for r in [&creator, &title, &headline, &caption, &keywords, &copyright, &credit, &source, &instructions] {
            people.add(r);
        }
        let place = adw::PreferencesGroup::builder().title("Place and Time").build();
        for r in [&city, &province, &country, &date] {
            place.add(r);
        }
        let gear = adw::PreferencesGroup::builder().title("Original Photograph").description("The scanner, resolution, roll, frame and scan date are added automatically.").build();
        for r in [&camera, &lens, &film] {
            gear.add(r);
        }
        let status_group = adw::PreferencesGroup::new();
        status_group.add(&status);
        let page = adw::PreferencesPage::new();
        for g in [&preset_group, &people, &place, &gear, &status_group] {
            page.add(g);
        }

        let dialog = adw::Window::builder().transient_for(self).modal(true).title("Metadata").default_width(520).default_height(760).build();
        let cancel = gtk::Button::builder().label("Cancel").build();
        let apply = gtk::Button::builder().label("Apply to Roll").css_classes(["suggested-action"]).build();
        let header = adw::HeaderBar::builder().show_end_title_buttons(false).show_start_title_buttons(false).build();
        header.pack_start(&cancel);
        header.pack_end(&apply);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&header);
        content.append(&page);
        dialog.set_content(Some(&content));
        let d = dialog.clone();
        cancel.connect_clicked(move |_| d.close());

        {
            let (read, status, preset_name, refill) = (read.clone(), status.clone(), preset_name.clone(), refill.clone());
            save_preset.connect_clicked(move |_| match metadata::save_preset(&preset_name.text(), &read()) {
                Ok(_) => {
                    status.set_label("Preset saved.");
                    refill();
                }
                Err(e) => status.set_label(&format!("Not saved: {e}")),
            });
        }
        {
            let (presets, preset_row, status, refill) = (presets.clone(), preset_row.clone(), status.clone(), refill.clone());
            delete_preset.connect_clicked(move |_| {
                let Some(i) = (preset_row.selected() as usize).checked_sub(1) else {
                    status.set_label("Choose a preset to delete.");
                    return;
                };
                let name = presets.borrow().get(i).map(|p| p.0.clone()).unwrap_or_default();
                match metadata::delete_preset(&name) {
                    Ok(()) => {
                        status.set_label(&format!("Deleted “{name}”."));
                        refill();
                    }
                    Err(e) => status.set_label(&format!("Not deleted: {e}")),
                }
            });
        }
        let weak = self.downgrade();
        let (status2, dialog2) = (status.clone(), dialog.clone());
        apply.connect_clicked(move |_| {
            let Some(win) = weak.upgrade() else { return };
            let meta = read();
            if !metadata::valid_date(&meta.date_taken) {
                status2.set_label("The date must look like 2024, 2024-06 or 2024-06-15.");
                return;
            }
            if let Some(roll) = win.imp().roll.borrow_mut().as_mut() {
                roll.settings.export.metadata = meta;
            } else {
                status2.set_label("Open or create a roll first.");
                return;
            }
            win.schedule_autosave();
            win.show_toast("Metadata saved for this roll");
            dialog2.close();
        });
        dialog.present();
    }
}
