#![cfg(target_os = "windows")]

use std::pin::Pin;
use std::future::Future;
use napi::Result;

use super::{CaptureBackendImpl, FrameTsfnType};
use super::dxgi::DxgiBackend;
use super::xcap::XCapBackend;

pub struct WindowsBackend {
    inner: Box<dyn CaptureBackendImpl>,
}

unsafe impl Send for WindowsBackend {}
unsafe impl Sync for WindowsBackend {}

impl WindowsBackend {
    pub fn new() -> Self {
        // Try DXGI first
        match DxgiBackend::new() {
            Ok(dxgi) => {
                // println!("Using DXGI Backend");
                Self { inner: Box::new(dxgi) }
            },
            Err(e) => {
                eprintln!("DXGI Init failed: {:?}. Falling back to XCap/GDI.", e);
                Self { inner: Box::new(XCapBackend::new()) }
            }
        }
    }
}

impl CaptureBackendImpl for WindowsBackend {
    fn start<'a>(&'a mut self, tsfn: FrameTsfnType, fps: u32) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        self.inner.start(tsfn, fps)
    }

    fn stop(&mut self) -> Result<()> {
        self.inner.stop()
    }
}
