#[cfg(feature = "ffi")]
mod ffi {
    use std::ffi::{CStr, CString};
    use std::os::raw::c_char;
    use std::sync::mpsc;
    use std::thread;

    use crate::{App, Config, Result};

    const OK: i32 = 0;
    const ERR: i32 = -1;

    #[repr(C)]
    pub struct ShutdownHandler(mpsc::Sender<()>);

    /// Start bwt. Accepts the config as a json string, a callback function
    /// to receive status updates, and a pointer for the shutdown handler
    #[no_mangle]
    pub extern "C" fn bwt_start(
        json_config: *const c_char,
        callback_fn: extern "C" fn(*const c_char, f32, *const c_char),
        shutdown_out: *mut *const ShutdownHandler,
    ) -> i32 {
        let json_config = unsafe { CStr::from_ptr(json_config) }.to_str().unwrap();

        let callback = |msg_type: &str, progress: f32, detail: &str| {
            callback_fn(cstring(msg_type), progress, cstring(detail))
        };

        let start = || -> Result<_> {
            let config: Config = serde_json::from_str(json_config)?;
            if config.verbose > 0 {
                config.setup_logger();
            }

            // TODO emit rescan progress updates with ETA from App::boot() and forward them
            callback("booting", 0.0, "");
            let app = App::boot(config)?;

            #[cfg(feature = "electrum")]
            if let Some(addr) = app.electrum_addr() {
                callback("ready:electrum", 1.0, &addr.to_string());
            }
            #[cfg(feature = "http")]
            if let Some(addr) = app.http_addr() {
                callback("ready:http", 1.0, &addr.to_string());
            }

            callback("ready", 1.0, "");

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
                callback("error", 0.0, &e.to_string());
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

    fn cstring(s: &str) -> *const c_char {
        CString::new(s).unwrap().into_raw()
    }
}
