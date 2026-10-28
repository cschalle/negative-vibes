//! The Export dialog (Ctrl+E): every export setting in one place, with a
//! preview of the file name, and Export for the selected frame or all
//! scanned frames.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::ObjectSubclassIsExt;
use nv_export::{ColourSpace, Compression, ExportFormat, ExportSettings, OutputSize};

use super::Window;

const FORMATS: [ExportFormat; 4] = [ExportFormat::Tiff, ExportFormat::Jpeg, ExportFormat::Png, ExportFormat::Dng];
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
        let format = combo("Format", &["TIFF", "JPEG", "PNG", "DNG (the raw scan)"], FORMATS.iter().position(|f| *f == current.format).unwrap_or(0));
        let depth = combo("Bit Depth", &["8 bit", "16 bit"], usize::from(current.bit_depth == 16));
        let compression = combo("Compression", &["None", "LZW", "Deflate"], COMPRESSIONS.iter().position(|c| *c == current.compression).unwrap_or(0));
        let quality = adw::SpinRow::builder().title("Quality").adjustment(&gtk::Adjustment::new(current.jpeg_quality as f64, 1.0, 100.0, 1.0, 10.0, 0.0)).build();
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
        let embed_meta =
            adw::SwitchRow::builder().title("Embed Metadata").subtitle("The author, caption and film details from Metadata (Ctrl+I)").active(current.embed_metadata).build();
        let after_scan = adw::SwitchRow::builder().title("Export After Each Scan").active(current.after_scan).build();
        let google_sub = if self.google_signed_in() {
            "Signed in. Every picture you export is added to your library"
        } else {
            "Sign in under Preferences, or when asked, to turn this on"
        };
        let google = adw::SwitchRow::builder().title("Send to Google Photos").subtitle(google_sub).active(current.google_photos).build();
        let google_album = adw::SwitchRow::builder().title("Put the Roll in an Album").subtitle("An album named after the roll").active(current.google_album).build();
        // One switch for "scan, and it is in my library": both of the two that make that happen.
        let google_auto = adw::SwitchRow::builder()
            .title("Send Each New Scan")
            .subtitle("Export and send every scan as soon as it is saved. Turns on Send to Google Photos and Export After Each Scan")
            .active(current.google_photos && current.after_scan)
            .build();
        // Export acts on the selected frame, which has to be scanned; when it is not (a new
        // frame that has only been previewed), the scanned ones are the useful default.
        let (selected_scanned, scanned_count) = {
            let roll = self.imp().roll.borrow();
            let scanned = |i: u32| roll.as_ref().and_then(|r| r.frame(i)).is_some_and(|f| f.raw.is_some());
            (selected.is_some_and(scanned), roll.as_ref().map_or(0, |r| r.frames.iter().filter(|f| f.raw.is_some()).count()))
        };
        let frames = combo("Frames", &["Selected Frame", "All Scanned Frames"], usize::from(!selected_scanned && scanned_count > 0));

        let folder: Rc<RefCell<Option<std::path::PathBuf>>> = Rc::new(RefCell::new(current.folder.clone()));
        let folder_row = adw::ActionRow::builder().title("Folder").build();
        let choose = gtk::Button::builder().label("Choose…").valign(gtk::Align::Center).build();
        let default_btn = gtk::Button::builder().label("Default").valign(gtk::Align::Center).css_classes(["flat"]).tooltip_text("Use ~/Pictures/Scans/<roll>").build();
        folder_row.add_suffix(&default_btn);
        folder_row.add_suffix(&choose);

        // The dialog's widgets as an ExportSettings.
        let read = {
            let (format, depth, compression, quality, space, grey, embed, template, after_scan, folder, size, size_value, embed_meta, google, google_album) = (
                format.clone(),
                depth.clone(),
                compression.clone(),
                quality.clone(),
                space.clone(),
                grey.clone(),
                embed.clone(),
                template.clone(),
                after_scan.clone(),
                folder.clone(),
                size.clone(),
                size_value.clone(),
                embed_meta.clone(),
                google.clone(),
                google_album.clone(),
            );
            let metadata = current.metadata.clone();
            Rc::new(move || ExportSettings {
                format: FORMATS[(format.selected() as usize).min(3)],
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
                metadata: metadata.clone(),
                embed_metadata: embed_meta.is_active(),
                google_photos: google.is_active(),
                google_album: google_album.is_active(),
            })
        };

        // What applies to what, and the example name.
        let refresh = {
            let (win, read, depth, compression, quality, preview, folder_row, folder, size, size_value, space, grey, embed, google_album) = (
                self.downgrade(),
                read.clone(),
                depth.clone(),
                compression.clone(),
                quality.clone(),
                preview.clone(),
                folder_row.clone(),
                folder.clone(),
                size.clone(),
                size_value.clone(),
                space.clone(),
                grey.clone(),
                embed.clone(),
                google_album.clone(),
            );
            Rc::new(move || {
                let Some(win) = win.upgrade() else { return };
                let s = read();
                let jpeg = s.format == ExportFormat::Jpeg;
                // A DNG is the scan as it came off the scanner: nothing below applies to it.
                let dng = s.format == ExportFormat::Dng;
                for w in [size.upcast_ref::<gtk::Widget>(), space.upcast_ref(), grey.upcast_ref(), embed.upcast_ref()] {
                    w.set_sensitive(!dng);
                }
                depth.set_sensitive(!jpeg && !dng);
                if jpeg {
                    depth.set_selected(0);
                }
                compression.set_sensitive(s.format == ExportFormat::Tiff);
                google_album.set_sensitive(s.google_photos);
                size_value.set_sensitive(!dng);
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
                folder_row.set_subtitle(&format!("{}{}", glib::markup_escape_text(&shown.display().to_string()), if folder.borrow().is_none() { " (default)" } else { "" }));
            })
        };
        refresh();

        {
            let refresh = refresh.clone();
            google.connect_active_notify(move |_| refresh());
        }
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
            let (format, depth, compression, quality, space, size, size_value, google, after_scan) = (
                format.clone(),
                depth.clone(),
                compression.clone(),
                quality.clone(),
                space.clone(),
                size.clone(),
                size_value.clone(),
                google.clone(),
                after_scan.clone(),
            );
            let refresh = refresh.clone();
            preset.connect_selected_notify(move |row| {
                let Some((_, p)) = (row.selected() as usize).checked_sub(1).and_then(|i| ExportSettings::presets().into_iter().nth(i)) else { return };
                format.set_selected(FORMATS.iter().position(|f| *f == p.format).unwrap_or(0) as u32);
                depth.set_selected(u32::from(p.bit_depth == 16));
                compression.set_selected(COMPRESSIONS.iter().position(|c| *c == p.compression).unwrap_or(0) as u32);
                quality.set_value(p.jpeg_quality as f64);
                space.set_selected(SPACES.iter().position(|c| *c == p.colour_space).unwrap_or(0) as u32);
                match OutputSize::presets().iter().position(|(_, o)| *o == p.size) {
                    Some(i) => size.set_selected(i as u32),
                    None => match p.size {
                        OutputSize::LongEdge { pixels } => {
                            size.set_selected(OutputSize::presets().len() as u32);
                            size_value.set_value(pixels as f64);
                        }
                        OutputSize::Percent { percent } => {
                            size.set_selected(OutputSize::presets().len() as u32 + 1);
                            size_value.set_value(percent as f64);
                        }
                        _ => {}
                    },
                }
                google.set_active(p.google_photos);
                // The Google Photos preset is for sending as you scan.
                if p.google_photos {
                    after_scan.set_active(true);
                }
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

        {
            let syncing = Rc::new(Cell::new(false));
            {
                let (google, after_scan, syncing) = (google.clone(), after_scan.clone(), syncing.clone());
                google_auto.connect_active_notify(move |row| {
                    if syncing.replace(true) {
                        return;
                    }
                    if row.is_active() {
                        google.set_active(true);
                        after_scan.set_active(true);
                    } else {
                        after_scan.set_active(false);
                    }
                    syncing.set(false);
                });
            }
            for real in [&google, &after_scan] {
                let (google, after_scan, google_auto, syncing) = (google.clone(), after_scan.clone(), google_auto.clone(), syncing.clone());
                real.connect_active_notify(move |_| {
                    if syncing.replace(true) {
                        return;
                    }
                    google_auto.set_active(google.is_active() && after_scan.is_active());
                    syncing.set(false);
                });
            }
        }

        let page = adw::PreferencesPage::new();
        let g = |title: &str| adw::PreferencesGroup::builder().title(title).build();
        let (g_preset, g_format, g_colour, g_files, g_google, g_which) = (g(""), g("Format"), g("Colour"), g("Files"), g("Google Photos"), g(""));
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
        g_files.add(&embed_meta);
        g_files.add(&after_scan);
        g_google.add(&google);
        g_google.add(&google_album);
        g_google.add(&google_auto);
        g_which.add(&frames);
        for grp in [&g_preset, &g_format, &g_colour, &g_files, &g_google, &g_which] {
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
            let (frames_row, export) = (frames.clone(), export.clone());
            let check = Rc::new(move || {
                let (frames, all) = (&frames_row, frames_row.selected() == 1);
                let (ok, why) = if all {
                    (scanned_count > 0, "No frame has been scanned yet")
                } else if selected.is_none() {
                    (false, "No frame is selected")
                } else {
                    (selected_scanned, "The selected frame has not been scanned: scan it, or export all scanned frames")
                };
                export.set_sensitive(ok);
                frames.set_subtitle(if ok { "" } else { why });
            });
            check();
            frames.connect_selected_notify(move |_| check());
        }
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
                } else {
                    // No roll yet: the one made by the first scan starts from the preferences,
                    // so hold the choice there for this session (it is not saved to disk).
                    win.imp().prefs.borrow_mut().export = settings;
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
        super::chrome::present_labelled(&dialog);
    }
}
