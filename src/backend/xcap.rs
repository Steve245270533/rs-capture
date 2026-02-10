use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use napi::threadsafe_function::ThreadsafeFunctionCallMode;
use napi::{Result, Status};
use xcap::Monitor;

use super::{CaptureBackendImpl, FrameDataInternal, FrameTsfnType};

pub struct XCapBackend {
  running: Arc<AtomicBool>,
  handle: Option<thread::JoinHandle<()>>,
}

impl XCapBackend {
  pub fn new() -> Self {
    Self {
      running: Arc::new(AtomicBool::new(false)),
      handle: None,
    }
  }
}

impl CaptureBackendImpl for XCapBackend {
  fn start<'a>(
    &'a mut self,
    tsfn: FrameTsfnType,
    fps: u32,
  ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
    Box::pin(async move {
      if self.running.load(Ordering::SeqCst) {
        return Ok(());
      }

      self.running.store(true, Ordering::SeqCst);
      let running = self.running.clone();

      let handle = thread::spawn(move || {
        let monitors = match Monitor::all() {
          Ok(m) => m,
          Err(e) => {
            eprintln!("Failed to get monitors: {}", e);
            return;
          }
        };

        if monitors.is_empty() {
          eprintln!("No monitors found");
          return;
        }

        let monitor = &monitors[0];
        let target_interval = Duration::from_secs_f64(1.0 / fps as f64);

        while running.load(Ordering::SeqCst) {
          let start = Instant::now();
          match monitor.capture_image() {
            Ok(img) => {
              let width = img.width();
              let height = img.height();
              let data = img.into_raw();
              let stride = width * 4;

              let frame = FrameDataInternal {
                width,
                height,
                stride,
                data,
              };

              let status = tsfn.call(frame, ThreadsafeFunctionCallMode::NonBlocking);
              if status != Status::Ok {
                break;
              }
            }
            Err(e) => {
              eprintln!("Capture failed: {}", e);
              thread::sleep(Duration::from_millis(100));
            }
          }

          let elapsed = start.elapsed();
          if elapsed < target_interval {
            thread::sleep(target_interval - elapsed);
          }
        }
      });

      self.handle = Some(handle);
      Ok(())
    })
  }

  fn stop(&mut self) -> Result<()> {
    self.running.store(false, Ordering::SeqCst);
    if let Some(handle) = self.handle.take() {
      let _ = handle.join();
    }
    Ok(())
  }
}
