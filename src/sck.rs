use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::thread;
use std::time::{Duration, Instant};

use napi::bindgen_prelude::*;
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi_derive::napi;

#[cfg(target_os = "macos")]
use block2::RcBlock;
#[cfg(target_os = "macos")]
use objc2::AnyThread;
#[cfg(target_os = "macos")]
use objc2::{
  define_class, msg_send,
  rc::{Allocated, Retained},
  ClassType, DeclaredClass,
};
#[cfg(target_os = "macos")]
use objc2_core_media::{CMSampleBuffer, CMTime, CMTimeFlags};
#[cfg(target_os = "macos")]
use objc2_foundation::{NSArray, NSError, NSObject, NSObjectProtocol};
#[cfg(target_os = "macos")]
use objc2_screen_capture_kit::*;

use xcap::Monitor;

#[cfg(target_os = "macos")]
#[link(name = "CoreMedia", kind = "framework")]
extern "C" {
  fn CMSampleBufferGetImageBuffer(sbuf: *mut c_void) -> *mut c_void;
}

#[cfg(target_os = "macos")]
#[link(name = "CoreVideo", kind = "framework")]
extern "C" {
  fn CVPixelBufferGetBaseAddress(pbuf: *mut c_void) -> *mut c_void;
  fn CVPixelBufferGetBytesPerRow(pbuf: *mut c_void) -> usize;
  fn CVPixelBufferGetWidth(pbuf: *mut c_void) -> usize;
  fn CVPixelBufferGetHeight(pbuf: *mut c_void) -> usize;
  fn CVPixelBufferLockBaseAddress(pbuf: *mut c_void, flags: u64) -> i32;
  fn CVPixelBufferUnlockBaseAddress(pbuf: *mut c_void, flags: u64) -> i32;
}

pub struct FrameDataInternal {
  pub width: u32,
  pub height: u32,
  pub stride: u32,
  pub data: Vec<u8>,
}

#[napi(object)]
pub struct FrameData {
  pub width: u32,
  pub height: u32,
  pub stride: u32,
  pub rgba: Buffer,
}

type FrameTsfn =
  ThreadsafeFunction<FrameDataInternal, (), sys::napi_value, Status, false, false, 0>;
type FrameTsfnType = Arc<FrameTsfn>;

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

// -----------------------------------------------------------------------------
// SCK Implementation (MacOS)
// -----------------------------------------------------------------------------

#[cfg(target_os = "macos")]
pub struct StreamDelegateIvars {
  tsfn_ptr: usize,
}

#[cfg(target_os = "macos")]
impl Drop for StreamDelegateIvars {
  fn drop(&mut self) {
    if self.tsfn_ptr != 0 {
      unsafe { drop(Box::from_raw(self.tsfn_ptr as *mut FrameTsfnType)) };
    }
  }
}

#[cfg(target_os = "macos")]
define_class!(
    #[unsafe(super(NSObject))]
    #[name = "StreamDelegate"]
    #[ivars = StreamDelegateIvars]
    pub struct StreamDelegate;

    impl StreamDelegate {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        fn did_output(&self, _stream: &SCStream, sample: &CMSampleBuffer, kind: SCStreamOutputType) {
            if kind == SCStreamOutputType::Screen {
                 let ptr = self.ivars().tsfn_ptr;
                 if ptr != 0 {
                     let tsfn = unsafe { &*(ptr as *const FrameTsfnType) };

                     unsafe {
                         let sbuf_ptr = sample as *const CMSampleBuffer as *mut c_void;
                         let pixel_buffer = CMSampleBufferGetImageBuffer(sbuf_ptr);
                         if !pixel_buffer.is_null() {
                             CVPixelBufferLockBaseAddress(pixel_buffer, 1); // ReadOnly
                             let width = CVPixelBufferGetWidth(pixel_buffer);
                             let height = CVPixelBufferGetHeight(pixel_buffer);
                             let stride = CVPixelBufferGetBytesPerRow(pixel_buffer);
                             let base = CVPixelBufferGetBaseAddress(pixel_buffer);

                             if !base.is_null() {
                                 let base_ptr = base as *const u8;
                                 let mut data = Vec::with_capacity(width * height * 4);

                                 // Compact and Swap RB (BGRA -> RGBA)
                                 for y in 0..height {
                                     let row_start = base_ptr.add(y * stride);
                                     let row_slice = std::slice::from_raw_parts(row_start, width * 4);

                                     for chunk in row_slice.chunks_exact(4) {
                                         data.push(chunk[2]); // R
                                         data.push(chunk[1]); // G
                                         data.push(chunk[0]); // B
                                         data.push(chunk[3]); // A
                                     }
                                 }

                                 let frame = FrameDataInternal { width: width as u32, height: height as u32, stride: (width * 4) as u32, data };
                                 tsfn.call(frame, ThreadsafeFunctionCallMode::NonBlocking);
                             }
                             CVPixelBufferUnlockBaseAddress(pixel_buffer, 1);
                         }
                     }
                 }
            }
        }
    }
);

