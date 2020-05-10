use std::sync::mpsc;
use std::thread;

use ::reqwest::blocking as reqwest;

use crate::indexer::IndexUpdate;

pub struct WebHookNotifier {
    _thread: thread::JoinHandle<()>,
    tx: mpsc::Sender<IndexUpdate>,
}

impl WebHookNotifier {
    pub fn start(urls: Vec<String>) -> Self {
        let (tx, rx) = mpsc::channel::<IndexUpdate>();

        Self {
            // Spawn a separate thread for sending HTTP requests
            // TODO use reqwest's non-blocking mode
            _thread: thread::spawn(move || {
                let client = reqwest::Client::new();
                while let Ok(update) = rx.recv() {
                    for url in &urls {
                        info!("notifying webhook {}: {:?}", url, update);
                        client
                            .post(url)
                            .json(&update)
                            .send()
                            .map_err(|e| warn!("failed notifying {}: {:?}", url, e))
                            .ok();
                    }
                }
            }),
            tx,
        }
    }

    pub fn send_updates(&self, updates: &Vec<IndexUpdate>) {
        info!("sending webhook notifications");

        // TODO implement filter support
        // XXX attach full tx info json to webhook request?
        for update in updates {
            self.tx.send(update.clone()).unwrap();
        }
    }
}
