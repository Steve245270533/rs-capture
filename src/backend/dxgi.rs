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
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0};
use windows::Win32::Graphics::Direct3D11::{
  D3D11CreateDevice, D3D11_BIND_FLAG, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
  D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_FLAG, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
  D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::{
  CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput1,
  IDXGIResource, DXGI_ERROR_WAIT_TIMEOUT, DXGI_MAP_READ, DXGI_OUTDUPL_FRAME_INFO,
};

use super::{CaptureBackendImpl, FrameDataInternal, FrameTsfnType};

pub struct DxgiBackend {
  running: Arc<AtomicBool>,
  handle: Option<thread::JoinHandle<()>>,
}

unsafe impl Send for DxgiBackend {}
unsafe impl Sync for DxgiBackend {}

impl DxgiBackend {
  pub fn new() -> Result<Self> {
    // Try to initialize DXGI to check compatibility
    // We won't keep the objects here to avoid Send/Sync issues with COM across threads easily
    // Instead, we verify we can create them, then drop them. The actual loop will recreate them.
    // Or better: just return Ok(Self) and let start() handle the creation and return error if it fails?
    // But the requirement is "if init fails, fallback".
    // So we should do a check here.

    unsafe {
      let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
      let adapter = get_adapter(&factory)?;
      let output = get_output(&adapter)?;

      let mut device: Option<ID3D11Device> = None;
      let mut context: Option<ID3D11DeviceContext> = None;

      // D3D11CreateDevice
      D3D11CreateDevice(
        &adapter,
        D3D_DRIVER_TYPE_UNKNOWN, // Use adapter
        None,
        D3D11_CREATE_DEVICE_FLAG(0),
        Some(&[D3D_FEATURE_LEVEL_11_0]),
        D3D11_SDK_VERSION,
        Some(&mut device),
        None,
        Some(&mut context),
      )?;

      let device = device.ok_or_else(|| anyhow!("Failed to create D3D11 device"))?;

      // Try DuplicateOutput
      let output1: IDXGIOutput1 = output.cast()?;
      // This might fail if another app is using it or if not supported
      // We just test it. Note: DuplicateOutput fails if we are not on the active desktop.
      // But we don't want to steal ownership yet if we can't keep it.
      // Actually, we can keep the logic in start(), but `windows.rs` needs to know if it SHOULD use DXGI.
      // Let's assume if we can create device and find output, we are good to GO.
      // A full duplication test might be too heavy or cause flicker.
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
  // 1. Init D3D11 and DXGI
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

  // 2. Duplicate Output
  let duplication = output1.DuplicateOutput(&device)?;

  let target_interval = Duration::from_secs_f64(1.0 / fps as f64);
  let mut staging_texture: Option<ID3D11Texture2D> = None;

  while running.load(Ordering::SeqCst) {
    let start_time = Instant::now();
    let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
    let mut resource: Option<IDXGIResource> = None;

    // AcquireNextFrame
    // Timeout 0 to be non-blocking? Or some ms?
    // Using 100ms timeout
    match duplication.AcquireNextFrame(100, &mut frame_info, &mut resource) {
      Ok(_) => {
        if let Some(res) = resource {
          let texture: ID3D11Texture2D = res.cast()?;

          // Create staging texture if needed
          let mut desc = D3D11_TEXTURE2D_DESC::default();
          texture.GetDesc(&mut desc);

          if staging_texture.is_none() || {
            let mut current_desc = D3D11_TEXTURE2D_DESC::default();
            staging_texture.as_ref().unwrap().GetDesc(&mut current_desc);
            current_desc.Width != desc.Width || current_desc.Height != desc.Height
          } {
            desc.Usage = D3D11_USAGE_STAGING;
            desc.BindFlags = D3D11_BIND_FLAG(0); // 0 for staging
            desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
            desc.MiscFlags = windows::Win32::Graphics::Direct3D11::D3D11_RESOURCE_MISC_FLAG(0);

            let mut new_staging: Option<ID3D11Texture2D> = None;
            device.CreateTexture2D(&desc, None, Some(&mut new_staging))?;
            staging_texture = new_staging;
          }

          if let Some(staging) = &staging_texture {
            context.CopyResource(staging, &texture);

            // Map
            let mut mapped =
              windows::Win32::Graphics::Direct3D11::D3D11_MAPPED_SUBRESOURCE::default();
            context.Map(staging, 0, DXGI_MAP_READ, 0, Some(&mut mapped))?;

            let width = desc.Width;
            let height = desc.Height;
            let src_stride = mapped.RowPitch as usize;
            let src_ptr = mapped.pData as *const u8;

            // Copy data
            // DXGI usually returns BGRA
            // We need to copy to Vec<u8> and potentially convert to RGBA if needed,
            // but rs-capture/sck.rs seems to expect RGBA based on "Compact and Swap RB (BGRA -> RGBA)" comment in macos.rs.
            // Wait, macos.rs does manual swap.
            // XCap uses `image` crate which usually handles it.
            // Let's do BGRA -> RGBA conversion here to be consistent.

            let mut data = Vec::with_capacity((width * height * 4) as usize);

            for y in 0..height {
              let row_start = src_ptr.add((y as usize) * src_stride);
              let row_slice = std::slice::from_raw_parts(row_start, (width * 4) as usize);

              // DXGI is B G R A
              for chunk in row_slice.chunks_exact(4) {
                data.push(chunk[2]); // R
                data.push(chunk[1]); // G
                data.push(chunk[0]); // B
                data.push(chunk[3]); // A
              }
            }

            context.Unmap(staging, 0);

            let frame = FrameDataInternal {
              width: width,
              height: height,
              stride: width * 4,
              data,
            };

            let status = tsfn.call(frame, ThreadsafeFunctionCallMode::NonBlocking);
            if status != Status::Ok {
              running.store(false, Ordering::SeqCst);
            }
          }
        }

        let _ = duplication.ReleaseFrame();
      }
      Err(e) => {
        if e.code() == DXGI_ERROR_WAIT_TIMEOUT {
          // Timeout is fine, just continue
        } else {
          // Real error (e.g. mode change)
          // We should probably try to re-initialize or break
          // For now, break to trigger fallback or stop
          return Err(anyhow!("AcquireNextFrame failed: {:?}", e));
        }
      }
    }

    // FPS cap
    let elapsed = start_time.elapsed();
    if elapsed < target_interval {
      thread::sleep(target_interval - elapsed);
    }
  }

  Ok(())
}
