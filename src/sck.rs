use std::sync::{Arc, Mutex as StdMutex};

use napi::bindgen_prelude::*;
use napi_derive::napi;

#[cfg(target_os = "macos")]
use crate::backend::macos::SCKBackend;
#[cfg(target_os = "windows")]
use crate::backend::windows::WindowsBackend;
use crate::backend::xcap::XCapBackend;
use crate::backend::{CaptureBackendImpl, FrameDataInternal, FrameTsfnType};

#[napi(object)]
pub struct FrameData {
  pub width: u32,
  pub height: u32,
  pub stride: u32,
  pub rgba: Buffer,
}

#[napi(string_enum)]
#[derive(Clone, Copy)]
pub enum CaptureBackend {
  ScreenCaptureKit,
  XCap,
}

#[napi(object)]
pub struct ScreenCaptureConfig {
  pub backend: Option<CaptureBackend>, // "ScreenCaptureKit" | "xcap"
  pub fps: Option<u32>,
}

#[napi]
pub struct ScreenCapture {
  backend: Arc<StdMutex<Option<Box<dyn CaptureBackendImpl>>>>,
  tsfn: FrameTsfnType,
  fps: u32,
}

#[napi]
impl ScreenCapture {
  #[napi(
    constructor,
    ts_args_type = "callback: (frame: FrameData) => void, config?: ScreenCaptureConfig | null"
  )]
  pub fn new(callback: Function<'_, (), ()>, config: Option<ScreenCaptureConfig>) -> Result<Self> {
    let tsfn: FrameTsfnType = Arc::new(
      callback
        .build_threadsafe_function::<FrameDataInternal>()
        .build_callback(|ctx| {
          let frame: FrameDataInternal = ctx.value;
          let mut js_obj = Object::new(&ctx.env)?;

          js_obj.set_named_property("width", frame.width)?;
          js_obj.set_named_property("height", frame.height)?;
          js_obj.set_named_property("stride", frame.stride)?;

          let buf = Buffer::from(frame.data);
          js_obj.set_named_property("rgba", buf)?;
          Ok(js_obj.raw())
        })?,
    );

    let mut backend_enum = None;
    let mut fps = 60;

    if let Some(cfg) = &config {
      backend_enum = cfg.backend;
      if let Some(f) = cfg.fps {
        fps = f;
      }
    }

    let backend: Box<dyn CaptureBackendImpl> = match backend_enum {
      Some(CaptureBackend::ScreenCaptureKit) => {
        #[cfg(target_os = "macos")]
        {
          Box::new(SCKBackend::new())
        }
        #[cfg(not(target_os = "macos"))]
        {
          Box::new(XCapBackend::new())
        }
      }
      Some(CaptureBackend::XCap) => Box::new(XCapBackend::new()),
      None => {
        #[cfg(target_os = "macos")]
        {
          Box::new(SCKBackend::new())
        }
        #[cfg(target_os = "windows")]
        {
          Box::new(WindowsBackend::new())
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
          Box::new(XCapBackend::new())
        }
      }
    };

    Ok(ScreenCapture {
      backend: Arc::new(StdMutex::new(Some(backend))),
      tsfn,
      fps,
    })
  }

  #[napi]
  pub async fn start(&self) -> Result<()> {
    let backend_opt = {
      let mut backend_guard = self.backend.lock().unwrap();
      backend_guard.take()
    };

    if let Some(mut backend) = backend_opt {
      let result = backend.start(self.tsfn.clone(), self.fps).await;

      let mut backend_guard = self.backend.lock().unwrap();
      *backend_guard = Some(backend);

      result
    } else {
      Err(Error::new(
        Status::GenericFailure,
        "Backend is missing".to_string(),
      ))
    }
  }

  #[napi]
  pub fn stop(&self) -> Result<()> {
    let mut backend_guard = self.backend.lock().unwrap();
    if let Some(backend) = backend_guard.as_mut() {
      backend.stop()
    } else {
      Ok(())
    }
  }
}
