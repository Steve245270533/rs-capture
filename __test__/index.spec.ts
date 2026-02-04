import test from 'ava'
import { ScreenCapture } from '../index.js'

test('ScreenCapture: init', (t) => {
  const capturer = new ScreenCapture(() => {})
  t.truthy(capturer)
  t.is(typeof capturer.start, 'function')
  t.is(typeof capturer.stop, 'function')
})
