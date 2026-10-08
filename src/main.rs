mod application;
mod canvas_geom;
mod config;
mod export_queue;
mod window;

use application::Application;

fn main() -> glib::ExitCode {
    Application::run()
}
