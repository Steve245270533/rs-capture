# @vertfrag/rs-capture

[English](./README.md)

基于 Rust 的 Node.js 跨平台屏幕捕获库。

`rs-capture` 利用 Rust 和原生 API 提供高性能的屏幕捕获能力。在 macOS 上，它使用 **ScreenCaptureKit** 以获得最佳性能；在 Windows 和 Linux 等其他平台上，则回退到 **XCap**（基于 WebRTC/其他原生 API）以保证跨平台兼容性。

## 特性

- 🚀 **高性能**：基于 Rust 和 N-API 构建，开销极低。
- 🖥️ **跨平台**：支持 macOS、Windows 和 Linux。
- 🍎 **ScreenCaptureKit 支持**：在 macOS 上利用 Apple 最新的 ScreenCaptureKit 实现高效、低延迟的捕获。
- 🔧 **可配置**：支持控制帧率 (FPS) 和后端选择。
- 📦 **易于集成**：简单的基于回调的 API，直接接收原始 RGBA 帧数据。

## 安装

```bash
npm install @vertfrag/rs-capture
# 或
pnpm add @vertfrag/rs-capture
```

## 支持平台

| 平台    | 架构       | 后端                          |
| ------- | ---------- | ----------------------------- |
| macOS   | x64, arm64 | ScreenCaptureKit (默认), XCap |
| Windows | x64, arm64 | DXGI (GDI 回退), XCap         |
| Linux   | x64        | XCap                          |

## 使用方法

```javascript
import { ScreenCapture, CaptureBackend } from '@vertfrag/rs-capture'

// 处理捕获帧的回调函数
const onFrame = (frame) => {
  // frame.rgba 是包含原始 RGBA 像素数据的 Buffer
  console.log(`Frame received: ${frame.width}x${frame.height}, Stride: ${frame.stride}`)
  console.log(`Data length: ${frame.rgba.length}`)
}

// 配置（可选）
const config = {
  fps: 60, // 目标帧率（默认：60）
  // 在 macOS 上，你可以显式选择后端。
  // macOS 上默认为 ScreenCaptureKit，其他平台默认为 XCap。
  backend: CaptureBackend.ScreenCaptureKit,
}

try {
  // 初始化捕获器
  const capturer = new ScreenCapture(onFrame, config)

  // 获取单个截图
  console.log('Taking screenshot...')
  const frame = await capturer.screenshot()
  console.log(`Screenshot captured: ${frame.width}x${frame.height}`)

  // 开始捕获
  console.log('Starting capture...')
  await capturer.start()

  // 持续捕获 5 秒
  setTimeout(() => {
    capturer.stop()
    console.log('Capture stopped')
  }, 5000)
} catch (err) {
  console.error('Error:', err)
}
```

## API 参考

### `ScreenCapture`

控制屏幕捕获的主类。

#### `constructor(callback: (frame: FrameData) => void, config?: ScreenCaptureConfig)`

创建一个新的 `ScreenCapture` 实例。

- **callback**: 每当捕获到新帧时调用的函数。回调接收一个 `FrameData` 对象。
- **config**: 可选的配置对象，用于控制后端和 FPS。

#### `start(): Promise<void>`

异步开始屏幕捕获会话。返回一个 Promise，当捕获成功开始时解析。

#### `stop(): void`

立即停止屏幕捕获会话。

#### `screenshot(): Promise<FrameData>`

立即捕获单个帧。返回一个解析为 `FrameData` 的 Promise。

### `FrameData`

传递给回调函数的对象。

| 属性     | 类型     | 描述                                 |
| -------- | -------- | ------------------------------------ |
| `width`  | `number` | 捕获帧的宽度（像素）。               |
| `height` | `number` | 捕获帧的高度（像素）。               |
| `stride` | `number` | 每行的字节数（通常为 `width * 4`）。 |
| `rgba`   | `Buffer` | RGBA 格式的原始像素数据。            |

### `ScreenCaptureConfig`

| 属性      | 类型             | 描述                    |
| --------- | ---------------- | ----------------------- |
| `fps`     | `number`         | 目标帧率。默认为 `60`。 |
| `backend` | `CaptureBackend` | 显式选择捕获后端。      |

### `CaptureBackend`

用于选择捕获后端的枚举。

```typescript
export const enum CaptureBackend {
  ScreenCaptureKit = 'ScreenCaptureKit',
  XCap = 'XCap',
}
```

- **ScreenCaptureKit**: 使用 macOS 原生 ScreenCaptureKit（高性能，macOS 12.3+）。
- **XCap**: 使用跨平台实现（基于 WebRTC/原生 API）。

## 开发

### 环境要求

- 安装最新的 [Rust](https://rustup.rs/)
- 安装 Node.js >= 10
- 安装 pnpm（推荐通过 Corepack）

### 构建与测试

1. **安装依赖**：

   ```bash
   pnpm install
   ```

2. **构建项目**：

   ```bash
   pnpm build
   ```

   这将编译 Rust 代码并生成原生插件。

3. **运行测试**：
   ```bash
   pnpm test
   ```

## 许可证

MIT
