//! Scan a Roll (DESIGN §8.5): the guided flow for newcomers. It asks what
//! the film is and what the scans are for, previews, offers three looks,
//! scans, and says where the files went. It drives the same actions the main
//! window has, so everything it does can also be done by hand afterwards.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::gdk;
use adw::subclass::prelude::ObjectSubclassIsExt;
use nv_export::Purpose;
use nv_film::{FilmKind, FilmProfile};
use nv_process::params::ImageType;
use nv_process::{Look, Proxy, Settings};

use super::Window;

const STEPS: [&str; 6] = ["Film", "Purpose", "Preview", "Look", "Scan", "Done"];
const TYPES: [(&str, ImageType); 4] = [
    ("Colour negative", ImageType::ColourNegative),
    ("Black and white negative", ImageType::BwNegative),
    ("Slide", ImageType::Positive),
    ("Kodachrome slide", ImageType::Kodachrome),
];

/// Profiles that suit an image type, generic first.
fn stocks_for(t: ImageType) -> Vec<FilmProfile> {
    let kind = |k: &FilmKind| match t {
        ImageType::ColourNegative => *k == FilmKind::ColourNegative,
        ImageType::BwNegative => *k == FilmKind::BwNegative,
        ImageType::Positive | ImageType::Kodachrome => matches!(k, FilmKind::ColourReversal | FilmKind::BwReversal),
    };
    let mut v: Vec<FilmProfile> = nv_film::all_profiles().into_iter().filter(|p| kind(&p.kind)).collect();
    v.sort_by_key(|p| p.vendor != "Generic");
    v
}

/// The listed resolution nearest `want` without going over, else the lowest.
pub(super) fn fit_dpi(list: &[u32], want: u32) -> u32 {
    list.iter().copied().filter(|d| *d <= want).max().or_else(|| list.iter().copied().min()).unwrap_or(want)
}

/// A reduced copy of the proxy for the look thumbnails.
fn reduced(p: &Proxy, side: usize) -> Proxy {
    let step = p.width.max(p.height).div_ceil(side).max(1);
    let (w, h) = (p.width / step, p.height / step);
    let mut px = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * step) * p.width + x * step) * 3;
            px.extend_from_slice(&p.pixels[i..i + 3]);
        }
    }
    Proxy::new(w.max(1), h.max(1), p.factor * step, px)
}

struct Guide {
    stack: gtk::Stack,
    step: Cell<usize>,
    back: gtk::Button,
    next: gtk::Button,
    title: gtk::Label,
    type_row: adw::ComboRow,
    stock_row: adw::ComboRow,
    stocks: RefCell<Vec<FilmProfile>>,
    frames_row: adw::SpinRow,
    purpose_row: adw::ComboRow,
    preview_status: gtk::Label,
    look_box: gtk::Box,
    look_choice: Cell<Option<Look>>,
    scan_status: gtk::Label,
    done_label: gtk::Label,
}

impl Window {
    /// Whether to open the guide as the app starts.
    pub fn should_show_guide(&self) -> bool {
        let headless = ["NV_IMPORT_FILES", "NV_AUTOSCAN", "NV_EXPORT_DIR"].iter().any(|k| std::env::var_os(k).is_some());
        self.imp().prefs.borrow().guided_on_start && !headless
    }