#[cfg(target_os = "macos")]
unsafe impl SCStreamOutput for StreamDelegate {}
#[cfg(target_os = "macos")]
unsafe impl NSObjectProtocol for StreamDelegate {}
#[cfg(target_os = "macos")]
unsafe impl Send for StreamDelegate {}
#[cfg(target_os = "macos")]
unsafe impl Sync for StreamDelegate {}

#[cfg(target_os = "macos")]
impl StreamDelegate {
  fn new(tsfn: FrameTsfnType) -> Retained<Self> {
    let boxed = Box::new(tsfn);
    let ptr = Box::into_raw(boxed) as usize;

    let cls = Self::class();
    let obj: Allocated<Self> = unsafe { msg_send![cls, alloc] };
    let obj = obj.set_ivars(StreamDelegateIvars { tsfn_ptr: ptr });
    unsafe { msg_send![super(obj), init] }
  }
}

#[cfg(target_os = "macos")]
struct SendRetained<T>(Retained<T>);
#[cfg(target_os = "macos")]
unsafe impl<T> Send for SendRetained<T> {}

#[cfg(target_os = "macos")]
struct SCKBackend {
  stream: Option<Retained<SCStream>>,
  delegate: Option<Retained<StreamDelegate>>,
}

#[cfg(target_os = "macos")]
unsafe impl Send for SCKBackend {}
#[cfg(target_os = "macos")]
unsafe impl Sync for SCKBackend {}

// -----------------------------------------------------------------------------
// XCap Implementation (Cross-platform)
// -----------------------------------------------------------------------------

struct XCapBackend {
  running: Arc<AtomicBool>,
  handle: Option<thread::JoinHandle<()>>,
}

// -----------------------------------------------------------------------------
// Wrapper
// -----------------------------------------------------------------------------

enum BackendWrapper {
  #[cfg(target_os = "macos")]
  Sck(SCKBackend),
  XCap(XCapBackend),
}

#[napi]
pub struct ScreenCapture {
  backend: Arc<StdMutex<Option<BackendWrapper>>>,
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

    let wrapper = if use_sck {
      #[cfg(target_os = "macos")]
      {
        BackendWrapper::Sck(SCKBackend {
          stream: None,
          delegate: None,
        })
      }
      #[cfg(not(target_os = "macos"))]
      {
        // This branch is unreachable due to logic above
        BackendWrapper::XCap(XCapBackend {
          running: Arc::new(AtomicBool::new(false)),
          handle: None,
        })
      }
    } else {
      BackendWrapper::XCap(XCapBackend {
        running: Arc::new(AtomicBool::new(false)),
        handle: None,
      })
    };

