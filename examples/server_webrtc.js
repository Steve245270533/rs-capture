const express = require('express')
const http = require('http')
const os = require('os')
const { spawn } = require('child_process')
const path = require('path')
const { Server } = require('socket.io')
const { RTCPeerConnection, RTCVideoSource, nonstandard } = require('@roamhq/wrtc')
const { ScreenCapture } = require('@vertfrag/rs-capture')

const app = express()
const server = http.createServer(app)
const io = new Server(server)

const MAX_WIDTH = Number.parseInt(process.env.CAP_MAX_WIDTH ?? (process.platform === 'win32' ? '1280' : '0'), 10)
const CAP_FPS = Number.parseInt(process.env.CAP_FPS ?? '60', 10)
const CAP_ENCODER = (process.env.CAP_ENCODER ?? (process.platform === 'win32' ? 'ffmpeg' : 'sharp')).toLowerCase()
const CAP_FFMPEG_PATH = process.env.CAP_FFMPEG_PATH ?? null
let DEFAULT_FFMPEG_PATH = null
try {
  DEFAULT_FFMPEG_PATH = require('@ffmpeg-installer/ffmpeg').path
} catch {}
const CAP_FFMPEG = CAP_FFMPEG_PATH || DEFAULT_FFMPEG_PATH || 'ffmpeg'

// High quality bitrate settings for WebRTC (in kbps)
const WEBRTC_BITRATE = Number.parseInt(process.env.WEBRTC_BITRATE ?? '15000', 10)

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

console.log(
  `WebRTC encoder config: encoder=${CAP_ENCODER} fps=${CAP_FPS} maxWidth=${MAX_WIDTH} bitrate=${WEBRTC_BITRATE}kbps`,
)

app.use(express.static(path.join(__dirname, 'public')))

let capture = null
let videoSource = null
let track = null
let connections = new Set()
let encoder = null
let encoderW = 0
let encoderH = 0
let encoderBackpressure = false
let isProcessing = false

let encodedFrames = 0
let broadcastFrames = 0
let lastStatsAt = Date.now()

setInterval(() => {
  const now = Date.now()
  const dt = (now - lastStatsAt) / 1000
  lastStatsAt = now
  const encFps = Math.round(encodedFrames / dt)
  const outFps = Math.round(broadcastFrames / dt)
  encodedFrames = 0
  broadcastFrames = 0
  if (connections.size > 0) {
    console.log(
      `WebRTC stats: clients=${connections.size} enc_fps=${encFps} out_fps=${outFps} encoder=${encoder ? CAP_ENCODER : isProcessing ? 'sharp' : 'none'}`,
    )
  }
}, 1000).unref()

