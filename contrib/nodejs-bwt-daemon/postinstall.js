#!/usr/bin/env node

const https = require('https')
    , fs = require('fs')
    , path = require('path')
    , crypto = require('crypto')
    , tar = require('tar')

const getUrl = (version, dist_name) => `https://github.com/bwt-dev/libbwt/releases/download/v${version}/${dist_name}.tar.gz`
const getDistName = (version, platform, variant) => `libbwt-${version}-${variant?variant+'-':''}${platform}`

if (!process.env.BWT_NO_DOWNLOAD) {
  download(require('./package.json').version, process.env.BWT_VARIANT)
    .catch(err => console.error(`[bwt] Failed! ${err}`))
}

// Fetch the shared library file from GitHub releases and verify its hash
async function download(version, variant=null) {
   const { platform, libname } = getPlatform()
       , dist_name = getDistName(version, platform, variant)
       , release_url = getUrl(version, dist_name)
       , temp_path = path.join(__dirname, `__${dist_name}__temp${Math.random()*99999|0}.tar.gz`)
       , target = path.join(__dirname, libname)
       , hash = getExpectedHash(dist_name)

  if (fs.existsSync(target) && !process.env.BWT_FORCE && !process.env.BWT_VARIANT) {
    console.error(`[bwt] Skipping download, ${target} found`)
    return
  }

  console.log(`[bwt] Downloading ${dist_name} from ${release_url}`)
  const actual_hash = await getFile(release_url, temp_path)

  if (actual_hash != hash) {
    throw new Error(`Hash mismatch for ${temp_path} downloaded from ${release_url}, expected ${hash}, found ${actual_hash}`)
  }
  console.log(`[bwt] Verified SHA256(${dist_name}.tar.gz) == ${hash}`)

  await tar.extract({ file: temp_path, cwd: __dirname, strip: 1 }, [ `${dist_name}/${libname}` ])
  console.log(`[bwt] Extracted to ${path.join(__dirname, libname)}`)

  fs.unlinkSync(temp_path)
}

// Download the file while stream-hashing its contents
async function getFile(url, dest) {
  return new Promise((resolve, reject) => {
    const options = { headers: { 'user-agent': 'bwt-daemon' } }
    https.get(url, options, resp => {
      if (resp.statusCode == 302) {
        return getFile(resp.headers.location, dest).then(resolve, reject)
      } else if (resp.statusCode != 200) {
        return reject(new Error(`Invalid status code ${resp.statusCode} while downloading bwt`))
      }
      const hasher = crypto.createHash('sha256')

      resp.on('data', d => hasher.update(d))
        .pipe(fs.createWriteStream(dest)
           .on('finish', () => resolve(hasher.digest('hex')))
           .on('error', reject)
         )
    }).on('error', err => {
      reject(`Error while downloading bwt: ${err}`)
    })
  })
}

// Read the expected sha256 hash out of the SHA256SUMS file
function getExpectedHash(dist_name) {
  let line = fs.readFileSync(path.join(__dirname, 'SHA256SUMS'))
    .toString()
    .split('\n')
    .find(line => line.endsWith(` ${dist_name}.tar.gz`))
  if (!line) throw new Error(`Cannot find ${dist_name} in SHA256SUMS`)
  return line.split(' ')[0]
}

function getPlatform() {
  const libname = `libbwt.${{'linux':  'so', 'darwin': 'dylib', 'win32':  'dll'}[process.platform]}`
  const arch = process.env.npm_config_arch || require('os').arch()
  const platform = {
    'x64-darwin': 'x86_64-osx'
  , 'x64-win32': 'x86_64-windows'
  , 'x64-linux': 'x86_64-linux'
  , 'arm-linux': 'arm32v7-linux'
  , 'arm64-linux': 'arm64v8-linux'
  }[`${arch}-${process.platform}`]
  if (!platform) throw new Error(`Unsuppported platform: ${arch}-${process.platform}`)

  return { platform, libname }
}
