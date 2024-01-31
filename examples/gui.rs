#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

use eframe::egui;
use eframe::egui::util::hash;
use eframe::egui::{Align, Color32, Image, ImageSource, Layout};

use currently_playing::listener::{MediaSource, MediaSourceConfig};
use currently_playing::ws::WebsocketMediaSourceBackground;

fn main() -> Result<(), eframe::Error> {
  env_logger::init(); // Log to stderr (if you run with `RUST_LOG=debug`).

  let options = eframe::NativeOptions {
    viewport: egui::ViewportBuilder::default()
      .with_active(true)
      .with_inner_size([1920.0, 1080.0]),
    ..Default::default()
  };

  eframe::run_native(
    "My egui App",
    options,
    Box::new(|cc| {
      let mut style = cc.egui_ctx.style().as_ref().clone();

      style.text_styles.iter_mut().for_each(|(_, id)| {
        id.size *= 2.0;
      });

      cc.egui_ctx.set_style(style);

      // This gives us image support:
      egui_extras::install_image_loaders(&cc.egui_ctx);

      Box::new(MyApp {
        listener: WebsocketMediaSourceBackground::create(MediaSourceConfig::default()).unwrap(),
      })
    }),
  )
}

struct MyApp {
  listener: WebsocketMediaSourceBackground,
}

impl eframe::App for MyApp {
  fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
    let metadata = self.listener.poll();

    egui::CentralPanel::default().show(ctx, |ui| {
      let Ok(metadata) = metadata else {
        let err = metadata.unwrap_err();
        ui.label(format!("{err:#?}"));
        return;
      };

      let c = Color32::from_gray(254);
      ui.colored_label(c, format!("Title: {}", metadata.title));
      ui.colored_label(c, format!("State: {:?}", metadata.state));
      ui.colored_label(c, format!("Length: {:?}", metadata.duration));
      ui.colored_label(c, format!("Elapsed: {:?}", metadata.elapsed));
      ui.colored_label(c, format!("Artist: {:?}", metadata.artists));
      ui.colored_label(c, format!("Album: {:?}", metadata.album));
      ui.colored_label(c, format!("Cover: {:?}", metadata.cover));
      ui.colored_label(c, format!("Cover (URL): {:?}", metadata.cover_url));
      ui.colored_label(c, format!("Background: {:?}", metadata.background));
      #[rustfmt::skip]
      ui.colored_label(c,format!("Background (URL): {:?}", metadata.background_url));

      ui.with_layout(Layout::left_to_right(Align::LEFT), |ui| {
        if let Some(cover) = metadata.cover {
          let source = ImageSource::Bytes {
            uri: format!("bytes://{}.png", hash(&cover)).into(),
            bytes: cover.data.into(),
          };

          let image = Image::new(source);

          ui.add_sized([512., 512.], image);
        }

        if let Some(cover) = metadata.cover_url {
          let source = ImageSource::Uri(cover.into());
          let image = Image::new(source);

          ui.add_sized([512., 512.], image);
        }

        if let Some(background) = &metadata.background_url {
          let source = ImageSource::Uri(background.into());
          let image = Image::new(source);

          let size = [ui.available_width(), 512.].into();

          if let Some(image_size) = image.load_and_calc_size(ui, size) {
            ui.add_sized(image_size, image);
          }
        }
      });
    });

    ctx.request_repaint();
  }
}