    pub(super) fn open_guided(&self) {
        if let Some(w) = self.imp().guide.borrow().as_ref() {
            w.present();
            return;
        }
        let page = |title: &str, description: &str| {
            let p = adw::PreferencesPage::new();
            let g = adw::PreferencesGroup::builder().title(title).description(description).build();
            p.add(&g);
            (p, g)
        };

        // 1. Film
        let (p1, g1) = page("What are you scanning?", "This sets how the picture is turned into a positive.");
        let type_row = adw::ComboRow::builder().title("Film Type").model(&gtk::StringList::new(&TYPES.map(|t| t.0))).build();
        let stock_row = adw::ComboRow::builder().title("Film Stock").subtitle("Pick the film if it is listed; otherwise leave the generic one").build();
        let frames_row = adw::SpinRow::builder().title("Frames").subtitle("How many frames are on the strip or roll").adjustment(&gtk::Adjustment::new(6.0, 1.0, 100.0, 1.0, 6.0, 0.0)).build();
        let start_row = adw::SwitchRow::builder().title("Show This Guide at Start").active(self.imp().prefs.borrow().guided_on_start).build();
        for r in [type_row.upcast_ref::<gtk::Widget>(), stock_row.upcast_ref(), frames_row.upcast_ref(), start_row.upcast_ref()] {
            g1.add(r);
        }
        // 2. Purpose
        let (p2, g2) = page("What are the scans for?", "This sets the resolution and the files you get.");
        let mut names: Vec<String> = Purpose::ALL.iter().map(|p| format!("{} — {}", p.name(), p.description())).collect();
        names.push("Custom — keep the settings as they are".to_owned());
        let purpose_row = adw::ComboRow::builder().title("Purpose").model(&gtk::StringList::new(&names.iter().map(String::as_str).collect::<Vec<_>>())).build();
        g2.add(&purpose_row);
        // 3. Preview
        let (p3, g3) = page("Preview the film", "Put the film in its holder, then preview. The frames are found for you.");
        let preview_btn = gtk::Button::builder().label("Preview Film").halign(gtk::Align::Start).css_classes(["suggested-action", "pill"]).build();
        let preview_status = gtk::Label::builder().wrap(true).xalign(0.0).css_classes(["dim-label"]).margin_top(8).build();
        let preview_stop = gtk::Button::builder().label("Stop").halign(gtk::Align::Start).build();
        let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.append(&preview_btn);
        row.append(&preview_stop);
        b.append(&row);
        b.append(&preview_status);
        g3.add(&b);
        // 4. Look
        let (p4, g4) = page("Choose a look", "The automatic conversion, three ways. You can adjust everything afterwards.");
        let look_box = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(12).homogeneous(true).build();
        g4.add(&look_box);
        // 5. Scan
        let (p5, g5) = page("Scan", "The frames are scanned at full resolution and exported when each one is done.");
        let scan_btn = gtk::Button::builder().label("Scan").halign(gtk::Align::Start).css_classes(["suggested-action", "pill"]).build();
        let stop_btn = gtk::Button::builder().label("Stop").halign(gtk::Align::Start).build();
        let scan_status = gtk::Label::builder().wrap(true).xalign(0.0).css_classes(["dim-label"]).margin_top(8).build();
        let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.append(&scan_btn);
        row.append(&stop_btn);
        b.append(&row);
        b.append(&scan_status);
        g5.add(&b);
        // 6. Done
        let (p6, g6) = page("Done", "");
        let done_label = gtk::Label::builder().wrap(true).xalign(0.0).build();
        let open_btn = gtk::Button::builder().label("Open Folder").halign(gtk::Align::Start).build();
        let again_btn = gtk::Button::builder().label("Scan Another Roll").halign(gtk::Align::Start).build();
        let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
        b.append(&done_label);
        b.append(&open_btn);
        b.append(&again_btn);
        g6.add(&b);

        let stack = gtk::Stack::builder().vexpand(true).transition_type(gtk::StackTransitionType::SlideLeftRight).build();
        for (i, p) in [&p1, &p2, &p3, &p4, &p5, &p6].into_iter().enumerate() {
            stack.add_named(p, Some(&i.to_string()));
        }
        let back = gtk::Button::builder().label("Back").build();
        let next = gtk::Button::builder().label("Next").css_classes(["suggested-action"]).build();
        let title = gtk::Label::builder().css_classes(["heading"]).build();
        let header = adw::HeaderBar::builder().show_end_title_buttons(true).show_start_title_buttons(false).title_widget(&title).build();
        header.pack_start(&back);
        header.pack_end(&next);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&header);
        content.append(&stack);
        let dialog = adw::Window::builder().transient_for(self).modal(false).title("Scan a Roll").default_width(620).default_height(560).content(&content).build();

