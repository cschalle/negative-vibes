//! The Export dialog (Ctrl+E): every export setting in one place, with a
//! preview of the file name, and Export for the selected frame or all
//! scanned frames.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::ObjectSubclassIsExt;
use nv_export::{ColourSpace, Compression, ExportFormat, ExportSettings, OutputSize};

use super::Window;

const FORMATS: [ExportFormat; 3] = [ExportFormat::Tiff, ExportFormat::Jpeg, ExportFormat::Png];
const COMPRESSIONS: [Compression; 3] = [Compression::None, Compression::Lzw, Compression::Deflate];
const SPACES: [ColourSpace; 2] = [ColourSpace::Srgb, ColourSpace::AdobeRgb];

fn combo(title: &str, items: &[&str], selected: usize) -> adw::ComboRow {
    let row = adw::ComboRow::builder().title(title).model(&gtk::StringList::new(items)).build();
    row.set_selected(selected as u32);
    row
}

impl Window {
    pub(super) fn open_export_dialog(&self) {
        let current = self.export_settings();
        let selected = *self.imp().selected_frame.borrow();

        let preset_names: Vec<&str> = std::iter::once("Custom").chain(ExportSettings::presets().iter().map(|(n, _)| *n)).collect();
        let preset = combo("Preset", &preset_names, 0);
        let format = combo("Format", &["TIFF", "JPEG", "PNG"], FORMATS.iter().position(|f| *f == current.format).unwrap_or(0));
        let depth = combo("Bit Depth", &["8 bit", "16 bit"], usize::from(current.bit_depth == 16));
        let compression = combo("Compression", &["None", "LZW", "Deflate"], COMPRESSIONS.iter().position(|c| *c == current.compression).unwrap_or(0));
        let quality = adw::SpinRow::builder()
            .title("Quality")
            .adjustment(&gtk::Adjustment::new(current.jpeg_quality as f64, 1.0, 100.0, 1.0, 10.0, 0.0))
            .build();
        let space = combo("Colour Space", &["sRGB", "Adobe RGB (1998)"], SPACES.iter().position(|c| *c == current.colour_space).unwrap_or(0));
        // Size: the named sizes, then a custom long edge or percentage.
        let presets = OutputSize::presets();
        let mut size_names: Vec<&str> = presets.iter().map(|(n, _)| *n).collect();
        size_names.extend(["Long edge…", "Percent…"]);
        let size_idx = |s: OutputSize| match presets.iter().position(|(_, p)| *p == s) {
            Some(i) => i,
            None => match s {
                OutputSize::Percent { .. } => presets.len() + 1,
                _ => presets.len(),
            },
        };
        let size = combo("Size", &size_names, size_idx(current.size));
        let size_value = adw::SpinRow::builder()
            .title("Pixels")
            .adjustment(&gtk::Adjustment::new(
                match current.size {
                    OutputSize::LongEdge { pixels } => pixels as f64,
                    OutputSize::Percent { percent } => percent as f64,
                    _ => 2048.0,
                },
                1.0,
                100000.0,
                1.0,
                100.0,
                0.0,
            ))
            .build();
        let grey = adw::SwitchRow::builder().title("Grey Output").subtitle("One grey channel, for black and white film").active(current.grey).build();
        let embed = adw::SwitchRow::builder().title("Embed Colour Profile").active(current.embed_profile).build();
        let template = adw::EntryRow::builder().title("File Name").text(&current.template).build();
        let preview = adw::ActionRow::builder().title("Example").subtitle("").build();
        preview.add_css_class("property");
        let after_scan = adw::SwitchRow::builder().title("Export After Each Scan").active(current.after_scan).build();
        let frames = combo("Frames", &["Selected Frame", "All Scanned Frames"], 0);

        let folder: Rc<RefCell<Option<std::path::PathBuf>>> = Rc::new(RefCell::new(current.folder.clone()));
        let folder_row = adw::ActionRow::builder().title("Folder").build();
        let choose = gtk::Button::builder().label("Choose…").valign(gtk::Align::Center).build();
        let default_btn = gtk::Button::builder().label("Default").valign(gtk::Align::Center).css_classes(["flat"]).tooltip_text("Use ~/Pictures/Scans/<roll>").build();
        folder_row.add_suffix(&default_btn);
        folder_row.add_suffix(&choose);

        // The dialog's widgets as an ExportSettings.
        let read = {
            let (format, depth, compression, quality, space, grey, embed, template, after_scan, folder, size, size_value) =
                (format.clone(), depth.clone(), compression.clone(), quality.clone(), space.clone(), grey.clone(), embed.clone(), template.clone(), after_scan.clone(), folder.clone(), size.clone(), size_value.clone());
            Rc::new(move || ExportSettings {
                format: FORMATS[(format.selected() as usize).min(2)],
                bit_depth: if depth.selected() == 1 { 16 } else { 8 },
                compression: COMPRESSIONS[(compression.selected() as usize).min(2)],
                jpeg_quality: quality.value() as u8,
                colour_space: SPACES[(space.selected() as usize).min(1)],
                size: {
                    let n = OutputSize::presets().len();
                    match size.selected() as usize {
                        i if i < n => OutputSize::presets()[i].1,
                        i if i == n => OutputSize::LongEdge { pixels: size_value.value() as u32 },
                        _ => OutputSize::Percent { percent: size_value.value() as f32 },
                    }
                },
                grey: grey.is_active(),
                embed_profile: embed.is_active(),
                folder: folder.borrow().clone(),
                template: template.text().to_string(),
                after_scan: after_scan.is_active(),
            })
        };

        // What applies to what, and the example name.
        let refresh = {
            let (win, read, depth, compression, quality, preview, folder_row, folder, size, size_value) =
                (self.downgrade(), read.clone(), depth.clone(), compression.clone(), quality.clone(), preview.clone(), folder_row.clone(), folder.clone(), size.clone(), size_value.clone());
            Rc::new(move || {
                let Some(win) = win.upgrade() else { return };
                let s = read();
                let jpeg = s.format == ExportFormat::Jpeg;
                depth.set_sensitive(!jpeg);
                if jpeg {
                    depth.set_selected(0);
                }
                compression.set_sensitive(s.format == ExportFormat::Tiff);
                quality.set_sensitive(jpeg);
                let n = OutputSize::presets().len();
                let custom = size.selected() as usize >= n;
                size_value.set_visible(custom);
                size_value.set_title(if size.selected() as usize == n { "Long Edge (px)" } else { "Percent" });
                let example_frame = selected.or_else(|| win.imp().roll.borrow().as_ref().and_then(|r| r.frames.first().map(|f| f.index))).unwrap_or(1);
                match win.export_stem(example_frame, &s.template) {
                    Ok(stem) => {
                        preview.remove_css_class("error");
                        preview.set_subtitle(&format!("{stem}.{}", s.format.extension()));
                    }
                    Err(e) => {
                        preview.add_css_class("error");
                        preview.set_subtitle(&e.to_string());
                    }
                }
                let shown = win.export_folder(&s);
                folder_row.set_subtitle(&format!("{}{}", shown.display(), if folder.borrow().is_none() { " (default)" } else { "" }));
            })
        };
        refresh();

        for row in [&format, &depth, &compression, &space, &size] {
            let refresh = refresh.clone();
            row.connect_selected_notify(move |_| refresh());
        }
        {
            let refresh = refresh.clone();
            template.connect_changed(move |_| refresh());
        }
        {
            let refresh = refresh.clone();
            size_value.connect_value_notify(move |_| refresh());
        }

        // A preset fills in the format, depth, compression, quality and colour; the folder and name stay.
        {
            let (format, depth, compression, quality, space) = (format.clone(), depth.clone(), compression.clone(), quality.clone(), space.clone());
            let refresh = refresh.clone();
            preset.connect_selected_notify(move |row| {
                let Some((_, p)) = (row.selected() as usize).checked_sub(1).and_then(|i| ExportSettings::presets().into_iter().nth(i)) else { return };
                format.set_selected(FORMATS.iter().position(|f| *f == p.format).unwrap_or(0) as u32);
                depth.set_selected(u32::from(p.bit_depth == 16));
                compression.set_selected(COMPRESSIONS.iter().position(|c| *c == p.compression).unwrap_or(0) as u32);
                quality.set_value(p.jpeg_quality as f64);
                space.set_selected(SPACES.iter().position(|c| *c == p.colour_space).unwrap_or(0) as u32);
                refresh();
            });
        }

        let dialog = adw::Window::builder().transient_for(self).modal(true).title("Export").default_width(480).default_height(720).build();

        {
            let (folder, refresh, dialog) = (folder.clone(), refresh.clone(), dialog.clone());
            choose.connect_clicked(move |_| {
                let (folder, refresh) = (folder.clone(), refresh.clone());
                gtk::FileDialog::builder().title("Export Folder").modal(true).build().select_folder(Some(&dialog), None::<&gio::Cancellable>, move |r| {
                    if let Some(path) = r.ok().and_then(|f| f.path()) {
                        *folder.borrow_mut() = Some(path);
                        refresh();
                    }
                });
            });
        }
        {
            let (folder, refresh) = (folder.clone(), refresh.clone());
            default_btn.connect_clicked(move |_| {
                *folder.borrow_mut() = None;
                refresh();
            });
        }

        let page = adw::PreferencesPage::new();
        let g = |title: &str| adw::PreferencesGroup::builder().title(title).build();
        let (g_preset, g_format, g_colour, g_files, g_which) = (g(""), g("Format"), g("Colour"), g("Files"), g(""));
        g_preset.add(&preset);
        for r in [&format, &depth, &compression] {
            g_format.add(r);
        }
        g_format.add(&quality);
        g_format.add(&size);
        g_format.add(&size_value);
        for w in [space.upcast_ref::<gtk::Widget>(), grey.upcast_ref(), embed.upcast_ref()] {
            g_colour.add(w);
        }
        g_files.add(&folder_row);
        g_files.add(&template);
        g_files.add(&preview);
        g_files.add(&after_scan);
        g_which.add(&frames);
        for grp in [&g_preset, &g_format, &g_colour, &g_files, &g_which] {
            page.add(grp);
        }

        let cancel = gtk::Button::builder().label("Cancel").build();
        let export = gtk::Button::builder().label("Export").css_classes(["suggested-action"]).build();
        let header = adw::HeaderBar::builder().show_end_title_buttons(false).show_start_title_buttons(false).build();
        header.pack_start(&cancel);
        header.pack_end(&export);
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&page));
        dialog.set_content(Some(&toolbar));

        {
            let dialog = dialog.clone();
            cancel.connect_clicked(move |_| dialog.close());
        }
        {
            let (weak, dialog, read, frames) = (self.downgrade(), dialog.clone(), read.clone(), frames.clone());
            export.connect_clicked(move |_| {
                let Some(win) = weak.upgrade() else { return };
                let settings = read().normalised();
                if let Err(e) = win.export_stem(1, &settings.template) {
                    win.show_toast(&format!("The file name is not valid: {e}"));
                    return;
                }
                if let Some(roll) = win.imp().roll.borrow_mut().as_mut() {
                    roll.settings.export = settings;
                }
                win.schedule_autosave();
                let targets: Vec<u32> = if frames.selected() == 1 {
                    win.imp().roll.borrow().as_ref().map(|r| r.frames.iter().filter(|f| f.raw.is_some()).map(|f| f.index).collect()).unwrap_or_default()
                } else {
                    selected.into_iter().collect()
                };
                dialog.close();
                if targets.is_empty() {
                    win.show_toast("Choose a scanned frame to export");
                } else {
                    win.queue_export(&targets);
                }
            });
        }
        dialog.present();
    }
}
