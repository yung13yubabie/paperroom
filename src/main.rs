#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(any(windows, test))]
mod archive;
#[cfg(any(windows, test))]
mod board_image_dib;
#[cfg(any(windows, test))]
mod canvas;
#[cfg(any(windows, test))]
mod canvas_projection;
#[cfg(any(windows, test))]
mod formula;
#[cfg(any(windows, test))]
mod markdown;
#[cfg(any(windows, test))]
mod output;
#[cfg(any(windows, test))]
mod store;
#[cfg(any(windows, test))]
mod vault;
#[cfg(any(windows, test))]
mod vault_edit;
#[cfg(any(windows, test))]
mod pdf_geometry;
#[cfg(any(windows, test))]
mod pdf_render;
#[cfg(any(windows, test))]
mod about_notices;
#[cfg(windows)]
mod dialog;
#[cfg(windows)]
mod window;

#[cfg(windows)]
fn main() {
    window::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!(
        "紙間的圖形介面使用 Windows 10 / 11 原生控制項。可在此平台以 cargo test 驗證資料層。"
    );
}
