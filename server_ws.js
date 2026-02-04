const express = require('express')
const WebSocket = require('ws')
const http = require('http')
const path = require('path')
const sharp = require('sharp')
const robot = require('robotjs')
const { ScreenCapture } = require('./index')

const app = express()
const server = http.createServer(app)
const wss = new WebSocket.Server({ server })

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
      { fps: 120, backend: 'ScreenCaptureKit' },
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

  ws.on('message', (message) => {
    try {
      const event = JSON.parse(message)
      handleInputEvent(event)
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

function handleInputEvent(event) {
  try {
    const { type, x, y, button, key, modifiers } = event

    switch (type) {
      case 'mousemove':
        robot.moveMouse(x, y)
        break
      case 'mousedown':
        robot.mouseToggle('down', button || 'left')
        break
      case 'mouseup':
        robot.mouseToggle('up', button || 'left')
        break
      case 'click':
        robot.mouseClick(button || 'left')
        break
      case 'dblclick':
        robot.mouseClick(button || 'left', true) // true = double click
        break
      case 'keydown':
        if (key) robot.keyToggle(key, 'down', modifiers || [])
        break
      case 'keyup':
        if (key) robot.keyToggle(key, 'up', modifiers || [])
        break
      case 'keypress':
        if (key) robot.keyTap(key, modifiers || [])
        break
      case 'scroll':
        // robotjs doesn't support smooth scroll well, but we can try
        // robot.scrollMouse(x, y)
        break
    }
  } catch (err) {
    console.error('RobotJS error:', err)
  }
}

server.listen(PORT, () => {
  console.log(`Server running at http://localhost:${PORT}`)
})
