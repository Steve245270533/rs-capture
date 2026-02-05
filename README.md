# @vertfrag/rs-capture

[中文文档](./README.zh-CN.md)

Cross-platform screen capture library for Node.js powered by Rust.

`rs-capture` provides high-performance screen capturing capabilities by leveraging native APIs through Rust. It uses **ScreenCaptureKit** on macOS for optimal performance and falls back to **XCap** (WebRTC/other native APIs) for cross-platform compatibility (Windows, Linux).

## Features

- 🚀 **High Performance**: Built with Rust and N-API for minimal overhead.
- 🖥️ **Cross-Platform**: Supports macOS, Windows, and Linux.
- 🍎 **ScreenCaptureKit Support**: Utilizes Apple's latest ScreenCaptureKit on macOS for efficient, low-latency capture.
- 🔧 **Configurable**: Control frame rate (FPS) and backend selection.
- 📦 **Easy Integration**: Simple callback-based API receiving raw RGBA frame data.

## Installation

```bash
npm install @vertfrag/rs-capture
# or
pnpm add @vertfrag/rs-capture
```

## Supported Platforms

| Platform | Architecture | Backend                          |
| -------- | ------------ | -------------------------------- |
| macOS    | x64, arm64   | ScreenCaptureKit (Default), XCap |
| Windows  | x64, arm64   | XCap                             |
| Linux    | x64          | XCap                             |

## Usage

```javascript
import { ScreenCapture, CaptureBackend } from '@vertfrag/rs-capture'

// Callback function to handle captured frames
const onFrame = (frame) => {
  // frame.rgba is a Buffer containing raw RGBA pixel data
  console.log(`Frame received: ${frame.width}x${frame.height}, Stride: ${frame.stride}`)
  console.log(`Data length: ${frame.rgba.length}`)
}

// Configuration (Optional)
const config = {
  fps: 60, // Target FPS (Default: 60)
  // On macOS, you can explicitly choose the backend.
  // Defaults to ScreenCaptureKit on macOS, and XCap on others.
  backend: CaptureBackend.ScreenCaptureKit,
}

try {
  // Initialize the capturer
  const capturer = new ScreenCapture(onFrame, config)

  // Start capturing
  console.log('Starting capture...')
  await capturer.start()

  // Keep capturing for 5 seconds
  setTimeout(() => {
    capturer.stop()
    console.log('Capture stopped')
  }, 5000)
} catch (err) {
  console.error('Error:', err)
}
```

## API Reference

### `ScreenCapture`

The main class for controlling screen capture.

#### `constructor(callback: (frame: FrameData) => void, config?: ScreenCaptureConfig)`

Creates a new `ScreenCapture` instance.

- **callback**: A function called whenever a new frame is captured. The callback receives a `FrameData` object.
- **config**: Optional configuration object to control backend and FPS.

#### `start(): Promise<void>`

Starts the screen capture session asynchronously. Returns a Promise that resolves when capturing has successfully started.

#### `stop(): void`

Stops the screen capture session immediately.

### `FrameData`

The object passed to the callback function.

| Property | Type     | Description                                    |
| -------- | -------- | ---------------------------------------------- |
| `width`  | `number` | Width of the captured frame in pixels.         |
| `height` | `number` | Height of the captured frame in pixels.        |
| `stride` | `number` | Number of bytes per row (usually `width * 4`). |
| `rgba`   | `Buffer` | Raw pixel data in RGBA format.                 |

### `ScreenCaptureConfig`

| Property  | Type             | Description                                |
| --------- | ---------------- | ------------------------------------------ |
| `fps`     | `number`         | Target frames per second. Default is `60`. |
| `backend` | `CaptureBackend` | Explicitly choose the capture backend.     |

### `CaptureBackend`

Enum for selecting the capture backend.

```typescript
export const enum CaptureBackend {
  ScreenCaptureKit = 'ScreenCaptureKit',
  XCap = 'XCap',
}
```

- **ScreenCaptureKit**: Uses macOS native ScreenCaptureKit (High performance, macOS 12.3+).
- **XCap**: Uses a cross-platform implementation (based on WebRTC/native APIs).

## Development

### Requirements

- Install the latest [Rust](https://rustup.rs/)
- Install Node.js >= 10
- Install pnpm (recommended via Corepack)

### Build & Test

1. **Install dependencies**:

   ```bash
   pnpm install
   ```

2. **Build the project**:

   ```bash
   pnpm build
   ```

   This will compile the Rust code and generate the native addon.

3. **Run tests**:
   ```bash
   pnpm test
   ```

## License

MIT
