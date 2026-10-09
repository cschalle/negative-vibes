mod application;
mod canvas_geom;
mod config;
mod export_queue;
mod google;
mod prefs;
mod window;

use application::Application;

fn main() -> glib::ExitCode {
    Application::run()
}
