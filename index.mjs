import { createRequire } from 'module'
const require = createRequire(import.meta.url)
const { CaptureBackend, ScreenCapture } = require('./index.js')

export { CaptureBackend, ScreenCapture }
