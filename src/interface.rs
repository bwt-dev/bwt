#[cfg(feature = "ffi")]
mod ffi {
    use std::ffi::{CStr, CString};
    use std::os::raw::c_char;
    use std::sync::{mpsc, Once};
    use std::thread;

    use crate::util::bitcoincore_ext::Progress;
    use crate::{App, Config, Result};

    const OK: i32 = 0;
    const ERR: i32 = -1;

    type Callback = extern "C" fn(*const c_char, f32, *const c_char);

    #[repr(C)]
    pub struct ShutdownHandler(mpsc::SyncSender<()>);

    static INIT_LOGGER: Once = Once::new();

    /// Start bwt. Accepts the config as a json string, a callback function
    /// to receive status updates, and a pointer for the shutdown handler.
    ///
    /// This will locks the thread until the initial sync is completed, then spawn
    /// a background thread for continuous syncing and return a shutdown handler.
    #[no_mangle]
    pub extern "C" fn bwt_start(
        json_config: *const c_char,
        callback_fn: Callback,
        shutdown_out: *mut *const ShutdownHandler,
    ) -> i32 {
        let json_config = unsafe { CStr::from_ptr(json_config) }.to_str().unwrap();

        let start = || -> Result<_> {
            let config: Config = serde_json::from_str(json_config)?;
            if config.verbose > 0 {
                // The verbosity level cannot be changed once enabled.
                INIT_LOGGER.call_once(|| config.setup_logger());
            }

            let (progress_tx, progress_rx) = mpsc::channel();
            let _progress_thread = spawn_recv_progress_thread(progress_rx, callback_fn.clone());

            notify(callback_fn, "booting", 0.0, "");
            let app = App::boot(config, Some(progress_tx))?;

            // XXX progress_thread.join().unwrap();

            #[cfg(feature = "electrum")]
            if let Some(addr) = app.electrum_addr() {
                notify(callback_fn, "ready:electrum", 1.0, &addr.to_string());
            }
            #[cfg(feature = "http")]
            if let Some(addr) = app.http_addr() {
                notify(callback_fn, "ready:http", 1.0, &addr.to_string());
            }

            notify(callback_fn, "ready", 1.0, "");

            let shutdown_tx = app.sync_background();

            Ok(ShutdownHandler(shutdown_tx))
        };

        match start() {
            Ok(shutdown_handler) => unsafe {
                *shutdown_out = Box::into_raw(Box::new(shutdown_handler));
                OK
            },
            Err(e) => {
                warn!("{:?}", e);
                notify(callback_fn, "error", 0.0, &e.to_string());
                ERR
            }
        }
    }

    #[no_mangle]
    pub extern "C" fn bwt_shutdown(shutdown_ptr: *mut ShutdownHandler) -> i32 {
        assert!(!shutdown_ptr.is_null());
        unsafe {
            // Take ownership and drop it. This will disconnect the mpsc channel and shutdown the app.
            Box::from_raw(shutdown_ptr);
        }
        OK
    }

    fn notify(callback_fn: Callback, msg_type: &str, progress: f32, detail: &str) {
        callback_fn(cstring(msg_type), progress, cstring(detail))
    }

    fn cstring(s: &str) -> *const c_char {
        CString::new(s).unwrap().into_raw()
    }

    // Spawn a thread to receive mpsc progress updates and forward them to the callback_fn
    fn spawn_recv_progress_thread(
        progress_rx: mpsc::Receiver<Progress>,
        callback_fn: Callback,
    ) -> thread::JoinHandle<()> {
        thread::spawn(move || loop {
            match progress_rx.recv() {
                Ok(Progress::Sync {
                    progress_n,
                    tip_time,
                }) => notify(
                    callback_fn,
                    "progress:sync",
                    progress_n,
                    &tip_time.to_string(),
                ),
                Ok(Progress::Scan { progress_n, eta }) => {
                    notify(callback_fn, "progress:scan", progress_n, &eta.to_string())
                }
                Err(mpsc::RecvError) => break,
            }
        })
    }
}