    Ok(ScreenCapture {
      backend: Arc::new(StdMutex::new(Some(wrapper))),
      tsfn,
      fps,
    })
  }

  #[napi]
  pub async fn start(&self) -> Result<()> {
    let mut wrapper = {
      let mut backend_guard = self.backend.lock().unwrap();
      backend_guard
        .take()
        .ok_or_else(|| Error::new(Status::GenericFailure, "Backend is missing".to_string()))?
    };

    let result = match wrapper {
      #[cfg(target_os = "macos")]
      BackendWrapper::Sck(ref mut sck) => Self::start_sck(sck, self.tsfn.clone(), self.fps).await,
      BackendWrapper::XCap(ref mut xcap) => Self::start_xcap(xcap, self.tsfn.clone(), self.fps),
    };

    {
      let mut backend_guard = self.backend.lock().unwrap();
      *backend_guard = Some(wrapper);
    }

    result
  }

  #[napi]
  pub fn stop(&self) -> Result<()> {
    let mut backend_guard = self.backend.lock().unwrap();
    let Some(wrapper) = backend_guard.as_mut() else {
      return Ok(());
    };

    match wrapper {
      #[cfg(target_os = "macos")]
      BackendWrapper::Sck(ref mut sck) => Self::stop_sck(sck),
      BackendWrapper::XCap(ref mut xcap) => Self::stop_xcap(xcap),
    }
  }

  // Helper methods

  #[cfg(target_os = "macos")]
  async fn start_sck(sck: &mut SCKBackend, tsfn: FrameTsfnType, fps: u32) -> Result<()> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    // Use StdMutex for sync callback
    let tx = Arc::new(StdMutex::new(Some(tx)));

    {
      let handler = RcBlock::new(
        move |content: *mut SCShareableContent, error: *mut NSError| {
          let mut tx_guard = tx.lock().unwrap();
          if let Some(tx) = tx_guard.take() {
            if !error.is_null() {
              let _ = tx.send(Err("SCK Error".to_string()));
            } else {
              // Unsafe unwrap assuming content is valid if error is null
              let content = unsafe { Retained::retain(content) }.expect("Content is null");
              let _ = tx.send(Ok(SendRetained(content)));
            }
          }
        },
      );

      unsafe {
        SCShareableContent::getShareableContentWithCompletionHandler(&handler);
      }
    }

    let content_opt = rx
      .await
      .map_err(|e| Error::new(Status::GenericFailure, format!("Await error: {:?}", e)))?;
    let content_res = content_opt.map_err(|e| Error::new(Status::GenericFailure, e))?;

    // Scope to ensure !Send types are dropped before await
    let (stream_wrapper, delegate_wrapper) = {
      let content = content_res.0;
      let displays = unsafe { content.displays() };
      let display = displays
        .firstObject()
        .ok_or_else(|| Error::new(Status::GenericFailure, "No display found".to_string()))?;

      let filter = unsafe {
        SCContentFilter::initWithDisplay_excludingApplications_exceptingWindows(
          SCContentFilter::alloc(),
          &display,
          &NSArray::array(),
          &NSArray::array(),
        )
      };

      let config = unsafe { SCStreamConfiguration::new() };
      unsafe {
        config.setWidth(display.width() as usize);
        config.setHeight(display.height() as usize);
        config.setMinimumFrameInterval(CMTime {
          value: 1,
          timescale: fps as i32,
          flags: CMTimeFlags(1),
          epoch: 0,
        });
        config.setQueueDepth(5);
        config.setPixelFormat(1111970369); // kCVPixelFormatType_32BGRA
      }

      let stream = unsafe {
        SCStream::initWithFilter_configuration_delegate(SCStream::alloc(), &filter, &config, None)
      };

      let delegate = StreamDelegate::new(tsfn);

      let queue = unsafe { dispatch_queue_create(c"com.napi.sck".as_ptr(), ptr::null_mut()) };

      unsafe {
        let _: bool = msg_send![&stream, addStreamOutput: &*delegate, type: SCStreamOutputType::Screen, sampleHandlerQueue: queue as *mut NSObject, error: ptr::null_mut::<*mut NSError>()];
      }

      {
        let start_handler = RcBlock::new(move |error: *mut NSError| {
          if !error.is_null() {
            // println!("SCK Start failed with error");
          }
        });

        unsafe {
          stream.startCaptureWithCompletionHandler(Some(&*start_handler));
        }
      }

      (SendRetained(stream), SendRetained(delegate))
    };

    sck.stream = Some(stream_wrapper.0);
    sck.delegate = Some(delegate_wrapper.0);

    Ok(())
  }

  #[cfg(target_os = "macos")]
  fn stop_sck(sck: &mut SCKBackend) -> Result<()> {
    if let Some(stream) = sck.stream.take() {
      let handler = RcBlock::new(move |_error: *mut NSError| {});
      unsafe { stream.stopCaptureWithCompletionHandler(Some(&*handler)) };
    }
    sck.delegate = None;
    Ok(())
  }

  fn start_xcap(xcap: &mut XCapBackend, tsfn: FrameTsfnType, fps: u32) -> Result<()> {
    if xcap.running.load(Ordering::SeqCst) {
      return Ok(());
    }

    xcap.running.store(true, Ordering::SeqCst);
    let running = xcap.running.clone();

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
            // img is RgbaImage (from image crate), which is ImageBuffer<Rgba<u8>, Vec<u8>>
            // into_raw() returns Vec<u8> (RGBA)
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

        // Cap at FPS
        let elapsed = start.elapsed();
        if elapsed < target_interval {
          thread::sleep(target_interval - elapsed);
        }
      }
    });

    xcap.handle = Some(handle);
    Ok(())
  }

  fn stop_xcap(xcap: &mut XCapBackend) -> Result<()> {
    xcap.running.store(false, Ordering::SeqCst);
    if let Some(handle) = xcap.handle.take() {
      let _ = handle.join();
    }
    Ok(())
  }
}

#[cfg(target_os = "macos")]
#[link(name = "System", kind = "dylib")]
extern "C" {
  fn dispatch_queue_create(label: *const i8, attr: *mut c_void) -> *mut c_void;
}
