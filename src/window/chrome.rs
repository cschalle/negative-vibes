//! The parts of the window every GNOME app has: the primary menu's About and
//! Keyboard Shortcuts, the sidebar and open-roll actions behind them, and the
//! accessible names Orca reads for buttons that show only an icon.

use adw::prelude::*;
use adw::subclass::prelude::ObjectSubclassIsExt;

use super::Window;

/// Shortcut groups for the Keyboard Shortcuts window: `(group, [(accelerator, what it does)])`.
const SHORTCUTS: [(&str, &[(&str, &str)]); 5] = [
    (
        "Rolls and Scanning",
        &[
            ("<Primary>o", "Open a roll"),
            ("<Primary>r", "Preview the film"),
            ("<Primary>Return", "Scan the selected frame"),
            ("<Primary><Shift>Return", "Scan all frames"),
            ("<Primary><Shift>f", "Detect frames"),
        ],
    ),
    (
        "Frames",
        &[
            ("Page_Down", "Next frame"),
            ("Page_Up", "Previous frame"),
            ("<Primary>d", "Duplicate the frame"),
            ("Delete", "Delete the frame"),
            ("<Primary>Left", "Rotate left"),
            ("<Primary>Right", "Rotate right"),
        ],
    ),
    ("Editing", &[("<Primary>z", "Undo"), ("<Primary>y", "Redo"), ("<Primary>i", "Metadata"), ("<Primary>e", "Export")]),
    (
        "View",
        &[
            ("<Primary>plus", "Zoom in"),
            ("<Primary>minus", "Zoom out"),
            ("<Primary>0", "Fit to window"),
            ("<Primary>1", "Actual pixels"),
            ("F9", "Show or hide the frames"),
            ("<Shift>F9", "Show or hide the inspector"),
        ],
    ),
    ("General", &[("F1", "Help"), ("<Primary>comma", "Preferences"), ("<Primary>question", "Keyboard shortcuts"), ("<Primary>q", "Quit")]),
];

/// The names a button has for a screen reader: its text when it has some, else
/// what its tooltip says (without the shortcut in brackets).
fn accessible_name(w: &gtk::Widget) -> Option<String> {
    let has_text =
        |b: &gtk::Button| b.label().is_some_and(|l| !l.is_empty()) || b.child().is_some_and(|c| c.is::<gtk::Label>() || c.first_child().is_some_and(|k| k.is::<gtk::Label>()));
    let icon_only = if let Some(b) = w.downcast_ref::<gtk::Button>() {
        !has_text(b)
    } else if let Some(b) = w.downcast_ref::<gtk::MenuButton>() {
        b.label().is_none_or(|l| l.is_empty()) && b.child().is_none()
    } else if let Some(b) = w.downcast_ref::<gtk::ToggleButton>() {
        !has_text(b.upcast_ref())
    } else {
        false
    };
    if !icon_only {
        return None;
    }
    let tip = w.tooltip_text()?;
    Some(tip.split(" (").next().unwrap_or(&tip).trim().to_owned()).filter(|s| !s.is_empty())
}

/// Give every button below `root` that shows only an icon the name of its tooltip.
pub fn label_icon_buttons(root: &gtk::Widget) {
    if let Some(name) = accessible_name(root) {
        root.update_property(&[gtk::accessible::Property::Label(&name)]);
    }
    let mut c = root.first_child();
    while let Some(child) = c {
        label_icon_buttons(&child);
        c = child.next_sibling();
    }
}

/// Let a row of buttons with text give up some of it (with an ellipsis) when the
/// window is narrow, instead of setting the least width of the whole window.
pub fn let_buttons_shrink(row: &gtk::Box) {
    let mut c = row.first_child();
    while let Some(child) = c {
        if let Some(label) = child.downcast_ref::<gtk::Button>().and_then(|b| b.child()).and_then(|w| w.downcast::<gtk::Label>().ok()) {
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            label.set_width_chars(1);
        }
        c = child.next_sibling();
    }
}

