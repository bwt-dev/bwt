use std::fs;
use std::net;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;

// Spawn a unix socket listener that triggers an indexer sync by whenever a connection is opened
pub fn start(socket_path: PathBuf, tx: mpsc::Sender<()>) -> thread::JoinHandle<()> {
    thread::spawn(move || bind_listener(socket_path, tx).unwrap())
}

fn bind_listener(socket_path: PathBuf, sync_tx: mpsc::Sender<()>) -> std::io::Result<()> {
    // cleanup socket file from previous run (should ideally happen on shutdown)
    if let Ok(meta) = fs::metadata(&socket_path) {
        if meta.file_type().is_socket() {
            fs::remove_file(&socket_path)?;
        }
    }

    info!("binding unix socket on {:?}", socket_path);

    let listener = UnixListener::bind(socket_path)?;
    for stream in listener.incoming() {
        trace!("received sync notification via unix socket");
        // drop the connection, ignore any errors
        stream.and_then(|s| s.shutdown(net::Shutdown::Both)).ok();

        if sync_tx.send(()).is_err() {
            break;
        }
        // FIXME the listener thread won't be closed until it receives a connection and attempts to send()
    }
    Ok(())
}
