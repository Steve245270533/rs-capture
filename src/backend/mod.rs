use std::sync::Arc;
use std::pin::Pin;
use std::future::Future;
use napi::threadsafe_function::ThreadsafeFunction;
use napi::{Status, sys, Result};

pub struct FrameDataInternal {
  pub width: u32,
  pub height: u32,
  pub stride: u32,
  pub data: Vec<u8>,
}

pub type FrameTsfn = ThreadsafeFunction<FrameDataInternal, (), sys::napi_value, Status, false, false, 0>;
pub type FrameTsfnType = Arc<FrameTsfn>;

pub trait CaptureBackendImpl: Send + Sync {
    fn start<'a>(&'a mut self, tsfn: FrameTsfnType, fps: u32) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
    fn stop(&mut self) -> Result<()>;
}

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod dxgi;
#[cfg(target_os = "windows")]
pub mod windows;
pub mod xcap;
