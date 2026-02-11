const express = require('express')
const WebSocket = require('ws')
const http = require('http')
const path = require('path')
const os = require('os')
const { spawn } = require('child_process')
const { mouse, keyboard, Button, Key, Point } = require('@nut-tree/nut-js')
const { ScreenCapture } = require('@vertfrag/rs-capture')

const app = express()
const server = http.createServer(app)
const wss = new WebSocket.Server({ server, perMessageDeflate: false })

const JPEG_QUALITY = Number.parseInt(process.env.CAP_JPEG_QUALITY ?? '60', 10)
const MAX_WIDTH = Number.parseInt(process.env.CAP_MAX_WIDTH ?? (process.platform === 'win32' ? '1280' : '0'), 10)
const CAP_FPS = Number.parseInt(process.env.CAP_FPS ?? '60', 10)
const CAP_ENCODER = (process.env.CAP_ENCODER ?? (process.platform === 'win32' ? 'ffmpeg' : 'sharp')).toLowerCase()
const CAP_FFMPEG_PATH = process.env.CAP_FFMPEG_PATH ?? null
let DEFAULT_FFMPEG_PATH = null
try {
  DEFAULT_FFMPEG_PATH = require('@ffmpeg-installer/ffmpeg').path
} catch {}
const CAP_FFMPEG = CAP_FFMPEG_PATH || DEFAULT_FFMPEG_PATH || 'ffmpeg'

let sharp = null
let sharpInitTried = false
let sharpUnavailableLogged = false

function getSharp() {
  if (sharp) return sharp
  if (sharpInitTried) return null
  sharpInitTried = true
  try {
    sharp = require('sharp')
    sharp.cache(false)
    sharp.concurrency(Math.max(1, Math.min(os.cpus().length, 4)))
    return sharp
  } catch (e) {
    if (!sharpUnavailableLogged) {
      sharpUnavailableLogged = true
      console.error('sharp is not available (build scripts may be disabled). Set CAP_ENCODER=ffmpeg to avoid sharp.')
    }
    return null
  }
}

console.log(`encoder config: encoder=${CAP_ENCODER} fps=${CAP_FPS} maxWidth=${MAX_WIDTH} jpegQuality=${JPEG_QUALITY}`)
if (CAP_ENCODER === 'ffmpeg') {
  console.log(`encoder config: ffmpeg=${CAP_FFMPEG}`)
}

// Configure nut-js
mouse.config.autoDelayMs = 0
keyboard.config.autoDelayMs = 0

// Serve static files from 'public' directory
app.use(express.static(path.join(__dirname, 'public')))

wss.on('connection', (ws) => {
  console.log('Client connected')
  if (ws._socket) {
    ws._socket.setNoDelay(true)
  }

  try {
    attachClient(ws)
  } catch (err) {
    console.error('Failed to initialize capture:', err)
    ws.close()
  }

  ws.on('message', async (message) => {
    try {
      const event = JSON.parse(message)
      await handleInputEvent(event)
    } catch (err) {
      console.error('Invalid message format:', err)
    }
  })

  ws.on('close', () => {
    console.log('Client disconnected')
    detachClient(ws)
  })

  ws.on('error', (err) => {
    console.error('WebSocket error:', err)
    detachClient(ws)
  })
})

const PORT = 3000

// Helper to map robotjs string keys to nut.js Key enum
function mapKey(key) {
  if (!key) return null
  const k = key.toLowerCase()
  // Basic mapping - extend as needed
  const map = {
    backspace: Key.Backspace,
    delete: Key.Delete,
    enter: Key.Enter,
    tab: Key.Tab,
    escape: Key.Escape,
    up: Key.Up,
    down: Key.Down,
    left: Key.Left,
    right: Key.Right,
    home: Key.Home,
    end: Key.End,
    pageup: Key.PageUp,
    pagedown: Key.PageDown,
    space: Key.Space,
    command: Key.LeftCmd,
    alt: Key.LeftAlt,
    control: Key.LeftControl,
    shift: Key.LeftShift,
    // Alphanumeric
    a: Key.A,
    b: Key.B,
    c: Key.C,
    d: Key.D,
    e: Key.E,
    f: Key.F,
    g: Key.G,
    h: Key.H,
    i: Key.I,
    j: Key.J,
    k: Key.K,
    l: Key.L,
    m: Key.M,
    n: Key.N,
    o: Key.O,
    p: Key.P,
    q: Key.Q,
    r: Key.R,
    s: Key.S,
    t: Key.T,
    u: Key.U,
    v: Key.V,
    w: Key.W,
    x: Key.X,
    y: Key.Y,
    z: Key.Z,
    0: Key.Num0,
    1: Key.Num1,
    2: Key.Num2,
    3: Key.Num3,
    4: Key.Num4,
    5: Key.Num5,
    6: Key.Num6,
    7: Key.Num7,
    8: Key.Num8,
    9: Key.Num9,
  }
  return map[k]
}