        let g = Rc::new(Guide { stack, step: Cell::new(0), back, next, title, type_row, stock_row, stocks: RefCell::new(Vec::new()), frames_row, purpose_row, preview_status, look_box, look_choice: Cell::new(None), scan_status, done_label });
        let fill_stocks = {
            let g = g.clone();
            Rc::new(move || {
                let t = TYPES[(g.type_row.selected() as usize).min(3)].1;
                let list = stocks_for(t);
                let labels: Vec<String> = list.iter().map(|p| if p.vendor == "Generic" { format!("Generic {}", p.name) } else { format!("{} {}", p.vendor, p.name) }).collect();
                g.stock_row.set_model(Some(&gtk::StringList::new(&labels.iter().map(String::as_str).collect::<Vec<_>>())));
                *g.stocks.borrow_mut() = list;
            })
        };
        fill_stocks();
        {
            let f = fill_stocks.clone();
            g.type_row.connect_selected_notify(move |_| f());
        }
        {
            let weak = self.downgrade();
            start_row.connect_active_notify(move |r| {
                if let Some(win) = weak.upgrade() {
                    let mut p = win.imp().prefs.borrow_mut();
                    p.guided_on_start = r.is_active();
                    let _ = p.save();
                }
            });
        }

        // Navigation.
        let show = {
            let (g, weak) = (g.clone(), self.downgrade());
            Rc::new(move |step: usize| {
                g.step.set(step);
                g.stack.set_visible_child_name(&step.to_string());
                g.title.set_label(&format!("{} of {} — {}", step + 1, STEPS.len(), STEPS[step]));
                g.back.set_sensitive(step > 0 && step < 5);
                g.next.set_visible(step < 5);
                g.next.set_label(if step == 4 { "Finish" } else { "Next" });
                if let Some(win) = weak.upgrade() {
                    win.guide_refresh(&g);
                }
            })
        };
        {
            let (g, show, weak) = (g.clone(), show.clone(), self.downgrade());
            g.clone().next.connect_clicked(move |_| {
                let Some(win) = weak.upgrade() else { return };
                let step = g.step.get();
                if win.guide_leave(&g, step) {
                    show(step + 1);
                }
            });
        }
        {
            let (g, show) = (g.clone(), show.clone());
            g.clone().back.connect_clicked(move |_| show(g.step.get().saturating_sub(1)));
        }
        {
            let weak = self.downgrade();
            preview_btn.connect_clicked(move |_| {
                if let Some(win) = weak.upgrade() {
                    if win.imp().busy.get() || !win.device_ready() {
                        win.show_toast("The scanner is not ready");
                    } else {
                        win.start_preview();
                    }
                }
            });
        }
        {
            let weak = self.downgrade();
            preview_stop.connect_clicked(move |_| {
                if let Some(win) = weak.upgrade() {
                    win.stop();
                }
            });
        }
        {
            let weak = self.downgrade();
            scan_btn.connect_clicked(move |_| {
                if let Some(win) = weak.upgrade() {
                    gio::prelude::ActionGroupExt::activate_action(&win, "scan-all", None);
                }
            });
            let weak = self.downgrade();
            stop_btn.connect_clicked(move |_| {
                if let Some(win) = weak.upgrade() {
                    win.stop();
                }
            });
        }
        {
            let weak = self.downgrade();
            open_btn.connect_clicked(move |_| {
                if let Some(win) = weak.upgrade() {
                    let dir = win.export_folder(&win.export_settings());
                    gio::prelude::ActionGroupExt::activate_action(&win, "show-in-files", Some(&dir.display().to_string().to_variant()));
                }
            });
            let show = show.clone();
            again_btn.connect_clicked(move |_| show(0));
        }

