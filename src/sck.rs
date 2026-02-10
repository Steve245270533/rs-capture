use std::sync::{Arc, Mutex as StdMutex};

use napi::bindgen_prelude::*;
use napi_derive::napi;

#[cfg(target_os = "macos")]
use crate::backend::macos::SCKBackend;
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

    // Default to XCap unless configured otherwise, or if SCK is requested but not on macOS
    let mut use_sck = false;
    let mut fps = 60;

    if let Some(cfg) = &config {
      if let Some(CaptureBackend::ScreenCaptureKit) = &cfg.backend {
        use_sck = true;
      }
      if let Some(f) = cfg.fps {
        fps = f;
      }
    }

    // Force XCap if not on MacOS even if requested SCK
    #[cfg(not(target_os = "macos"))]
    {
      if use_sck {
        use_sck = false;
      }
    }

    let backend: Box<dyn CaptureBackendImpl> = if use_sck {
      #[cfg(target_os = "macos")]
      {
        Box::new(SCKBackend::new())
      }
      #[cfg(not(target_os = "macos"))]
      {
        // This branch is unreachable due to logic above
        Box::new(XCapBackend::new())
      }
    } else {
      Box::new(XCapBackend::new())
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
