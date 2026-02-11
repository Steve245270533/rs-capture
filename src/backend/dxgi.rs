use std::ffi::c_void;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use napi::threadsafe_function::ThreadsafeFunctionCallMode;
use napi::Status;
use windows::core::Interface;
use windows::Win32::Foundation::{HANDLE, HWND};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0};
use windows::Win32::Graphics::Direct3D11::{
  D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_CREATE_DEVICE_FLAG,
  D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
  D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::{
  CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput1, IDXGIOutputDuplication,
  IDXGIResource, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
};
use windows::Win32::Graphics::Gdi::{
  BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC,
  SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, HBITMAP, HDC,
  HGDIOBJ, ROP_CODE, SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

use super::{CaptureBackendImpl, FrameDataInternal, FrameTsfnType};

pub struct DxgiBackend {
  running: Arc<AtomicBool>,
  handle: Option<thread::JoinHandle<()>>,
}

unsafe impl Send for DxgiBackend {}
unsafe impl Sync for DxgiBackend {}

struct DxgiState {
  device: ID3D11Device,
  context: ID3D11DeviceContext,
  duplication: IDXGIOutputDuplication,
  fastlane: bool,
  width: u32,
  height: u32,
  staging_texture: Option<ID3D11Texture2D>,
}

enum DxgiCaptureError {
  AccessLost(anyhow::Error),
  Other(anyhow::Error),
}

struct GdiState {
  screen_dc: HDC,
  mem_dc: HDC,
  dib: HBITMAP,
  old_obj: HGDIOBJ,
  bits: *mut c_void,
  width: i32,
  height: i32,
}

impl Drop for GdiState {
  fn drop(&mut self) {
    unsafe {
      if !self.mem_dc.0.is_null() && !self.old_obj.0.is_null() {
        let _ = SelectObject(self.mem_dc, self.old_obj);
      }
      if !self.dib.0.is_null() {
        let _ = DeleteObject(HGDIOBJ(self.dib.0));
      }
      if !self.mem_dc.0.is_null() {
        let _ = DeleteDC(self.mem_dc);
      }
      if !self.screen_dc.0.is_null() {
        let _ = ReleaseDC(HWND(std::ptr::null_mut()), self.screen_dc);
      }
    }
  }
}

impl GdiState {
  unsafe fn new() -> Result<Self> {
    let width = GetSystemMetrics(SM_CXSCREEN);
    let height = GetSystemMetrics(SM_CYSCREEN);
    if width <= 0 || height <= 0 {
      return Err(anyhow!("Invalid screen size"));
    }

    let screen_dc = GetDC(HWND(std::ptr::null_mut()));
    if screen_dc.0.is_null() {
      return Err(anyhow!("GetDC failed"));
    }

    let mem_dc = CreateCompatibleDC(screen_dc);
    if mem_dc.0.is_null() {
      let _ = ReleaseDC(HWND(std::ptr::null_mut()), screen_dc);
      return Err(anyhow!("CreateCompatibleDC failed"));
    }

    let mut bmi = BITMAPINFO::default();
    bmi.bmiHeader = BITMAPINFOHEADER {
      biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
      biWidth: width,
      biHeight: -height,
      biPlanes: 1,
      biBitCount: 32,
      biCompression: BI_RGB.0 as u32,
      biSizeImage: 0,
      biXPelsPerMeter: 0,
      biYPelsPerMeter: 0,
      biClrUsed: 0,
      biClrImportant: 0,
    };

    let mut bits: *mut c_void = std::ptr::null_mut();
    let dib = CreateDIBSection(
      screen_dc,
      &bmi,
      DIB_RGB_COLORS,
      &mut bits,
      HANDLE(std::ptr::null_mut()),
      0,
    )?;
    if dib.0.is_null() || bits.is_null() {
      let _ = DeleteDC(mem_dc);
      let _ = ReleaseDC(HWND(std::ptr::null_mut()), screen_dc);
      return Err(anyhow!("CreateDIBSection failed"));
    }

    let old_obj = SelectObject(mem_dc, HGDIOBJ(dib.0));
    if old_obj.0.is_null() {
      let _ = DeleteObject(HGDIOBJ(dib.0));
      let _ = DeleteDC(mem_dc);
      let _ = ReleaseDC(HWND(std::ptr::null_mut()), screen_dc);
      return Err(anyhow!("SelectObject failed"));
    }

    Ok(Self {
      screen_dc,
      mem_dc,
      dib,
      old_obj,
      bits,
      width,
      height,
    })
  }

  unsafe fn capture_frame(&mut self) -> Result<FrameDataInternal> {
    let rop = ROP_CODE(SRCCOPY.0 | CAPTUREBLT.0);
    BitBlt(
      self.mem_dc,
      0,
      0,
      self.width,
      self.height,
      self.screen_dc,
      0,
      0,
      rop,
    )?;

    let data = bgra_to_rgba_compact_opaque(
      self.bits as *const u8,
      (self.width as usize) * 4,
      self.width as u32,
      self.height as u32,
    );

    Ok(FrameDataInternal {
      width: self.width as u32,
      height: self.height as u32,
      stride: (self.width as u32) * 4,
      data,
    })
  }
}

enum CaptureMode {
  Dxgi(DxgiState),
  Gdi(GdiState),
}

unsafe fn init_capture_mode() -> Result<CaptureMode> {
  match DxgiState::new() {
    Ok(dxgi) => Ok(CaptureMode::Dxgi(dxgi)),
    Err(dxgi_err) => match GdiState::new() {
      Ok(gdi) => Ok(CaptureMode::Gdi(gdi)),
      Err(gdi_err) => Err(anyhow!(
        "DXGI init failed: {:?}; GDI init failed: {:?}",
        dxgi_err,
        gdi_err
      )),
    },
  }
}

impl DxgiState {
  unsafe fn new() -> Result<Self> {
    let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
    let adapter = get_adapter(&factory)?;
    let output = get_output(&adapter)?;
    let output1: IDXGIOutput1 = output.cast()?;

    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;

    D3D11CreateDevice(
      &adapter,
      D3D_DRIVER_TYPE_UNKNOWN,
      None,
      D3D11_CREATE_DEVICE_FLAG(0),
      Some(&[D3D_FEATURE_LEVEL_11_0]),
      D3D11_SDK_VERSION,
      Some(&mut device),
      None,
      Some(&mut context),
    )?;

    let device = device.ok_or_else(|| anyhow!("Failed to create D3D11 device"))?;
    let context = context.ok_or_else(|| anyhow!("Failed to create D3D11 context"))?;
    let duplication = output1.DuplicateOutput(&device)?;

    let dupl_desc = duplication.GetDesc();
    let fastlane = dupl_desc.DesktopImageInSystemMemory.as_bool();
    let width = dupl_desc.ModeDesc.Width;
    let height = dupl_desc.ModeDesc.Height;

    Ok(Self {
      device,
      context,
      duplication,
      fastlane,
      width,
      height,
      staging_texture: None,
    })
  }

  unsafe fn capture_frame(
    &mut self,
    timeout_ms: u32,
  ) -> std::result::Result<Option<FrameDataInternal>, DxgiCaptureError> {
    let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
    let mut resource: Option<IDXGIResource> = None;

    match self
      .duplication
      .AcquireNextFrame(timeout_ms, &mut frame_info, &mut resource)
    {
      Ok(_) => {}
      Err(e) => {
        if e.code() == DXGI_ERROR_WAIT_TIMEOUT {
          return Ok(None);
        }
        if e.code() == DXGI_ERROR_ACCESS_LOST {
          return Err(DxgiCaptureError::AccessLost(anyhow!(
            "AcquireNextFrame failed: {:?}",
            e
          )));
        }
        return Err(DxgiCaptureError::Other(anyhow!(
          "AcquireNextFrame failed: {:?}",
          e
        )));
      }
    }

    struct ReleaseGuard(IDXGIOutputDuplication);
    impl Drop for ReleaseGuard {
      fn drop(&mut self) {
        let _ = unsafe { self.0.ReleaseFrame() };
      }
    }

    let _guard = ReleaseGuard(self.duplication.clone());

    if self.fastlane {
      struct SurfaceUnmapGuard(IDXGIOutputDuplication);
      impl Drop for SurfaceUnmapGuard {
        fn drop(&mut self) {
          let _ = unsafe { self.0.UnMapDesktopSurface() };
        }
      }

      let _surface_guard = SurfaceUnmapGuard(self.duplication.clone());
      let mapped = match self.duplication.MapDesktopSurface() {
        Ok(m) => m,
        Err(e) => {
          if e.code() == DXGI_ERROR_ACCESS_LOST {
            return Err(DxgiCaptureError::AccessLost(anyhow!(
              "MapDesktopSurface failed: {:?}",
              e
            )));
          }
          return Err(DxgiCaptureError::Other(anyhow!(
            "MapDesktopSurface failed: {:?}",
            e
          )));
        }
      };

      let src_ptr = mapped.pBits as *const u8;
      let src_stride = mapped.Pitch as usize;
      let data = bgra_to_rgba_compact(src_ptr, src_stride, self.width, self.height);

      return Ok(Some(FrameDataInternal {
        width: self.width,
        height: self.height,
        stride: self.width * 4,
        data,
      }));
    }

    let Some(res) = resource else {
      return Ok(None);
    };
    let texture: ID3D11Texture2D = match res.cast() {
      Ok(t) => t,
      Err(e) => {
        return Err(DxgiCaptureError::Other(anyhow!(
          "Cast to texture failed: {:?}",
          e
        )));
      }
    };

    let mut desc = D3D11_TEXTURE2D_DESC::default();
    texture.GetDesc(&mut desc);

    let needs_new_staging = match self.staging_texture.as_ref() {
      None => true,
      Some(staging) => {
        let mut current_desc = D3D11_TEXTURE2D_DESC::default();
        staging.GetDesc(&mut current_desc);
        current_desc.Width != desc.Width || current_desc.Height != desc.Height
      }
    };

    if needs_new_staging {
      desc.Usage = D3D11_USAGE_STAGING;
      desc.BindFlags = 0;
      desc.CPUAccessFlags = windows::Win32::Graphics::Direct3D11::D3D11_CPU_ACCESS_READ.0 as u32;
      desc.MiscFlags = 0;

      let mut new_staging: Option<ID3D11Texture2D> = None;
      if let Err(e) = self
        .device
        .CreateTexture2D(&desc, None, Some(&mut new_staging))
      {
        return Err(DxgiCaptureError::Other(anyhow!(
          "CreateTexture2D failed: {:?}",
          e
        )));
      }
      self.staging_texture = new_staging;
    }

    let Some(staging) = &self.staging_texture else {
      return Ok(None);
    };

    self.context.CopyResource(staging, &texture);

    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    if let Err(e) = self
      .context
      .Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
    {
      if e.code() == DXGI_ERROR_ACCESS_LOST {
        return Err(DxgiCaptureError::AccessLost(anyhow!("Map failed: {:?}", e)));
      }
      return Err(DxgiCaptureError::Other(anyhow!("Map failed: {:?}", e)));
    }

    let width = desc.Width;
    let height = desc.Height;
    let src_stride = mapped.RowPitch as usize;
    let src_ptr = mapped.pData as *const u8;

    let data = bgra_to_rgba_compact(src_ptr, src_stride, width, height);

    self.context.Unmap(staging, 0);

    Ok(Some(FrameDataInternal {
      width,
      height,
      stride: width * 4,
      data,
    }))
  }
}

impl DxgiBackend {
  pub fn new() -> Result<Self> {
    unsafe {
      if DxgiState::new().is_err() && GdiState::new().is_err() {
        return Err(anyhow!("Neither DXGI nor GDI capture is available"));
      }
    }

    Ok(Self {
      running: Arc::new(AtomicBool::new(false)),
      handle: None,
    })
  }
}

unsafe fn get_adapter(factory: &IDXGIFactory1) -> Result<IDXGIAdapter1> {
  factory
    .EnumAdapters1(0)
    .map_err(|_| anyhow!("No DXGI adapter found"))
}

unsafe fn get_output(adapter: &IDXGIAdapter1) -> Result<IDXGIOutput1> {
  let output = adapter
    .EnumOutputs(0)
    .map_err(|_| anyhow!("No DXGI output found"))?;
  let output1: IDXGIOutput1 = output.cast()?;
  Ok(output1)
}

fn bgra_to_rgba_compact(src_ptr: *const u8, src_stride: usize, width: u32, height: u32) -> Vec<u8> {
  let w = width as usize;
  let h = height as usize;
  let mut dst = vec![0u8; w * h * 4];

  for y in 0..h {
    let src_row = unsafe { src_ptr.add(y * src_stride) };
    for x in 0..w {
      let src_px = unsafe { src_row.add(x * 4) };
      let dst_i = (y * w + x) * 4;
      dst[dst_i] = unsafe { *src_px.add(2) };
      dst[dst_i + 1] = unsafe { *src_px.add(1) };
      dst[dst_i + 2] = unsafe { *src_px.add(0) };
      dst[dst_i + 3] = unsafe { *src_px.add(3) };
    }
  }

  dst
}

fn bgra_to_rgba_compact_opaque(
  src_ptr: *const u8,
  src_stride: usize,
  width: u32,
  height: u32,
) -> Vec<u8> {
  let w = width as usize;
  let h = height as usize;
  let mut dst = vec![0u8; w * h * 4];

  for y in 0..h {
    let src_row = unsafe { src_ptr.add(y * src_stride) };
    for x in 0..w {
      let src_px = unsafe { src_row.add(x * 4) };
      let dst_i = (y * w + x) * 4;
      dst[dst_i] = unsafe { *src_px.add(2) };
      dst[dst_i + 1] = unsafe { *src_px.add(1) };
      dst[dst_i + 2] = unsafe { *src_px.add(0) };
      dst[dst_i + 3] = 255;
    }
  }

  dst
}

impl CaptureBackendImpl for DxgiBackend {
  fn start<'a>(
    &'a mut self,
    tsfn: FrameTsfnType,
    fps: u32,
  ) -> Pin<Box<dyn Future<Output = napi::Result<()>> + Send + 'a>> {
    Box::pin(async move {
      if self.running.load(Ordering::SeqCst) {
        return Ok(());
      }

      self.running.store(true, Ordering::SeqCst);
      let running = self.running.clone();

      let handle = thread::spawn(move || {
        let result = unsafe { run_capture_loop(running.clone(), tsfn, fps) };
        if let Err(e) = result {
          eprintln!("DXGI Capture Loop Error: {:?}", e);
          running.store(false, Ordering::SeqCst);
        }
      });

      self.handle = Some(handle);
      Ok(())
    })
  }

  fn stop(&mut self) -> napi::Result<()> {
    self.running.store(false, Ordering::SeqCst);
    if let Some(handle) = self.handle.take() {
      let _ = handle.join();
    }
    Ok(())
  }
}