        // The status follows what the window is doing.
        {
            let (g, weak) = (g.clone(), self.downgrade());
            glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
                let Some(win) = weak.upgrade() else { return glib::ControlFlow::Break };
                if win.imp().guide.borrow().is_none() {
                    return glib::ControlFlow::Break;
                }
                win.guide_refresh(&g);
                glib::ControlFlow::Continue
            });
        }
        let weak = self.downgrade();
        dialog.connect_close_request(move |_| {
            if let Some(win) = weak.upgrade() {
                *win.imp().guide.borrow_mut() = None;
            }
            glib::Propagation::Proceed
        });
        *self.imp().guide.borrow_mut() = Some(dialog.clone());
        show(0);
        dialog.present();
    }

    /// Do what leaving `step` means. False keeps the person on the step.
    fn guide_leave(&self, g: &Rc<Guide>, step: usize) -> bool {
        let imp = self.imp();
        match step {
            0 => {
                let t = TYPES[(g.type_row.selected() as usize).min(3)].1;
                let Some(profile) = g.stocks.borrow().get(g.stock_row.selected() as usize).cloned() else { return false };
                if imp.roll_root.borrow().is_none() && self.ensure_roll_and_frame(false).is_none() {
                    self.show_toast("Could not create a roll");
                    return false;
                }
                if let Some(roll) = imp.roll.borrow_mut().as_mut() {
                    roll.settings.process.merge(&Settings { image_type: Some(t), profile: Some(profile.id.clone()), ..Default::default() });
                    roll.film_stock = (profile.vendor != "Generic").then(|| format!("{} {}", profile.vendor, profile.name));
                }
                self.schedule_autosave();
                let sel = *imp.selected_frame.borrow();
                self.load_params_for(sel);
                true
            }
            1 => {
                if let Some(p) = Purpose::ALL.get(g.purpose_row.selected() as usize).copied() {
                    if let Some(roll) = imp.roll.borrow_mut().as_mut() {
                        p.apply(&mut roll.settings.export);
                        roll.settings.export.after_scan = true;
                    }
                    let dpi = guide_fit(self, p.dpi());
                    self.set_scan_dpi(dpi);
                }
                true
            }
            2 => {
                if !imp.preview_ready.get() {
                    self.show_toast("Preview the film first");
                    return false;
                }
                true
            }
            3 => true,
            4 => {
                if self.guide_scanned() == 0 {
                    self.show_toast("Scan at least one frame first");
                    return false;
                }
                true
            }
            _ => true,
        }
    }

    fn guide_scanned(&self) -> usize {
        self.imp().roll.borrow().as_ref().map_or(0, |r| r.frames.iter().filter(|f| f.raw.is_some()).count())
    }

    /// Bring the current step's text and buttons up to date.
    fn guide_refresh(&self, g: &Rc<Guide>) {
        let imp = self.imp();
        match g.step.get() {
            2 => {
                let frames = imp.roll.borrow().as_ref().map_or(0, |r| r.frames.iter().filter(|f| f.preview.is_some()).count());
                let text = if !self.device_ready() {
                    "No scanner is ready. Connect and switch on the scanner.".to_owned()
                } else if imp.busy.get() {
                    format!("Previewing…{}", self.stuck_hint())
                } else if imp.preview_ready.get() {
                    format!("Preview ready. {frames} frame(s) found. Check the frame rectangles on the picture, then press Next.")
                } else {
                    "Press Preview Film.".to_owned()
                };
                g.preview_status.set_label(&text);
                g.next.set_sensitive(imp.preview_ready.get() && !imp.busy.get());
            }
            3 => {
                if g.look_box.first_child().is_none() {
                    self.guide_fill_looks(g);
                }
                g.next.set_sensitive(true);
            }
            4 => {
                let (scanned, total) = imp.roll.borrow().as_ref().map_or((0, 0), |r| (r.frames.iter().filter(|f| f.raw.is_some()).count(), r.frames.iter().filter(|f| f.preview.is_some()).count()));
                let manual = imp.manual_advance.get();
                // A one-frame window sees one frame at a time: the count comes from step 1.
                let total = if manual { total.max(g.frames_row.value() as usize) } else { total };
                g.scan_status.set_label(&format!(
                    "{scanned} of {total} frame(s) scanned.{}{}",
                    if imp.busy.get() { " Scanning…" } else { "" },
                    if manual { "\nThis scanner takes one frame at a time: after each scan, move the film to the next frame and press Scan Next Frame in the banner." } else { "" }
                ));
                if imp.busy.get() {
                    let hint = self.stuck_hint();
                    if !hint.is_empty() {
                        g.scan_status.set_label(&format!("{}{hint}", g.scan_status.label()));
                    }
                }
                g.next.set_sensitive(scanned > 0 && !imp.busy.get());
            }
            5 => {
                let dir = self.export_folder(&self.export_settings());
                g.done_label.set_label(&format!("{} frame(s) scanned. The raw scans are in the roll, and the exported pictures are in\n{}", self.guide_scanned(), dir.display()));
            }
            _ => {
                g.next.set_sensitive(true);
            }
        }
    }

    /// What to say when the scanner has gone quiet in the middle of a job.
    fn stuck_hint(&self) -> &'static str {
        let silent = self.imp().last_scan_event.get().is_some_and(|t| t.elapsed() > std::time::Duration::from_secs(30));
        if silent {
            "\nThe scanner has not answered for 30 seconds. Press Stop, switch the scanner off and on again, and try again."
        } else {
            ""
        }
    }

    /// Three thumbnails of the picture in the three looks; clicking one applies it to the roll.
    fn guide_fill_looks(&self, g: &Rc<Guide>) {
        let imp = self.imp();
        let (Some(proxy), Some(sel)) = (imp.proxy.borrow().clone(), *imp.selected_frame.borrow()) else { return };
        let small = reduced(&proxy, 240);
        let mut group: Option<gtk::ToggleButton> = None;
        for look in Look::ALL {
            let mut params = self.effective_params(sel);
            look.settings().apply_to(&mut params);
            if params.conversion.film_base.is_none() {
                params.conversion.film_base = Some(nv_process::estimate_film_base_proxy(&proxy));
            }
            let rgb = nv_process::render(&small, &params);
            let rgba: Vec<u8> = rgb.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect();
            let texture = gdk::MemoryTexture::new(small.width as i32, small.height as i32, gdk::MemoryFormat::R8g8b8a8, &glib::Bytes::from_owned(rgba), small.width * 4);
            let pic = gtk::Picture::for_paintable(&texture);
            pic.set_content_fit(gtk::ContentFit::Contain);
            pic.set_size_request(160, 110);
            let label = gtk::Label::new(Some(look.name()));
            let inner = gtk::Box::new(gtk::Orientation::Vertical, 6);
            inner.append(&pic);
            inner.append(&label);
            let b = gtk::ToggleButton::builder().child(&inner).active(look == Look::Neutral).build();
            match &group {
                Some(f) => b.set_group(Some(f)),
                None => group = Some(b.clone()),
            }
            let (weak, g2) = (self.downgrade(), g.clone());
            b.connect_toggled(move |b| {
                if !b.is_active() {
                    return;
                }
                let Some(win) = weak.upgrade() else { return };
                g2.look_choice.set(Some(look));
                if let Some(roll) = win.imp().roll.borrow_mut().as_mut() {
                    roll.settings.process.merge(&look.settings());
                }
                win.schedule_autosave();
                let sel = *win.imp().selected_frame.borrow();
                win.load_params_for(sel);
            });
            g.look_box.append(&b);
        }
    }
}

