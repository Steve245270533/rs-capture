use napi::threadsafe_function::ThreadsafeFunction;
use napi::{sys, Result, Status};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub struct FrameDataInternal {
  pub width: u32,
  pub height: u32,
  pub stride: u32,
  pub data: Vec<u8>,
}

pub type FrameTsfn =
  ThreadsafeFunction<FrameDataInternal, (), sys::napi_value, Status, false, false, 0>;
pub type FrameTsfnType = Arc<FrameTsfn>;

pub trait CaptureBackendImpl: Send + Sync {
  fn start<'a>(
    &'a mut self,
    tsfn: Option<FrameTsfnType>,
    fps: u32,
  ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
  fn stop(&mut self) -> Result<()>;

  fn screenshot<'a>(
    &'a mut self,
  ) -> Pin<Box<dyn Future<Output = Result<FrameDataInternal>> + Send + 'a>>;
}

#[cfg(target_os = "windows")]
pub mod dxgi;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;
pub mod xcap;
