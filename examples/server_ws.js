const express = require('express')
const WebSocket = require('ws')
const http = require('http')
const path = require('path')
const sharp = require('sharp')
const { mouse, keyboard, Button, Key, Point } = require('@nut-tree/nut-js')
const { ScreenCapture } = require('@vertfrag/rs-capture')

const app = express()
const server = http.createServer(app)
const wss = new WebSocket.Server({ server })

// Configure nut-js
mouse.config.autoDelayMs = 0
keyboard.config.autoDelayMs = 0

// Serve static files from 'public' directory
app.use(express.static(path.join(__dirname, 'public')))

wss.on('connection', (ws) => {
  console.log('Client connected')

  let capture = null
  let isProcessing = false

  try {
    capture = new ScreenCapture(
      async (frame) => {
        // Drop frame if previous one is still processing to avoid backpressure
        if (isProcessing) return
        if (ws.readyState !== WebSocket.OPEN) return

        isProcessing = true
        try {
          // Compress RGBA to JPEG
          const jpegBuffer = await sharp(frame.rgba, {
            raw: {
              width: frame.width,
              height: frame.height,
              channels: 4,
            },
          })
            .jpeg({ quality: 50, mozjpeg: true }) // Optimize for speed/size
            .toBuffer()

          if (ws.readyState === WebSocket.OPEN) {
            ws.send(jpegBuffer)
          }
        } catch (err) {
          console.error('Frame processing error:', err)
        } finally {
          isProcessing = false
        }
      },
      { fps: 60, backend: 'ScreenCaptureKit' },
    )

    capture
      .start()
      .then(() => {
        console.log('Screen capture started')
      })
      .catch((err) => {
        console.error('Failed to start capture:', err)
        ws.close()
      })
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
    if (capture) {
      capture.stop()
      capture = null
    }
  })

  ws.on('error', (err) => {
    console.error('WebSocket error:', err)
    if (capture) {
      capture.stop()
      capture = null
    }
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
