// SPDX-License-Identifier: GPL-2.0-or-later

//! The supplied PentaX artwork, decoded once and shared by the desktop.

use eframe::egui::{self, ColorImage, IconData, TextureHandle, TextureOptions};
use image::ImageFormat;

const DARK: &[u8] = include_bytes!("../assets/brand/logo-dark.png");
const LIGHT: &[u8] = include_bytes!("../assets/brand/logo-light.png");
const ICON: &[u8] = include_bytes!("../assets/brand/app-icon.png");

pub fn icon() -> Result<IconData, image::ImageError> {
    let image = image::load_from_memory_with_format(ICON, ImageFormat::Png)?
        .resize_exact(256, 256, image::imageops::FilterType::Lanczos3)
        .into_rgba8();
    Ok(IconData {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    })
}

pub fn install(ctx: &egui::Context) -> Result<(), image::ImageError> {
    for (name, bytes) in [("pentax-logo-dark", DARK), ("pentax-logo-light", LIGHT)] {
        let image = image::load_from_memory_with_format(bytes, ImageFormat::Png)?.into_rgba8();
        let pixels = ColorImage::from_rgba_unmultiplied(
            [image.width() as usize, image.height() as usize],
            image.as_raw(),
        );
        let texture = ctx.load_texture(name, pixels, TextureOptions::LINEAR);
        ctx.data_mut(|data| data.insert_temp(egui::Id::unique(name), texture));
    }
    Ok(())
}

pub fn show(ui: &mut egui::Ui, size: f32) {
    let name = if ui.visuals().dark_mode {
        "pentax-logo-dark"
    } else {
        "pentax-logo-light"
    };
    let texture = ui
        .ctx()
        .data(|data| data.get_temp::<TextureHandle>(egui::Id::unique(name)))
        .expect("PentaX artwork is installed during desktop startup");
    ui.add(
        egui::Image::new((texture.id(), texture.size_vec2()))
            .fit_to_exact_size(egui::Vec2::splat(size))
            .alt_text("PentaX PX monogram"),
    );
}
