// SPDX-License-Identifier: GPL-2.0-or-later
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod brand;
mod controller;
mod library;
mod profile;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("PentaX")
            .with_icon(brand::icon().map_err(|error| eframe::Error::AppCreation(Box::new(error)))?)
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([800.0, 600.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "PentaX",
        options,
        Box::new(|cc| {
            brand::install(&cc.egui_ctx)?;
            Ok(Box::new(app::Desktop::new(cc)))
        }),
    )
}