unsafe fn run_capture_loop(running: Arc<AtomicBool>, tsfn: FrameTsfnType, fps: u32) -> Result<()> {
  let mut mode = init_capture_mode()?;
  let target_interval = Duration::from_secs_f64(1.0 / fps as f64);

  while running.load(Ordering::SeqCst) {
    let start_time = Instant::now();

    match &mut mode {
      CaptureMode::Dxgi(state) => match state.capture_frame(100) {
        Ok(Some(frame)) => {
          let status = tsfn.call(frame, ThreadsafeFunctionCallMode::NonBlocking);
          if status != Status::Ok {
            running.store(false, Ordering::SeqCst);
          }
        }
        Ok(None) => {}
        Err(DxgiCaptureError::AccessLost(e)) => {
          eprintln!("DXGI access lost: {:?}", e);
          match DxgiState::new() {
            Ok(new_state) => mode = CaptureMode::Dxgi(new_state),
            Err(_) => match GdiState::new() {
              Ok(gdi) => mode = CaptureMode::Gdi(gdi),
              Err(e) => return Err(e),
            },
          }
        }
        Err(DxgiCaptureError::Other(e)) => match GdiState::new() {
          Ok(gdi) => mode = CaptureMode::Gdi(gdi),
          Err(_) => return Err(e),
        },
      },
      CaptureMode::Gdi(gdi) => {
        let frame = gdi.capture_frame()?;
        let status = tsfn.call(frame, ThreadsafeFunctionCallMode::NonBlocking);
        if status != Status::Ok {
          running.store(false, Ordering::SeqCst);
        }
      }
    }

    let elapsed = start_time.elapsed();
    if elapsed < target_interval {
      thread::sleep(target_interval - elapsed);
    }
  }

  Ok(())
}
