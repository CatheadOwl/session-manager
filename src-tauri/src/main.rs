#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    session_manager_lib::run();
}
