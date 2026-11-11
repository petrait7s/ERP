mod api;
mod app;
mod crud;
mod fmt;
mod i18n;
mod pages;
mod state;
mod ui;

fn main() {
    console_error_panic_hook::set_once();
    state::init();
    leptos::mount::mount_to_body(app::App);
}
