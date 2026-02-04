const { ScreenCapture } = require('./index')
const fs = require('fs')
const path = require('path')
const sharp = require('sharp')

// Create snapshot directory
const snapshotDir = path.join(__dirname, 'snapshot_sck')
if (!fs.existsSync(snapshotDir)) {
  fs.mkdirSync(snapshotDir)
}

let frameCount = 0
const MAX_FRAMES = 10

console.log('Creating capture with ScreenCaptureKit backend...')
try {
  // Pass config to select ScreenCaptureKit
  const capture = new ScreenCapture(
    async (frame) => {
      if (frameCount < MAX_FRAMES) {
        const currentFrame = frameCount++
        const fileName = `frame-${Date.now()}-${currentFrame}.png`
        const filePath = path.join(snapshotDir, fileName)

        try {
          await sharp(frame.rgba, {
            raw: {
              width: frame.width,
              height: frame.height,
              channels: 4,
            },
          }).toFile(filePath)

          console.log(`Saved ${fileName}`)
        } catch (err) {
          console.error(`Error saving frame ${currentFrame}:`, err)
        }
      }
    },
    { backend: 'ScreenCaptureKit' },
  )

  console.log('Starting capture...')
  capture
    .start()
    .then(() => {
      console.log('Capture started')

      const checkInterval = setInterval(() => {
        if (frameCount >= MAX_FRAMES) {
          console.log('Captured max frames, stopping...')
          clearInterval(checkInterval)
          capture.stop()
          console.log('Stopped')
          process.exit(0)
        }
      }, 100)

      setTimeout(() => {
        if (frameCount < MAX_FRAMES) {
          console.log('Timeout, stopping...')
          capture.stop()
          process.exit(0)
        }
      }, 5000)
    })
    .catch((e) => {
      console.error('Start failed:', e)
    })
} catch (e) {
  console.error('Initialization failed:', e)
}
