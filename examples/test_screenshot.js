const { ScreenCapture } = require('@vertfrag/rs-capture')
const fs = require('fs')
const path = require('path')
const sharp = require('sharp')

async function main() {
  const dir = path.join(__dirname, 'snapshot_screenshot')
  if (!fs.existsSync(dir)) fs.mkdirSync(dir)
  const capturer = new ScreenCapture()
  const frame = await capturer.screenshot()
  const file = path.join(dir, `screenshot-${Date.now()}.png`)
  await sharp(frame.rgba, {
    raw: { width: frame.width, height: frame.height, channels: 4 },
  }).toFile(file)
  console.log(file)
}

main().catch((e) => {
  console.error(e)
  process.exit(1)
})
