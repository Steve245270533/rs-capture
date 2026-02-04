const fs = require('fs')
const path = require('path')

const pkg = require('../package.json')
const { name, version, napi } = pkg
const { targets, binaryName } = napi

const platformMapping = {
  'x86_64-pc-windows-msvc': { os: 'win32', cpu: 'x64', platform: 'win32-x64-msvc' },
  'aarch64-pc-windows-msvc': { os: 'win32', cpu: 'arm64', platform: 'win32-arm64-msvc' },
  'x86_64-apple-darwin': { os: 'darwin', cpu: 'x64', platform: 'darwin-x64' },
  'aarch64-apple-darwin': { os: 'darwin', cpu: 'arm64', platform: 'darwin-arm64' },
  'x86_64-unknown-linux-gnu': { os: 'linux', cpu: 'x64', libc: 'glibc', platform: 'linux-x64-gnu' },
}

const npmDir = path.join(__dirname, '../npm')
if (fs.existsSync(npmDir)) {
  fs.rmSync(npmDir, { recursive: true, force: true })
}
fs.mkdirSync(npmDir)

targets.forEach((target) => {
  const info = platformMapping[target]
  if (!info) {
    console.error(`Unknown target: ${target}`)
    return
  }

  const targetDir = path.join(npmDir, info.platform)
  if (!fs.existsSync(targetDir)) {
    fs.mkdirSync(targetDir, { recursive: true })
  }

  const pkgJson = {
    name: `${name}-${info.platform}`,
    version: version,
    os: [info.os],
    cpu: [info.cpu],
    main: `${binaryName}.${info.platform}.node`,
    files: [`${binaryName}.${info.platform}.node`],
    license: pkg.license,
    engines: pkg.engines,
  }

  if (info.libc) {
    pkgJson.libc = [info.libc]
  }

  fs.writeFileSync(path.join(targetDir, 'package.json'), JSON.stringify(pkgJson, null, 2))
  fs.writeFileSync(
    path.join(targetDir, 'README.md'),
    `# ${pkgJson.name}\n\nThis is the ${info.platform} binary for ${name}.`,
  )

  console.log(`Created ${info.platform}`)
})
