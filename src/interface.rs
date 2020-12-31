use std::sync::{mpsc, Once};
use std::thread;

use anyhow::{Context, Error, Result};

use crate::util::bitcoincore_ext::Progress;
use crate::{App, Config};

#[repr(C)]
pub struct ShutdownHandler(mpsc::SyncSender<()>);

static INIT_LOGGER: Once = Once::new();

#[cfg(feature = "ffi")]
mod ffi {
    use super::*;
    use std::ffi::{CStr, CString};
    use std::os::raw::c_char;

    const OK: i32 = 0;
    const ERR: i32 = -1;

    type Callback = extern "C" fn(*const c_char, f32, u32, *const c_char);

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
            let config: Config = serde_json::from_str(json_config).context("Invalid config")?;
            // The verbosity level cannot be changed once enabled.
            INIT_LOGGER.call_once(|| config.setup_logger());

            let (progress_tx, progress_rx) = mpsc::channel();
            spawn_recv_progress_thread(progress_rx, callback_fn);

            notify(callback_fn, "booting", 0.0, 0, "");
            let app = App::boot(config, Some(progress_tx))?;

            #[cfg(feature = "electrum")]
            if let Some(addr) = app.electrum_addr() {
                notify(callback_fn, "ready:electrum", 1.0, 0, &addr.to_string());
            }
            #[cfg(feature = "http")]
            if let Some(addr) = app.http_addr() {
                notify(callback_fn, "ready:http", 1.0, 0, &addr.to_string());
            }

            notify(callback_fn, "ready", 1.0, 0, "");

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
                notify(callback_fn, "error", 0.0, 0, &e.to_string());
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

    fn notify(callback_fn: Callback, msg_type: &str, progress: f32, detail_n: u64, detail_s: &str) {
        let msg_type = CString::new(msg_type).unwrap();
        let detail_s = CString::new(detail_s).unwrap();
        callback_fn(
            msg_type.as_ptr(),
            progress,
            detail_n as u32,
            detail_s.as_ptr(),
        );
        // drop CStrings
    }

    // Spawn a thread to receive mpsc progress updates and forward them to the callback_fn
    fn spawn_recv_progress_thread(
        progress_rx: mpsc::Receiver<Progress>,
        callback_fn: Callback,
    ) -> thread::JoinHandle<()> {
        thread::spawn(move || loop {
            match progress_rx.recv() {
                Ok(Progress::Sync { progress_n, tip }) => {
                    notify(callback_fn, "progress:sync", progress_n, tip, "")
                }
                Ok(Progress::Scan { progress_n, eta }) => {
                    notify(callback_fn, "progress:scan", progress_n, eta, "")
                }
                Err(mpsc::RecvError) => break,
            }
        })
    }
}

#[cfg(feature = "jni")]
mod jni {
    use super::*;
    use ::jni::objects::{GlobalRef, JClass, JObject, JString};
    use ::jni::sys::{jfloat, jint, jlong};
    use ::jni::{JNIEnv, JavaVM};

    #[no_mangle]
    pub extern "system" fn Java_dev_bwt_daemon_NativeBwtDaemon_start(
        env: JNIEnv,
        _: JClass,
        json_config: JString,
        callback: JObject,
    ) {
        let json_config: String = env.get_string(json_config).unwrap().into();

        let jvm = env.get_java_vm().unwrap();
        let callback_g = env.new_global_ref(callback).unwrap();

        let start = || -> Result<_> {
            let config: Config = serde_json::from_str(&json_config).context("Invalid config")?;
            // The verbosity level cannot be changed once enabled.
            INIT_LOGGER.call_once(|| config.setup_logger());

            let (progress_tx, progress_rx) = mpsc::channel();
            spawn_recv_progress_thread(progress_rx, jvm, callback_g);

            env.call_method(callback, "onBooting", "()V", &[]).unwrap();
            let app = App::boot(config, Some(progress_tx))?;

            #[cfg(feature = "electrum")]
            if let Some(addr) = app.electrum_addr() {
                let addr = env.new_string(addr.to_string()).unwrap().into_inner();
                env.call_method(
                    callback,
                    "onElectrumReady",
                    "(Ljava/lang/String;)V",
                    &[addr.into()],
                )
                .unwrap();
            }
            #[cfg(feature = "http")]
            if let Some(addr) = app.http_addr() {
                let addr = env.new_string(addr.to_string()).unwrap().into_inner();
                env.call_method(
                    callback,
                    "onHttpReady",
                    "(Ljava/lang/String;)V",
                    &[addr.into()],
                )
                .unwrap();
            }

            let (shutdown_tx, shutdown_rx) = mpsc::sync_channel(1);
            let shutdown_handler = ShutdownHandler(shutdown_tx);
            let shutdown_ptr = Box::into_raw(Box::new(shutdown_handler)) as jlong;

            env.call_method(callback, "onReady", "(J)V", &[shutdown_ptr.into()])
                .unwrap();

            info!("start background sync");
            app.sync(Some(shutdown_rx));

            Ok(())
        };

        if let Err(e) = start() {
            warn!("{:?}", e);
            env.throw_new("dev/bwt/daemon/BwtException", &fmt_error(&e))
                .unwrap();
        }
    }

    #[no_mangle]
    pub unsafe extern "system" fn Java_dev_bwt_daemon_NativeBwtDaemon_shutdown(
        _env: JNIEnv,
        _: JClass,
        shutdown_ptr: jlong,
    ) {
        // Take ownership and drop it. This will disconnect the mpsc channel and shutdown the app.
        Box::from_raw(shutdown_ptr as *mut ShutdownHandler);
    }

    #[no_mangle]
    pub extern "system" fn Java_dev_bwt_daemon_NativeBwtDaemon_testRpc(
        env: JNIEnv,
        _: JClass,
        json_config: JString,
    ) {
        let json_config: String = env.get_string(json_config).unwrap().into();

        let test = || App::test_rpc(&serde_json::from_str(&json_config)?);

        if let Err(e) = test() {
            warn!("test rpc failed: {:?}", e);
            env.throw_new("dev/bwt/daemon/BwtException", &e.to_string())
                .unwrap();
        }
    }

    fn spawn_recv_progress_thread(
        progress_rx: mpsc::Receiver<Progress>,
        jvm: JavaVM,
        callback: GlobalRef,
    ) -> thread::JoinHandle<()> {
        let (tx, rx) = mpsc::sync_channel(1);
        let handle = thread::spawn(move || {
            tx.send(()).unwrap();
            let env = jvm.attach_current_thread().unwrap();
            loop {
                match progress_rx.recv() {
                    Ok(Progress::Sync { progress_n, tip }) => {
                        let progress_n = progress_n as jfloat;
                        let tip = tip as jint;
                        env.call_method(
                            &callback,
                            "onSyncProgress",
                            "(FI)V",
                            &[progress_n.into(), tip.into()],
                        )
                        .unwrap();
                    }
                    Ok(Progress::Scan { progress_n, eta }) => {
                        let progress_n = progress_n as jfloat;
                        let eta = eta as jint;
                        env.call_method(
                            &callback,
                            "onScanProgress",
                            "(FI)V",
                            &[progress_n.into(), eta.into()],
                        )
                        .unwrap();
                    }
                    Err(mpsc::RecvError) => break,
                }
            }
        });
        // wait for the thread to start
        rx.recv().unwrap();

        handle
    }
}

fn fmt_error(e: &Error) -> String {
    let causes: Vec<String> = e.chain().map(|cause| cause.to_string()).collect();
    causes.join(": ")
}
