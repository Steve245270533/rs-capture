use std::ffi::c_void;
use std::future::Future;
use std::pin::Pin;
use std::ptr;
use std::sync::{Arc, Mutex as StdMutex};

use block2::RcBlock;
use napi::bindgen_prelude::*;
use napi::threadsafe_function::ThreadsafeFunctionCallMode;
use objc2::AnyThread;
use objc2::{
  define_class, msg_send,
  rc::{Allocated, Retained},
  ClassType, DeclaredClass,
};
use objc2_core_media::{CMSampleBuffer, CMTime, CMTimeFlags};
use objc2_foundation::{NSArray, NSError, NSObject, NSObjectProtocol};
use objc2_screen_capture_kit::*;

use super::{CaptureBackendImpl, FrameDataInternal, FrameTsfnType};

#[link(name = "CoreMedia", kind = "framework")]
extern "C" {
  fn CMSampleBufferGetImageBuffer(sbuf: *mut c_void) -> *mut c_void;
}

#[link(name = "CoreVideo", kind = "framework")]
extern "C" {
  fn CVPixelBufferGetBaseAddress(pbuf: *mut c_void) -> *mut c_void;
  fn CVPixelBufferGetBytesPerRow(pbuf: *mut c_void) -> usize;
  fn CVPixelBufferGetWidth(pbuf: *mut c_void) -> usize;
  fn CVPixelBufferGetHeight(pbuf: *mut c_void) -> usize;
  fn CVPixelBufferLockBaseAddress(pbuf: *mut c_void, flags: u64) -> i32;
  fn CVPixelBufferUnlockBaseAddress(pbuf: *mut c_void, flags: u64) -> i32;
}

#[link(name = "System", kind = "dylib")]
extern "C" {
  fn dispatch_queue_create(label: *const i8, attr: *mut c_void) -> *mut c_void;
}

unsafe fn extract_frame(sample: &CMSampleBuffer) -> Option<FrameDataInternal> {
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

      CVPixelBufferUnlockBaseAddress(pixel_buffer, 1);
      return Some(FrameDataInternal {
        width: width as u32,
        height: height as u32,
        stride: (width * 4) as u32,
        data,
      });
    }
    CVPixelBufferUnlockBaseAddress(pixel_buffer, 1);
  }
  None
}

pub struct StreamDelegateIvars {
  tsfn_ptr: usize,
}

impl Drop for StreamDelegateIvars {
  fn drop(&mut self) {
    if self.tsfn_ptr != 0 {
      unsafe { drop(Box::from_raw(self.tsfn_ptr as *mut FrameTsfnType)) };
    }
  }
}

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
                         if let Some(frame) = extract_frame(sample) {
                             tsfn.call(frame, ThreadsafeFunctionCallMode::NonBlocking);
                         }
                     }
                 }
            }
        }
    }
);

unsafe impl SCStreamOutput for StreamDelegate {}
unsafe impl NSObjectProtocol for StreamDelegate {}
unsafe impl Send for StreamDelegate {}
unsafe impl Sync for StreamDelegate {}

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

pub struct ScreenshotDelegateIvars {
  tx_ptr: usize, // *mut Arc<StdMutex<Option<tokio::sync::oneshot::Sender<FrameDataInternal>>>>
}

impl Drop for ScreenshotDelegateIvars {
  fn drop(&mut self) {
    if self.tx_ptr != 0 {
      unsafe {
        drop(Box::from_raw(
          self.tx_ptr
            as *mut Arc<StdMutex<Option<tokio::sync::oneshot::Sender<FrameDataInternal>>>>,
        ))
      };
    }
  }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "ScreenshotDelegate"]
    #[ivars = ScreenshotDelegateIvars]
    pub struct ScreenshotDelegate;

    impl ScreenshotDelegate {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        fn did_output(&self, _stream: &SCStream, sample: &CMSampleBuffer, kind: SCStreamOutputType) {
            if kind == SCStreamOutputType::Screen {
                 let ptr = self.ivars().tx_ptr;
                 if ptr != 0 {
                     let tx_arc = unsafe { &*(ptr as *const Arc<StdMutex<Option<tokio::sync::oneshot::Sender<FrameDataInternal>>>>) };
                     // We need to check if we already sent.
                     // But we can't lock easily without blocking?
                     // Locking mutex is fine here.
                     let mut guard = tx_arc.lock().unwrap();
                     if let Some(tx) = guard.take() {
                         unsafe {
                             if let Some(frame) = extract_frame(sample) {
                                 let _ = tx.send(frame);
                             } else {
                                 // If failed to extract, put tx back?
                                 // Or just fail. If we put it back, we retry next frame.
                                 *guard = Some(tx);
                             }
                         }
                     }
                 }
            }
        }
    }
);

unsafe impl SCStreamOutput for ScreenshotDelegate {}
unsafe impl NSObjectProtocol for ScreenshotDelegate {}
unsafe impl Send for ScreenshotDelegate {}
unsafe impl Sync for ScreenshotDelegate {}

