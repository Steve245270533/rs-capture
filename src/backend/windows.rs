use napi::Result;
use std::future::Future;
use std::pin::Pin;

use super::dxgi::DxgiBackend;
use super::xcap::XCapBackend;
use super::{CaptureBackendImpl, FrameDataInternal, FrameTsfnType};

pub struct WindowsBackend {
  inner: Box<dyn CaptureBackendImpl>,
}

unsafe impl Send for WindowsBackend {}
unsafe impl Sync for WindowsBackend {}

impl WindowsBackend {
  pub fn new() -> Self {
    match DxgiBackend::new() {
      Ok(dxgi) => Self {
        inner: Box::new(dxgi),
      },
      Err(e) => {
        eprintln!(
          "Windows capture init failed: {:?}. Falling back to XCap.",
          e
        );
        Self {
          inner: Box::new(XCapBackend::new()),
        }
      }
    }
  }
}

impl Default for WindowsBackend {
  fn default() -> Self {
    Self::new()
  }
}

impl CaptureBackendImpl for WindowsBackend {
  fn start<'a>(
    &'a mut self,
    tsfn: Option<FrameTsfnType>,
    fps: u32,
  ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
    self.inner.start(tsfn, fps)
  }

  fn stop(&mut self) -> Result<()> {
    self.inner.stop()
  }

  fn screenshot<'a>(
    &'a mut self,
  ) -> Pin<Box<dyn Future<Output = Result<FrameDataInternal>> + Send + 'a>> {
    self.inner.screenshot()
  }
}