/// Development aid (`NV_DEBUG_MINWIDTH=<pixels>`): print every widget whose least
/// width is above that, with the widget it is in, to find what keeps a window from narrowing.
pub fn print_wide_widgets(root: &gtk::Widget, limit: i32) {
    fn walk(w: &gtk::Widget, limit: i32, trail: &mut Vec<String>) {
        let (min, _, _, _) = w.measure(gtk::Orientation::Horizontal, -1);
        trail.push(w.type_().name().to_owned());
        let mut wide_child = false;
        let mut c = w.first_child();
        while let Some(child) = c {
            if child.is_visible() && child.measure(gtk::Orientation::Horizontal, -1).0 > limit {
                wide_child = true;
                walk(&child, limit, trail);
            }
            c = child.next_sibling();
        }
        // The innermost widgets still over the limit are the ones to look at.
        if min > limit && !wide_child {
            eprintln!("min {min:4}: {}", trail.join(" > "));
        }
        trail.pop();
    }
    walk(root, limit, &mut Vec::new());
}

/// Development aid: the least widths of the window's main parts, by name.
pub fn print_min_widths(win: &super::Window) {
    let imp = win.imp();
    let min = |w: &gtk::Widget| w.measure(gtk::Orientation::Horizontal, -1).0;
    let scale = gtk::Settings::default().map_or(1.0, |s| s.gtk_xft_dpi() as f64 / 1024.0 / 96.0);
    eprintln!("text scale {scale:.2}");
    for (name, w) in [("outer split", imp.outer_split.borrow().clone().map(|w| w.upcast::<gtk::Widget>())), ("inner split", imp.inner_split.borrow().clone().map(|w| w.upcast()))] {
        if let Some(w) = w {
            let sp = w.downcast_ref::<adw::OverlaySplitView>().unwrap();
            eprintln!(
                "{name}: min {} (collapsed {}, min sidebar {}, max sidebar {}, fraction {})",
                min(&w),
                sp.is_collapsed(),
                sp.min_sidebar_width(),
                sp.max_sidebar_width(),
                sp.sidebar_width_fraction()
            );
            if let Some(c) = sp.content() {
                eprintln!("   content min {}", min(&c));
            }
            if let Some(c) = sp.sidebar() {
                eprintln!("   sidebar min {}", min(&c));
                if min(&c) > 300 {
                    print_wide_widgets(&c, 300);
                }
            }
        }
    }
}

/// Present a dialog whose icon-only buttons have been given names for screen readers.
pub fn present_labelled(w: &impl IsA<gtk::Window>) {
    let window: &gtk::Window = w.upcast_ref();
    label_icon_buttons(window.upcast_ref::<gtk::Widget>());
    // Escape closes a dialog, as it does in every GNOME dialog.
    let keys = gtk::EventControllerKey::new();
    let closing = window.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            closing.close();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    window.add_controller(keys);
    window.present();
}

impl Window {
    pub(super) fn setup_chrome(&self, sidebar_btn: &gtk::ToggleButton, inspector_btn: &gtk::ToggleButton) {
        let simple = |name: &str, f: Box<dyn Fn(&Window)>| {
            let action = gio::SimpleAction::new(name, None);
            let weak = self.downgrade();
            action.connect_activate(move |_, _| {
                if let Some(win) = weak.upgrade() {
                    f(&win);
                }
            });
            self.add_action(&action);
        };
        simple("open-roll", Box::new(|w| w.open_roll_dialog()));
        simple(
            "preview-film",
            Box::new(|w| {
                if let Some(b) = w.imp().preview_btn.borrow().as_ref()
                    && b.is_sensitive()
                {
                    b.emit_clicked();
                }
            }),
        );
        let (sb, ib) = (sidebar_btn.clone(), inspector_btn.clone());
        simple("toggle-frames", Box::new(move |_| sb.set_active(!sb.is_active())));
        simple("toggle-inspector", Box::new(move |_| ib.set_active(!ib.is_active())));
        simple("help", Box::new(|w| w.show_help()));
        simple("about", Box::new(|w| w.show_about()));
        simple("shortcuts", Box::new(|w| w.show_shortcuts()));
    }

