mod application;
mod canvas_geom;
mod config;
mod window;

use application::Application;

fn main() -> glib::ExitCode {
    Application::run()
}
