use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::{mpsc, Once};
use std::{any, panic, thread};

use anyhow::{Context, Error, Result};

use crate::util::bitcoincore_ext::Progress;
use crate::{App, Config};

const OK: i32 = 0;
const ERR: i32 = -1;

static INIT_LOGGER: Once = Once::new();

#[repr(C)]
pub struct ShutdownHandler(mpsc::SyncSender<()>);

type NotifyCallback = extern "C" fn(*const c_char, f32, u32, *const c_char);
type ReadyCallback = extern "C" fn(*const ShutdownHandler);

/// Start bwt. Accepts the config as a json string, a callback function
/// to receive status updates, and a pointer for the shutdown handler.
///
/// This will locks the thread until the initial sync is completed, then spawn
/// a background thread for continuous syncing and return a shutdown handler.
#[no_mangle]
pub extern "C" fn bwt_start(
    json_config: *const c_char,
    notify_fn: NotifyCallback,
    ready_fn: ReadyCallback,
) -> i32 {
    let json_config = unsafe { CStr::from_ptr(json_config) }.to_str().unwrap();

    let start = || -> Result<()> {
        let config: Config = serde_json::from_str(json_config).context("Invalid config")?;
        // The verbosity level cannot be changed once enabled.
        INIT_LOGGER.call_once(|| config.setup_logger());

        let (progress_tx, progress_rx) = mpsc::channel();
        spawn_recv_progress_thread(progress_rx, notify_fn);

        notify(notify_fn, "booting", 0.0, 0, "");
        let app = App::boot(config, Some(progress_tx))?;

        #[cfg(feature = "electrum")]
        if let Some(addr) = app.electrum_addr() {
            notify(notify_fn, "ready:electrum", 1.0, 0, &addr.to_string());
        }
        #[cfg(feature = "http")]
        if let Some(addr) = app.http_addr() {
            notify(notify_fn, "ready:http", 1.0, 0, &addr.to_string());
        }

        let (shutdown_tx, shutdown_rx) = mpsc::sync_channel(1);
        let shutdown_ptr = Box::into_raw(Box::new(ShutdownHandler(shutdown_tx)));
        ready_fn(shutdown_ptr);

        app.sync(Some(shutdown_rx));

        Ok(())
    };

    if let Err(e) = panic::catch_unwind(start)
        .map_err(fmt_panic)
        .and_then(|r| r.map_err(fmt_error))
    {
        warn!("{:?}", e);
        notify(notify_fn, "error", 0.0, 0, &e);
        ERR
    } else {
        OK
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

fn notify(notify_fn: NotifyCallback, msg_type: &str, progress: f32, detail_n: u64, detail_s: &str) {
    let msg_type = CString::new(msg_type).unwrap();
    let detail_s = CString::new(detail_s).unwrap();
    notify_fn(
        msg_type.as_ptr(),
        progress,
        detail_n as u32,
        detail_s.as_ptr(),
    );
    // drop CStrings
}

// Spawn a thread to receive mpsc progress updates and forward them to the notify_fn
fn spawn_recv_progress_thread(
    progress_rx: mpsc::Receiver<Progress>,
    notify_fn: NotifyCallback,
) -> thread::JoinHandle<()> {
    thread::spawn(move || loop {
        match progress_rx.recv() {
            Ok(Progress::Sync { progress_n, tip }) => {
                notify(notify_fn, "progress:sync", progress_n, tip, "")
            }
            Ok(Progress::Scan { progress_n, eta }) => {
                notify(notify_fn, "progress:scan", progress_n, eta, "")
            }
            Err(mpsc::RecvError) => break,
        }
    })
}

fn fmt_error(e: Error) -> String {
    let causes: Vec<String> = e.chain().map(|cause| cause.to_string()).collect();
    causes.join(": ")
}

fn fmt_panic(err: Box<dyn any::Any + Send + 'static>) -> String {
    format!(
        "panic: {}",
        if let Some(s) = err.downcast_ref::<&str>() {
            s
        } else if let Some(s) = err.downcast_ref::<String>() {
            s
        } else {
            "unknown panic"
        }
    )
}