impl ScreenshotDelegate {
  fn new(tx: tokio::sync::oneshot::Sender<FrameDataInternal>) -> Retained<Self> {
    let arc = Arc::new(StdMutex::new(Some(tx)));
    let boxed = Box::new(arc);
    let ptr = Box::into_raw(boxed) as usize;

    let cls = Self::class();
    let obj: Allocated<Self> = unsafe { msg_send![cls, alloc] };
    let obj = obj.set_ivars(ScreenshotDelegateIvars { tx_ptr: ptr });
    unsafe { msg_send![super(obj), init] }
  }
}

struct SendRetained<T>(Retained<T>);
unsafe impl<T> Send for SendRetained<T> {}

pub struct SCKBackend {
  stream: Option<Retained<SCStream>>,
  delegate: Option<Retained<StreamDelegate>>,
}

unsafe impl Send for SCKBackend {}
unsafe impl Sync for SCKBackend {}

impl SCKBackend {
  pub fn new() -> Self {
    Self {
      stream: None,
      delegate: None,
    }
  }
}

impl Default for SCKBackend {
  fn default() -> Self {
    Self::new()
  }
}

impl CaptureBackendImpl for SCKBackend {
  fn start<'a>(
    &'a mut self,
    tsfn: Option<FrameTsfnType>,
    fps: u32,
  ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
    Box::pin(async move {
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

      // Scope to ensure !Send types are dropped before await (if any, though here we just process and return)
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

        let delegate = if let Some(tsfn) = tsfn {
          Some(StreamDelegate::new(tsfn))
        } else {
          None
        };

        let queue = unsafe { dispatch_queue_create(c"com.napi.sck".as_ptr(), ptr::null_mut()) };

        if let Some(delegate) = &delegate {
          unsafe {
            let _: bool = msg_send![&stream, addStreamOutput: &**delegate, type: SCStreamOutputType::Screen, sampleHandlerQueue: queue as *mut NSObject, error: ptr::null_mut::<*mut NSError>()];
          }
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

        (SendRetained(stream), delegate.map(SendRetained))
      };

      self.stream = Some(stream_wrapper.0);
      self.delegate = delegate_wrapper.map(|d| d.0);

      Ok(())
    })
  }

  fn stop(&mut self) -> Result<()> {
    if let Some(stream) = self.stream.take() {
      let handler = RcBlock::new(move |_error: *mut NSError| {});
      unsafe { stream.stopCaptureWithCompletionHandler(Some(&*handler)) };
    }
    self.delegate = None;
    Ok(())
  }

  fn screenshot<'a>(
    &'a mut self,
  ) -> Pin<Box<dyn Future<Output = Result<FrameDataInternal>> + Send + 'a>> {
    Box::pin(async move {
      let (tx, rx) = tokio::sync::oneshot::channel();
      let tx = Arc::new(StdMutex::new(Some(tx)));

      {
        let handler = RcBlock::new(
          move |content: *mut SCShareableContent, error: *mut NSError| {
            let mut tx_guard = tx.lock().unwrap();
            if let Some(tx) = tx_guard.take() {
              if !error.is_null() {
                let _ = tx.send(Err("SCK Error".to_string()));
              } else {
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

      let (stream_wrapper, _delegate_wrapper, frame_rx) = {
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
            timescale: 60,
            flags: CMTimeFlags(1),
            epoch: 0,
          });
          config.setQueueDepth(5);
          config.setPixelFormat(1111970369); // kCVPixelFormatType_32BGRA
        }

        let stream = unsafe {
          SCStream::initWithFilter_configuration_delegate(SCStream::alloc(), &filter, &config, None)
        };

        let (frame_tx, frame_rx) = tokio::sync::oneshot::channel();
        let delegate = ScreenshotDelegate::new(frame_tx);

        let queue =
          unsafe { dispatch_queue_create(c"com.napi.sck.screenshot".as_ptr(), ptr::null_mut()) };

        unsafe {
          let _: bool = msg_send![&stream, addStreamOutput: &*delegate, type: SCStreamOutputType::Screen, sampleHandlerQueue: queue as *mut NSObject, error: ptr::null_mut::<*mut NSError>()];
        }

        {
          let start_handler = RcBlock::new(move |_error: *mut NSError| {});
          unsafe {
            stream.startCaptureWithCompletionHandler(Some(&*start_handler));
          }
        }

        (SendRetained(stream), SendRetained(delegate), frame_rx)
      };

      // Wait for frame
      let frame_res = frame_rx.await;

      // Stop capture
      let stream = stream_wrapper.0;
      let handler = RcBlock::new(move |_error: *mut NSError| {});
      unsafe { stream.stopCaptureWithCompletionHandler(Some(&*handler)) };

      match frame_res {
        Ok(frame) => Ok(frame),
        Err(_) => Err(Error::new(
          Status::GenericFailure,
          "Failed to capture frame".to_string(),
        )),
      }
    })
  }
}
