use std::thread::sleep;
use std::time::Duration;

use currently_playing::platform::MediaListener;

fn main() -> currently_playing::Result<()> {
  let media = MediaListener::new(None);

  loop {
    let metadata = media.poll(true)?;
    let elapsed = media.poll_elapsed(true)?;

    println!("\x1bc");

    println!("Title: {}", metadata.title);
    println!("State: {:?}", metadata.state);
    println!("Length: {:?}", metadata.duration);
    println!("Elapsed: {:?}", elapsed);
    println!("Artist: {:?}", metadata.artists);
    println!("Cover: {:?}", metadata.cover);
    println!("Cover (URL): {:?}", metadata.cover_url);

    sleep(Duration::from_millis(500));
  }
}