async function handleInputEvent(event) {
  try {
    const { type, x, y, button, key, modifiers } = event

    switch (type) {
      case 'mousemove':
        await mouse.setPosition(new Point(x, y))
        break
      case 'mousedown': {
        const btn = button === 'right' ? Button.RIGHT : button === 'middle' ? Button.MIDDLE : Button.LEFT
        await mouse.pressButton(btn)
        break
      }
      case 'mouseup': {
        const btn = button === 'right' ? Button.RIGHT : button === 'middle' ? Button.MIDDLE : Button.LEFT
        await mouse.releaseButton(btn)
        break
      }
      case 'click': {
        const btn = button === 'right' ? Button.RIGHT : button === 'middle' ? Button.MIDDLE : Button.LEFT
        await mouse.click(btn)
        break
      }
      case 'dblclick': {
        const btn = button === 'right' ? Button.RIGHT : button === 'middle' ? Button.MIDDLE : Button.LEFT
        await mouse.doubleClick(btn)
        break
      }
      case 'keydown': {
        const k = mapKey(key)
        if (k !== null) await keyboard.pressKey(k)
        break
      }
      case 'keyup': {
        const k = mapKey(key)
        if (k !== null) await keyboard.releaseKey(k)
        break
      }
      case 'keypress': {
        const k = mapKey(key)
        if (k !== null) {
          await keyboard.pressKey(k)
          await keyboard.releaseKey(k)
        }
        break
      }
      case 'scroll':
        // nut.js scroll support if needed
        // await mouse.scrollDown(amount)
        break
    }
  } catch (err) {
    console.error('NutJS error:', err)
  }
}

server.listen(PORT, () => {
  console.log(`Server running at http://localhost:${PORT}`)
})

let capture = null
let isProcessing = false
const clients = new Set()
let encodedFrames = 0
let broadcastFrames = 0
let lastStatsAt = Date.now()
let encoder = null
let encoderW = 0
let encoderH = 0
let encoderBackpressure = false

// setInterval(() => {
//   const now = Date.now()
//   const dt = (now - lastStatsAt) / 1000
//   lastStatsAt = now
//   const encFps = Math.round(encodedFrames / dt)
//   const outFps = Math.round(broadcastFrames / dt)
//   encodedFrames = 0
//   broadcastFrames = 0
//   console.log(`stats: clients=${clients.size} enc_fps=${encFps} out_fps=${outFps} encoder=${encoder ? CAP_ENCODER : 'none'}`)
// }, 1000).unref()

function attachClient(ws) {
  clients.add(ws)
  if (!capture) {
    startSharedCapture()
  }
}

function detachClient(ws) {
  clients.delete(ws)
  if (clients.size === 0) {
    stopSharedCapture()
  }
}

function stopSharedCapture() {
  if (!capture) return
  try {
    capture.stop()
  } catch (e) {
    console.error('Capture stop error:', e)
  }
  capture = null
  stopEncoder()
}

function stopEncoder() {
  if (!encoder) return
  try {
    encoder.close()
  } catch {}
  encoder = null
  encoderW = 0
  encoderH = 0
  encoderBackpressure = false
}

function qualityToQscale(quality) {
  const q = Number.isFinite(quality) ? Math.round(31 - (quality / 100) * 29) : 8
  return Math.max(2, Math.min(31, q))
}