function createFfmpegI420Encoder({ width, height, fps, maxWidth, onFrame }) {
  const args = ['-hide_banner', '-loglevel', 'error', '-nostdin']
  args.push('-f', 'rawvideo', '-pix_fmt', 'rgba', '-s', `${width}x${height}`, '-r', String(fps), '-i', 'pipe:0')

  if (maxWidth > 0 && width > maxWidth) {
    const outH = Math.max(1, Math.round((height * maxWidth) / width))
    args.push('-vf', `scale=${maxWidth}:${outH}:flags=fast_bilinear`)
  }

  args.push('-an', '-sn', '-dn')
  args.push('-f', 'rawvideo', '-pix_fmt', 'yuv420p', 'pipe:1')

  const proc = spawn(CAP_FFMPEG, args, { windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] })

  const targetW = maxWidth > 0 && width > maxWidth ? maxWidth : width
  const targetH = maxWidth > 0 && width > maxWidth ? Math.max(1, Math.round((height * maxWidth) / width)) : height
  const frameSize = targetW * targetH + (targetW * targetH) / 2

  let buf = Buffer.alloc(0)
  let closed = false

  proc.stdout.on('data', (chunk) => {
    if (closed) return
    buf = Buffer.concat([buf, chunk])
    while (buf.length >= frameSize) {
      const frame = buf.subarray(0, frameSize)
      buf = buf.subarray(frameSize)
      onFrame(frame, targetW, targetH)
    }
  })

  proc.stderr.on('data', (chunk) => {
    console.error('ffmpeg stderr:', String(chunk))
  })

  proc.on('close', () => {
    closed = true
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

/**
 * Optimized RGBA to I420 converter using integer math
 */
function rgbaToI420(width, height, rgba) {
  const ySize = width * height
  const uvSize = (width >> 1) * (height >> 1)
  const i420 = new Uint8Array(ySize + uvSize * 2)

  const yPlane = i420.subarray(0, ySize)
  const uPlane = i420.subarray(ySize, ySize + uvSize)
  const vPlane = i420.subarray(ySize + uvSize, ySize + uvSize * 2)

  let yIdx = 0
  let uvIdx = 0

  for (let row = 0; row < height; row++) {
    const rowOffset = row * width * 4
    const isUvRow = (row & 1) === 0

    for (let col = 0; col < width; col++) {
      const p = rowOffset + (col << 2)
      const r = rgba[p]
      const g = rgba[p + 1]
      const b = rgba[p + 2]

      // Y = 0.299R + 0.587G + 0.114B
      yPlane[yIdx++] = (r * 77 + g * 150 + b * 29) >> 8

      if (isUvRow && (col & 1) === 0) {
        // U = -0.169R - 0.331G + 0.500B + 128
        // V = 0.500R - 0.419G - 0.081B + 128
        uPlane[uvIdx] = ((-r * 43 - g * 84 + b * 127) >> 8) + 128
        vPlane[uvIdx] = ((r * 127 - g * 106 - b * 21) >> 8) + 128
        uvIdx++
      }
    }
  }
  return i420
}

function pushWebRTCFrame(i420Data, w, h) {
  encodedFrames++
  if (videoSource) {
    videoSource.onFrame({
      width: w,
      height: h,
      data: new Uint8ClampedArray(i420Data),
      rotation: 0,
    })
    broadcastFrames++
  }
}

/**
 * SDP Munging to increase bitrate
 */
function mungeSdp(sdp, bitrate) {
  // Add b=AS and x-google-max-bitrate
  let lines = sdp.split('\r\n')
  const mVideoIndex = lines.findIndex((line) => line.startsWith('m=video'))

  if (mVideoIndex !== -1) {
    // Insert b=AS line
    lines.splice(mVideoIndex + 1, 0, `b=AS:${bitrate}`)

    // Find a=fmtp line and append bitrates
    for (let i = 0; i < lines.length; i++) {
      if (lines[i].startsWith('a=fmtp')) {
        lines[i] += `;x-google-min-bitrate=${bitrate};x-google-max-bitrate=${bitrate};x-google-start-bitrate=${bitrate}`
      }
    }
  }
  return lines.join('\r\n')
}

function startSharedCapture() {
  if (capture) return

  console.log('Starting Optimized Shared WebRTC ScreenCapture...')

  videoSource = new nonstandard.RTCVideoSource()
  track = videoSource.createTrack()
  // Set content hint for high quality
  if (track.setApparentResolution) {
    track.contentHint = 'detail' // Prioritize image quality over motion
  }

  capture = new ScreenCapture(
    async (frame) => {
      if (connections.size === 0) return

      if (CAP_ENCODER === 'ffmpeg') {
        if (!encoder || encoderW !== frame.width || encoderH !== frame.height) {
          if (encoder) encoder.close()
          encoderW = frame.width
          encoderH = frame.height
          encoder = createFfmpegI420Encoder({
            width: frame.width,
            height: frame.height,
            fps: CAP_FPS,
            maxWidth: MAX_WIDTH,
            onFrame: pushWebRTCFrame,
          })
          encoder.onDrain(() => {
            encoderBackpressure = false
          })
        }

        if (encoder && !encoderBackpressure) {
          const ok = encoder.writeFrame(frame.rgba)
          if (ok === false) {
            encoderBackpressure = true
          } else if (ok === null) {
            if (encoder) {
              encoder.close()
              encoder = null
            }
          }
        }
        return
      }

      // Sharp path
      if (isProcessing) return
      isProcessing = true
      try {
        const sharpLib = getSharp()
        if (!sharpLib) {
          const i420 = rgbaToI420(frame.width, frame.height, frame.rgba)
          pushWebRTCFrame(i420, frame.width, frame.height)
          return
        }

        let pipeline = sharpLib(frame.rgba, {
          raw: {
            width: frame.width,
            height: frame.height,
            channels: 4,
          },
        })

        let targetW = frame.width
        let targetH = frame.height

        if (MAX_WIDTH > 0 && frame.width > MAX_WIDTH) {
          targetW = MAX_WIDTH
          targetH = Math.round((frame.height * MAX_WIDTH) / frame.width)
          pipeline = pipeline.resize({
            width: targetW,
            height: targetH,
            fit: 'fill',
            kernel: 'nearest',
          })
        }

        const processedRgba = await pipeline.raw().toBuffer()
        const i420 = rgbaToI420(targetW, targetH, processedRgba)
        pushWebRTCFrame(i420, targetW, targetH)
      } catch (err) {
        console.error('WebRTC Frame processing error:', err)
      } finally {
        isProcessing = false
      }
    },
    { fps: CAP_FPS },
  )

  capture.start().catch((err) => {
    console.error('Failed to start WebRTC capture:', err)
    stopSharedCapture()
  })
}

function stopSharedCapture() {
  if (capture) {
    console.log('Stopping shared WebRTC capture...')
    capture.stop()
    capture = null
  }
  if (encoder) {
    encoder.close()
    encoder = null
  }
  if (track) {
    track.stop()
    track = null
  }
  videoSource = null
  encoderW = 0
  encoderH = 0
  encoderBackpressure = false
}

io.on('connection', async (socket) => {
  console.log('Client connected:', socket.id)
  connections.add(socket.id)

  startSharedCapture()

  const pc = new RTCPeerConnection({
    iceServers: [{ urls: 'stun:stun.l.google.com:19302' }],
  })

  const candidateQueue = []

  pc.onicecandidate = (event) => {
    if (event.candidate) {
      socket.emit('candidate', event.candidate)
    }
  }

  const negotiate = async () => {
    try {
      if (pc.signalingState !== 'stable') return
      let offer = await pc.createOffer()
      // Munge SDP to increase bitrate before setting local description
      offer.sdp = mungeSdp(offer.sdp, WEBRTC_BITRATE)
      await pc.setLocalDescription(offer)
      socket.emit('offer', offer)
    } catch (err) {
      console.error(`[${socket.id}] Negotiation error:`, err)
    }
  }

  pc.onnegotiationneeded = negotiate

  const checkTrack = setInterval(() => {
    if (track) {
      try {
        const senders = pc.getSenders()
        if (!senders.find((s) => s.track === track)) {
          pc.addTrack(track)
          if (pc.signalingState === 'stable') {
            negotiate()
          }
        }
        clearInterval(checkTrack)
      } catch (e) {
        console.error(`[${socket.id}] Error adding track:`, e)
      }
    }
  }, 100)

  pc.onconnectionstatechange = () => {
    console.log(`[${socket.id}] PC state: ${pc.connectionState}`)
  }

  socket.on('answer', async (answer) => {
    try {
      if (pc.signalingState === 'have-local-offer') {
        // Munge answer too if needed
        answer.sdp = mungeSdp(answer.sdp, WEBRTC_BITRATE)
        await pc.setRemoteDescription(answer)
        while (candidateQueue.length > 0) {
          const candidate = candidateQueue.shift()
          await pc.addIceCandidate(candidate)
        }
      }
    } catch (err) {
      console.error(`[${socket.id}] Error setting remote description:`, err)
    }
  })

  socket.on('candidate', async (candidate) => {
    try {
      if (pc.remoteDescription) {
        await pc.addIceCandidate(candidate)
      } else {
        candidateQueue.push(candidate)
      }
    } catch (e) {
      console.error(`[${socket.id}] Error adding ice candidate:`, e)
    }
  })

  socket.on('disconnect', () => {
    console.log('Client disconnected:', socket.id)
    clearInterval(checkTrack)
    pc.close()
    connections.delete(socket.id)
    if (connections.size === 0) {
      stopSharedCapture()
    }
  })
})

const PORT = process.env.PORT || 3000
server.listen(PORT, () => {
  console.log(`WebRTC Server running at http://localhost:${PORT}/webrtc.html`)
})
