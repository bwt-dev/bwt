const ffi = require('ffi-napi')
    , ref = require('ref-napi')
    , path = require('path')
    , debug = require('debug')('bwt-daemon')

const LIB_PATH = process.env.BWT_LIB || path.join(__dirname, 'libbwt')

// Low-level private API

const OK = 0

const shutdownPtr = ref.refType('void')
    , shutdownPtrPtr = ref.refType(shutdownPtr)

const libbwt = ffi.Library(LIB_PATH, {
  bwt_start: [ 'int', [ 'string', 'pointer', shutdownPtrPtr ] ]
, bwt_shutdown: [ 'int', [ shutdownPtr ] ]
})

function start_bwt(options, progress_cb, done) {
  const opt_json = JSON.stringify(options)
      , progress_cb_ffi = ffi.Callback('void', [ 'string', 'float', 'uint32', 'string' ], progress_cb)
      , shutdown_ptrptr = ref.alloc(shutdownPtrPtr)

  debug('starting with %O', { ...options, bitcoind_auth: '**SCRUBBED**' });
  libbwt.bwt_start.async(opt_json, progress_cb_ffi, shutdown_ptrptr, function(err, code) {
    if (err) return done(err)
    if (code != OK) return done(new Error(`bwt failed with code ${code}`))
    done(null, shutdown_ptrptr.deref())
  })
}

// High-level public API

function init(options) {
  return new Promise((resolve, reject) => {
    let opt_progress = null
    if (options.progress) {
      opt_progress = options.progress
      delete options.progress
    }

    if (options.rescan_since) {
      options.rescan_since = parse_timestamp(options.rescan_since)
    }

    // Convenience shortcuts
    if (options.electrum) {
      options.electrum_addr || (options.electrum_addr = '127.0.0.1:0')
      delete options.electrum
    }
    if (options.http) {
      options.http_addr || (options.http_addr = '127.0.0.1:0')
      delete options.http
    }

    // Delete nully options so that they get their default value
    Object.entries(options)
      .filter(([ _, val ]) => val == null)
      .forEach(([ key, _ ]) => delete options[key])

    if (!options.electrum_addr && !options.http_addr) {
      throw new Error('None of the bwt services are enabled')
    }

    const services = {}

    function progress_cb(msg_type, progress, detail_n, detail_s) {
      debug('%s %s %s', msg_type, progress, detail_n, detail_s)
      if (msg_type == 'error') {
        reject(new Error(detail_s))
      } else if (msg_type.startsWith('ready:')) {
        services[msg_type.substr(6)] = detail_s
      } else if (msg_type == 'progress:sync') {
        opt_progress && opt_progress('sync', progress, { tip_time: detail_n })
      } else if (msg_type == 'progress:scan') {
        opt_progress && opt_progress('scan', progress, { eta: detail_n })
      } else if (['booting', 'ready'].includes(msg_type)) {
        opt_progress && opt_progress(msg_type, progress, {})
      }
    }

    start_bwt(options, progress_cb, (err, shutdown_ptr) => {
      if (err) reject(err)
      else resolve(new BwtDaemon(services, shutdown_ptr))
    })
  })
}

class BwtDaemon {
  constructor(services, shutdown_ptr) {
    this.shutdown_ptr = shutdown_ptr
    Object.entries(services).forEach(([ name, addr ]) =>
      this[`${name}_addr`] = addr)

    if (this.http_addr) this.http_url = `http://${this.http_addr}/`
  }

  shutdown() {
    if (!this.shutdown_ptr) return;
    const code = libbwt.bwt_shutdown(this.shutdown_ptr)
    this.shutdown_ptr.deref()
    this.shutdown_ptr = null
    if (code != OK) throw new Error(`bwt shutdown failed with code ${code}`)
  }
}

module.exports = init.BwtDaemon = init

// Utility

function parse_timestamp(ts) {
  // Pass 'now' as is
  if (ts == 'now') return ts
  // Date objects
  if (ts.getTime) return ts.getTime()/1000|0
  // Unix timestamp
  if (!isNaN(ts)) return +ts
  // Date string (e.g. YYYY-MM-DD)
  const dt = new Date(ts)
  if (!isNaN(dt.getTime())) return dt.getTime()/1000|0

  throw new Error(`Invalid rescan since value: ${ts}`)
}
