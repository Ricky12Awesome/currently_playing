use std::time::Duration;
use currently_playing::listener::{MediaSource, MediaSourceConfig};

use currently_playing::platform::MprisMediaSource;

fn main() {
  if cfg!(debug_assertions) {
    panic!("Cannot run in debug mode, since it effects performance")
  }

  let media = MprisMediaSource::create(MediaSourceConfig::default()).unwrap();

  let duration = Duration::from_secs(5);
  let result = benchmarking::bench_function_with_duration(duration, |m| {
    m.measure(|| media.poll_guarded());
  });

  let result = result.unwrap();

  println!(
    "{:?} [{:?}/s] [{:?} in {duration:?}]",
    result.elapsed(),
    result.speed(),
    result.times(),
  );
}
