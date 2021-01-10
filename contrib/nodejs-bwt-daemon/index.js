const ffi = require('ffi-napi')
    , ref = require('ref-napi')
    , path = require('path')
    , EventEmitter = require('events')
    , debug = require('debug')('bwt-daemon')

const LIB_PATH = process.env.BWT_LIB || path.join(__dirname, 'libbwt')

// Low-level private API

const OK = 0

const shutdownPtr = ref.refType('void')

const libbwt = ffi.Library(LIB_PATH, {
  bwt_start: [ 'int', [ 'string', 'pointer', 'pointer' ] ]
, bwt_shutdown: [ 'int', [ shutdownPtr ] ]
})

function bwt_start(options, init_cb, notify_cb, done) {
  const opt_json = JSON.stringify(options)
      , init_cb_ffi = ffi.Callback('void', [ shutdownPtr ], init_cb)
      , notify_cb_ffi = ffi.Callback('void', [ 'string', 'float', 'uint32', 'string' ], notify_cb)

  libbwt.bwt_start.async(opt_json, init_cb_ffi, notify_cb_ffi, function(err, code) {
    debug('stopped with', { err, code })
    if (err) return done(err)
    if (code != OK) return done(new Error(`bwt failed with code ${code}`))
    done(null)
  })
}

function bwt_shutdown(shutdown_ptr) {
  const code = libbwt.bwt_shutdown(shutdown_ptr)
  shutdown_ptr.deref()
  if (code != OK) throw new Error(`bwt shutdown failed with code ${code}`)
}

// High-level public API

class BwtDaemon extends EventEmitter {
  constructor(options) {
    super()

    if (options.progress) this.on('progress', take_prop(options, 'progress'))
    this.options = normalize_options(options)

    this.ready = new Promise((resolve, reject) => {
      this.on('ready', _ => resolve(this))
      this.on('error', reject)
      this.on('exit', err => {
        if (err) return this.emit('error', err)
        // this `reject` will be ignored if the daemon already started up successfully
        // and the promise was resolved, which can happen following a shutdown() call.
        // imagine this is wrapped in an `if (!promise.resolved)`.
        reject(new Error('daemon stopped while starting up'))
      })
    })
  }

  start() {
    if (this._started) throw new Error('daemon already started')
    this._started = true

    debug('starting with %O', { ...this.options, bitcoind_auth: '**SCRUBBED**' });
    bwt_start(this.options, this._init.bind(this), this._notify.bind(this)
            , err => this.emit('exit', err /*null for successful exit*/))
    return this.ready
  }

  shutdown() {
    debug('shutdown', this._shutdown_ptr != null)
    // we cannot shut down yet, mark for later termination
    if (!this._shutdown_ptr) return this._terminate = true

    bwt_shutdown(take_prop(this, '_shutdown_ptr'))
  }

  _init(shutdown_ptr) {
    this._shutdown_ptr = shutdown_ptr
    if (take_prop(this, '_terminate')) this.shutdown()
  }

  _notify(msg_type, progress_n, detail_n, detail_s) {
    debug('notify %s %s %s', msg_type, progress_n, detail_n, detail_s)
    switch (msg_type) {
      case 'error':
        this.emit('error', detail_s)
        break
      case 'progress:sync':
        this.emit('progress', 'sync', progress_n, { tip_time: detail_n })
        break
      case 'progress:scan':
        this.emit('progress', 'scan', progress_n, { eta: detail_n })
        break
      case 'ready:http':
			  this.http_addr = detail_s
			  this.http_url = `http://${this.http_addr}/`
				break
      case 'ready:electrum':
			  this.electrum_addr = detail_s
        break
      case 'ready':
        this.emit('ready')
        break
      default: debug('unknown msg', msg_type)
    }
  }
}

// optional 'new'
exports = module.exports = function(opt) { return new BwtDaemon(opt) }
exports.BwtDaemon = BwtDaemon
exports.start = BwtDaemon.start = opt => new BwtDaemon(opt).start()

// Utility

function normalize_options(options) {
  if (options.rescan_since) {
    options.rescan_since = parse_timestamp(options.rescan_since)
  }
  if (take_prop(options, 'electrum') && !options.electrum_addr) {
    options.electrum_addr = '127.0.0.1:0'
  }
  if (take_prop(options, 'http') && !options.http_addr) {
    options.http_addr = '127.0.0.1:0'
  }

  if (!options.electrum_addr && !options.http_addr) {
    throw new Error('None of the bwt services are enabled')
  }

  // Delete nully options so that they get their default value
  Object.entries(options)
    .filter(([ _, val ]) => val == null)
    .forEach(([ key, _ ]) => delete options[key])

  return options
}

function take_prop(obj, key) {
  const val = obj[key]
  delete obj[key]
  return val
}

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
