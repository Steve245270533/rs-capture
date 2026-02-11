let canvas = null
let ctx = null
let lastBitmap = null
let decoding = false
let pending = null

async function decodeAndDraw(buf) {
  if (!ctx || !canvas) return
  const blob = new Blob([buf], { type: 'image/jpeg' })
  const bitmap = await createImageBitmap(blob)

  if (lastBitmap) lastBitmap.close()
  lastBitmap = bitmap

  if (canvas.width !== bitmap.width || canvas.height !== bitmap.height) {
    canvas.width = bitmap.width
    canvas.height = bitmap.height
  }

  ctx.drawImage(bitmap, 0, 0)
}

self.onmessage = (event) => {
  const msg = event.data
  if (!msg || typeof msg !== 'object') return

  if (msg.type === 'init') {
    canvas = msg.canvas
    if (!canvas) return
    ctx = canvas.getContext('2d', { alpha: false, desynchronized: true })
    return
  }

  if (msg.type === 'frame') {
    const buf = msg.data
    if (!(buf instanceof ArrayBuffer)) return
    pending = buf
    if (decoding) return
    decoding = true
    ;(async () => {
      try {
        while (pending) {
          const next = pending
          pending = null
          await decodeAndDraw(next)
          postMessage({ type: 'frame' })
        }
      } catch (e) {
        postMessage({ type: 'error', message: String(e?.message ?? e) })
      } finally {
        decoding = false
      }
    })()
  }
}
