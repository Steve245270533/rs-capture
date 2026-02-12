let canvas = null
let ctx = null
let frameCount = 0
let lastTime = performance.now()

self.onmessage = async (event) => {
  const msg = event.data
  if (!msg || typeof msg !== 'object') return

  if (msg.type === 'init') {
    canvas = msg.canvas
    ctx = canvas.getContext('2d', {
      alpha: false,
      desynchronized: true,
      willReadFrequently: false,
    })
    return
  }

  if (msg.type === 'stream') {
    const reader = msg.readable.getReader()

    // Recursive draw to stay in sync with monitor refresh if possible
    async function processFrames() {
      while (true) {
        const { done, value: frame } = await reader.read()
        if (done) break

        if (canvas && ctx) {
          if (canvas.width !== frame.displayWidth || canvas.height !== frame.displayHeight) {
            canvas.width = frame.displayWidth
            canvas.height = frame.displayHeight
          }

          // Using drawImage with VideoFrame is highly efficient
          ctx.drawImage(frame, 0, 0)
          frameCount++

          const now = performance.now()
          if (now - lastTime >= 1000) {
            const fps = Math.round((frameCount * 1000) / (now - lastTime))
            postMessage({ type: 'fps', value: fps })
            frameCount = 0
            lastTime = now
          }
        }
        frame.close()
      }
    }

    processFrames().catch((err) => {
      postMessage({ type: 'error', value: err.message })
    })
  }
}