function createFfmpegMjpegEncoder({ width, height, fps, maxWidth, quality, onFrame }) {
  const args = ['-hide_banner', '-loglevel', 'error', '-nostdin']
  args.push('-f', 'rawvideo', '-pix_fmt', 'rgba', '-s', `${width}x${height}`, '-r', String(fps), '-i', 'pipe:0')
  if (maxWidth > 0 && width > maxWidth) {
    const outH = Math.max(1, Math.round((height * maxWidth) / width))
    args.push('-vf', `scale=${maxWidth}:${outH}:flags=fast_bilinear`)
  }
  args.push('-an', '-sn', '-dn')
  args.push('-c:v', 'mjpeg', '-q:v', String(qualityToQscale(quality)))
  args.push('-f', 'mjpeg', 'pipe:1')

  const proc = spawn(CAP_FFMPEG, args, { windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] })
  let buf = Buffer.alloc(0)
  let closed = false
  let lastStderr = ''

  function tryParse() {
    while (true) {
      const start = buf.indexOf(Buffer.from([0xff, 0xd8]))
      if (start < 0) {
        if (buf.length > 2) buf = buf.subarray(buf.length - 2)
        return
      }
      const end = buf.indexOf(Buffer.from([0xff, 0xd9]), start + 2)
      if (end < 0) {
        if (start > 0) buf = buf.subarray(start)
        return
      }
      const frame = buf.subarray(start, end + 2)
      buf = buf.subarray(end + 2)
      onFrame(frame)
    }
  }

  proc.stdout.on('data', (chunk) => {
    if (closed) return
    buf = buf.length === 0 ? chunk : Buffer.concat([buf, chunk], buf.length + chunk.length)
    if (buf.length > 20 * 1024 * 1024) {
      buf = buf.subarray(buf.length - 2)
    }
    tryParse()
  })

  proc.stderr.on('data', (chunk) => {
    lastStderr = String(chunk)
  })

  proc.on('close', () => {
    closed = true
    if (lastStderr) {
      console.error('ffmpeg closed:', lastStderr.trim())
    }
  })

  proc.on('error', (e) => {
    closed = true
    console.error('ffmpeg spawn error:', e)
  })

  return {
    writeFrame(rgba) {
      if (closed || proc.stdin.destroyed) return null
      return proc.stdin.write(rgba)
    },
    onDrain(fn) {
      proc.stdin.on('drain', fn)
    },
    close() {
      closed = true
      try {
        proc.stdin.end()
      } catch {}
      try {
        proc.kill('SIGKILL')
      } catch {}
    },
  }
}

function broadcastJpeg(jpegBuffer) {
  encodedFrames++
  for (const ws of clients) {
    if (ws.readyState !== WebSocket.OPEN) continue
    if (ws.bufferedAmount > 2 * 1024 * 1024) continue
    ws.send(jpegBuffer, { binary: true, compress: false })
    broadcastFrames++
  }
}

function startSharedCapture() {
  capture = new ScreenCapture(
    async (frame) => {
      if (clients.size === 0) return

      if (CAP_ENCODER === 'ffmpeg') {
        if (!encoder || encoderW !== frame.width || encoderH !== frame.height) {
          stopEncoder()
          try {
            encoderW = frame.width
            encoderH = frame.height
            encoder = createFfmpegMjpegEncoder({
              width: frame.width,
              height: frame.height,
              fps: CAP_FPS,
              maxWidth: MAX_WIDTH,
              quality: JPEG_QUALITY,
              onFrame: broadcastJpeg,
            })
            encoder.onDrain(() => {
              encoderBackpressure = false
            })
          } catch (e) {
            console.error('ffmpeg encoder init failed, falling back to sharp:', e)
            stopEncoder()
          }
        }

        if (encoder && !encoderBackpressure) {
          const ok = encoder.writeFrame(frame.rgba)
          if (ok === true) return
          if (ok === false) {
            encoderBackpressure = true
            return
          }
          stopEncoder()
        }
      }

      if (isProcessing) return
      isProcessing = true
      try {
        const sharpLib = getSharp()
        if (!sharpLib) {
          return
        }

        let pipeline = sharpLib(frame.rgba, {
          raw: {
            width: frame.width,
            height: frame.height,
            channels: 4,
          },
        })

        if (MAX_WIDTH > 0 && frame.width > MAX_WIDTH) {
          pipeline = pipeline.resize({
            width: MAX_WIDTH,
            height: Math.round((frame.height * MAX_WIDTH) / frame.width),
            fit: 'fill',
            kernel: 'nearest',
            fastShrinkOnLoad: true,
          })
        }

        const jpegBuffer = await pipeline
          .jpeg({
            quality: JPEG_QUALITY,
            mozjpeg: process.platform !== 'win32',
            progressive: false,
            optimiseScans: false,
            trellisQuantisation: false,
            overshootDeringing: false,
            optimiseCoding: false,
          })
          .toBuffer()

        broadcastJpeg(jpegBuffer)
      } catch (err) {
        console.error('Frame processing error:', err)
      } finally {
        isProcessing = false
      }
    },
    { fps: CAP_FPS },
  )

  capture
    .start()
    .then(() => {
      console.log('Screen capture started')
    })
    .catch((err) => {
      console.error('Failed to start capture:', err)
      stopSharedCapture()
      for (const ws of clients) {
        try {
          ws.close()
        } catch {}
      }
      clients.clear()
    })
}
