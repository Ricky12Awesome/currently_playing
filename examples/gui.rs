#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

use eframe::egui;
use eframe::egui::util::hash;
use eframe::egui::{Image, ImageSource};

use currently_playing::platform::MediaListener;

fn main() -> Result<(), eframe::Error> {
  env_logger::init(); // Log to stderr (if you run with `RUST_LOG=debug`).

  let options = eframe::NativeOptions {
    viewport: egui::ViewportBuilder::default()
      .with_active(true)
      .with_inner_size([1600.0, 900.0]),
    ..Default::default()
  };

  eframe::run_native(
    "My egui App",
    options,
    Box::new(|cc| {
      let mut style = cc.egui_ctx.style().as_ref().clone();

      style.text_styles.iter_mut().for_each(|(_, id)| {
        id.size *= 3.0;
      });

      cc.egui_ctx.set_style(style);

      // This gives us image support:
      egui_extras::install_image_loaders(&cc.egui_ctx);

      Box::new(MyApp {
        listener: MediaListener::new(None),
      })
    }),
  )
}

struct MyApp {
  listener: MediaListener,
}

impl eframe::App for MyApp {
  fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
    let metadata = self.listener.poll(true);
    let elapsed = self.listener.poll_elapsed(false);

    egui::CentralPanel::default().show(ctx, |ui| {
      let Ok(metadata) = metadata else {
        let err = metadata.unwrap_err();
        ui.label(format!("{err:#?}"));
        return;
      };

      ui.label(format!("Title: {}", metadata.title));
      ui.label(format!("State: {:?}", metadata.state));
      ui.label(format!("Length: {:?}", metadata.duration));
      ui.label(format!("Elapsed: {:?}", elapsed));
      ui.label(format!("Artist: {:?}", metadata.artists));
      ui.label(format!("Cover: {:?}", metadata.cover));
      ui.label(format!("Cover: {:?}", metadata.cover_url));

      if let Some(cover) = metadata.cover {
        let source = ImageSource::Bytes {
          uri: format!("bytes://{}.png", hash(&cover)).into(),
          bytes: cover.data.into(),
        };

        let image = Image::new(source);

        ui.add_sized([300., 300.], image);
      }


      if let Some(cover) = metadata.cover_url {
        let source = ImageSource::Uri(cover.into());
        let image = Image::new(source);

        ui.add_sized([300., 300.], image);
      }
    });

    ctx.request_repaint();
  }
}
