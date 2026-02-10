const express = require('express')
const http = require('http')
const { Server } = require('socket.io')
const { RTCPeerConnection, RTCVideoSource, nonstandard } = require('@roamhq/wrtc')
const { ScreenCapture } = require('@vertfrag/rs-capture')

const app = express()
const server = http.createServer(app)
const io = new Server(server)

const path = require('path')

app.use(express.static(path.join(__dirname, 'public')))

let capture = null
let videoSource = null
let track = null
let connections = new Set()

function startCapture() {
  if (capture) return

  console.log('Starting ScreenCaptureKit...')

  // Create a non-standard RTCVideoSource from node-webrtc
  videoSource = new nonstandard.RTCVideoSource()
  track = videoSource.createTrack()

  capture = new ScreenCapture(
    (frame) => {
      // frame: { width, height, stride, rgba: Buffer }
      // wrtc expects I420 data. We need to convert RGBA to I420.
      // However, doing this in JS is slow.
      // For this demo, we will try to push frame data if we can convert it,
      // or use a simpler approach if available.

      // RTCVideoSource.onFrame expects:
      // { width, height, data: Uint8ClampedArray (I420), rotation? }
      //
      // Since we don't have a fast RGBA->I420 converter in JS and doing it here
      // would block the event loop, we will use a very naive (and slow) conversion
      // or just grey-scale for demo if color is too heavy.
      //
      // Actually, let's try a basic RGB->YUV conversion.

      if (!videoSource) {
        return
      }

      const { width, height, rgba } = frame

      const i420Data = rgbaToI420(width, height, rgba)

      videoSource.onFrame({
        width,
        height,
        data: new Uint8ClampedArray(i420Data),
        rotation: 0,
      })
    },
    { fps: 60, backend: 'ScreenCaptureKit' },
  )

  capture.start()
}

function stopCapture() {
  if (connections.size === 0 && capture) {
    console.log('Stopping capture...')
    capture.stop()
    capture = null
    if (track) {
      track.stop()
      track = null
    }
    videoSource = null
  }
}

// Naive RGBA to I420 converter (Very slow in JS!)
// Ideally this should be done in Rust/C++
function rgbaToI420(width, height, rgba) {
  const ySize = width * height
  const uvSize = (width / 2) * (height / 2)
  const i420 = new Uint8Array(ySize + uvSize * 2)

  let yIndex = 0
  let uIndex = ySize
  let vIndex = ySize + uvSize

  for (let row = 0; row < height; row++) {
    for (let col = 0; col < width; col++) {
      const p = (row * width + col) * 4
      const r = rgba[p]
      const g = rgba[p + 1]
      const b = rgba[p + 2]

      // Y = 0.299R + 0.587G + 0.114B
      let y = 0.299 * r + 0.587 * g + 0.114 * b
      i420[yIndex++] = y

      // Subsample UV (2x2 block)
      if (row % 2 === 0 && col % 2 === 0) {
        // U = -0.169R - 0.331G + 0.500B + 128
        // V = 0.500R - 0.419G - 0.081B + 128
        let u = -0.169 * r - 0.331 * g + 0.5 * b + 128
        let v = 0.5 * r - 0.419 * g - 0.081 * b + 128

        i420[uIndex++] = u
        i420[vIndex++] = v
      }
    }
  }
  return i420
}

io.on('connection', async (socket) => {
  console.log('Client connected:', socket.id)
  connections.add(socket.id)

  startCapture()

  const pc = new RTCPeerConnection({
    iceServers: [{ urls: 'stun:stun.l.google.com:19302' }],
  })

  if (track) {
    pc.addTrack(track)
  } else {
    // If track is not ready yet, wait for it or handle it
    // For this demo, we assume startCapture() initializes it quickly
    // But let's check if we need to add it later
    const checkTrack = setInterval(() => {
      if (track) {
        try {
          pc.addTrack(track)
          clearInterval(checkTrack)
        } catch (e) {
          console.error('Error adding track:', e)
        }
      }
    }, 100)
  }

  pc.onicecandidate = (event) => {
    if (event.candidate) {
      socket.emit('candidate', event.candidate)
    }
  }

  pc.onnegotiationneeded = async () => {
    try {
      if (pc.signalingState !== 'stable') {
        console.log('Negotiation needed but state is not stable, ignoring')
        return
      }
      console.log('Negotiation needed - Creating offer')
      const offer = await pc.createOffer()
      await pc.setLocalDescription(offer)
      socket.emit('offer', offer)
    } catch (err) {
      console.error('Negotiation error:', err)
    }
  }

  pc.onconnectionstatechange = () => {
    console.log(`PC ${socket.id} state:`, pc.connectionState)
  }

  socket.on('offer', async (offer) => {
    // If client sends offer, we answer
    // But usually server sends offer for streaming
    await pc.setRemoteDescription(offer)
    const answer = await pc.createAnswer()
    await pc.setLocalDescription(answer)
    socket.emit('answer', answer)
  })

  socket.on('answer', async (answer) => {
    try {
      if (pc.signalingState === 'have-local-offer') {
        await pc.setRemoteDescription(answer)
      } else {
        console.warn('Received answer but state is not have-local-offer:', pc.signalingState)
      }
    } catch (err) {
      console.error('Error setting remote description:', err)
    }
  })

  socket.on('candidate', async (candidate) => {
    await pc.addIceCandidate(candidate)
  })

  socket.on('disconnect', () => {
    console.log('Client disconnected:', socket.id)
    pc.close()
    connections.delete(socket.id)
    stopCapture()
  })
})

const PORT = 3000
server.listen(PORT, () => {
  console.log(`WebRTC Server running at http://localhost:${PORT}/webrtc.html`)
})