fn guide_fit(win: &Window, want: u32) -> u32 {
    fit_dpi(&win.resolutions(), want)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_resolution_is_the_nearest_the_scanner_offers_without_going_over() {
        let list = [300, 600, 1200, 2400, 3600, 7200];
        assert_eq!(fit_dpi(&list, 3600), 3600);
        assert_eq!(fit_dpi(&list, 3000), 2400);
        assert_eq!(fit_dpi(&[1200, 2400], 3600), 2400);
        assert_eq!(fit_dpi(&[1200, 2400], 100), 1200);
        assert_eq!(fit_dpi(&[], 2400), 2400);
    }

    #[test]
    fn every_image_type_has_stocks_and_generic_comes_first() {
        for (_, t) in TYPES {
            let list = stocks_for(t);
            assert!(!list.is_empty(), "{t:?}");
            assert_eq!(list[0].vendor, "Generic", "{t:?}");
        }
        assert!(stocks_for(ImageType::ColourNegative).iter().all(|p| p.kind == FilmKind::ColourNegative));
    }

    #[test]
    fn the_reduced_proxy_keeps_the_picture_and_its_scale() {
        let (w, h) = (1000usize, 600usize);
        let px: Vec<f32> = (0..w * h * 3).map(|i| (i % 7) as f32 / 7.0).collect();
        let p = Proxy::new(w, h, 3, px);
        let r = reduced(&p, 240);
        assert!(r.width <= 240 && r.height <= 240 && r.width > 100);
        assert_eq!(r.factor, 3 * 5);
        assert_eq!(r.pixels.len(), r.width * r.height * 3);
    }
}
