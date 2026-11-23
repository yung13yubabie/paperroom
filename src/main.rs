#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(any(windows, test))]
mod archive;
#[cfg(any(windows, test))]
mod markdown;
#[cfg(any(windows, test))]
mod output;
#[cfg(any(windows, test))]
mod store;
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