    /// Names for what is drawn rather than built from widgets, which a screen
    /// reader would otherwise find nameless, and for the icon-only buttons of the window.
    pub fn label_for_screen_readers(&self) {
        let imp = self.imp();
        let name = |w: Option<gtk::Widget>, text: &str| {
            if let Some(w) = w {
                w.update_property(&[gtk::accessible::Property::Label(text)]);
            }
        };
        name(imp.canvas_picture.borrow().clone().map(|w| w.upcast()), "The picture of the film frame");
        name(imp.histogram_area.borrow().clone().map(|w| w.upcast()), "Histogram of the picture");
        name(imp.levels_area.borrow().clone().map(|w| w.upcast()), "Levels: black, grey and white points");
        name(imp.curve_area.borrow().clone().map(|w| w.upcast()), "Tone curve");
        name(imp.wheel_area.borrow().clone().map(|w| w.upcast()), "Colour balance wheel");
        name(imp.frame_overlay.borrow().clone().map(|w| w.upcast()), "Frame rectangle and brush strokes");
        name(imp.navigator_area.borrow().clone().map(|w| w.upcast()), "Map of the whole picture");
        label_icon_buttons(self.upcast_ref());
    }

    /// Something that matters failed (a scan, a roll that could not be saved): a toast
    /// would be gone before it was read, so this waits for an answer.
    pub fn show_error(&self, heading: &str, detail: &str) {
        let dialog = adw::MessageDialog::builder().transient_for(self).modal(true).heading(heading).body(glib::markup_escape_text(detail).as_str()).build();
        dialog.add_response("ok", "OK");
        dialog.set_default_response(Some("ok"));
        dialog.set_close_response("ok");
        dialog.present();
    }

    /// Open the help in Yelp: the installed manual, or, when run from the source tree, its pages.
    fn show_help(&self) {
        // The manual is installed under <datadir>/help/C/negative-vibes. A Flatpak does not export
        // it to the host's Yelp, so it is opened with the Yelp the GNOME runtime carries.
        let source = std::path::Path::new(crate::config::PKGDATADIR).join("../help/C/negative-vibes/index.page");
        if source.is_file() {
            match std::process::Command::new("yelp").arg(&source).spawn() {
                Ok(_) => return,
                Err(e) => eprintln!("nv: could not start yelp: {e}"),
            }
        }
        let launcher = gtk::UriLauncher::new("help:negative-vibes");
        let weak = self.downgrade();
        launcher.launch(Some(self), None::<&gio::Cancellable>, move |r| {
            if let (Err(_), Some(win)) = (r, weak.upgrade()) {
                win.show_error("The help could not be opened", "Install Yelp, the GNOME help viewer, to read the help.");
            }
        });
    }

    fn show_about(&self) {
        let about = adw::AboutWindow::builder()
            .transient_for(self)
            .modal(true)
            .application_name("Negative Vibes")
            .application_icon(crate::config::APP_ID)
            .developer_name("Christian Schaller")
            .version(env!("CARGO_PKG_VERSION"))
            .comments("Scan colour and black-and-white film negatives and slides.")
            .website("https://github.com/cschalle/negative-vibes")
            .issue_url("https://github.com/cschalle/negative-vibes/issues")
            .license_type(gtk::License::Gpl30)
            .copyright("© 2026 Christian Schaller")
            .build();
        present_labelled(&about);
    }

    fn show_shortcuts(&self) {
        let page = adw::PreferencesPage::new();
        for (group, items) in SHORTCUTS {
            let g = adw::PreferencesGroup::builder().title(group).build();
            for (accel, what) in items {
                let row = adw::ActionRow::builder().title(*what).build();
                row.add_suffix(&gtk::ShortcutLabel::builder().accelerator(*accel).valign(gtk::Align::Center).build());
                g.add(&row);
            }
            page.add(&g);
        }
        let header = adw::HeaderBar::new();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&header);
        content.append(&page);
        let window = adw::Window::builder().transient_for(self).modal(true).title("Keyboard Shortcuts").default_width(480).default_height(640).content(&content).build();
        present_labelled(&window);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every shortcut the shortcuts window lists is one the application has
    /// bound, and the other way round, so it can be trusted.
    #[test]
    fn the_shortcuts_window_lists_what_application_rs_binds() {
        let source = include_str!("../application.rs");
        let bound: Vec<String> =
            source.lines().filter(|l| l.contains("set_accels_for_action")).flat_map(|l| l.split('"').skip(3).step_by(2).map(String::from).collect::<Vec<_>>()).collect();
        for (_, items) in SHORTCUTS {
            for (accel, what) in items {
                let ours = accel.replace("<Primary>", "<Ctrl>");
                assert!(
                    bound.iter().any(|b| b.eq_ignore_ascii_case(&ours)) || *accel == "<Primary>question" && bound.iter().any(|b| b.contains("question")),
                    "{what} ({accel}) is listed but not bound"
                );
            }
        }
    }
}
