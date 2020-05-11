use std::sync::mpsc;
use std::thread;

use ::reqwest::blocking as reqwest;

use crate::indexer::IndexUpdate;

pub struct WebHookNotifier {
    _thread: thread::JoinHandle<()>,
    tx: mpsc::Sender<IndexUpdate>,
    num_urls: usize,
}

impl WebHookNotifier {
    pub fn start(urls: Vec<String>) -> Self {
        let (tx, rx) = mpsc::channel::<IndexUpdate>();
        let num_urls = urls.len();

        Self {
            // Spawn a separate thread for sending HTTP requests
            // TODO use reqwest's non-blocking mode
            _thread: thread::spawn(move || {
                let client = reqwest::Client::new();
                while let Ok(update) = rx.recv() {
                    for url in &urls {
                        debug!("notifying {}: {:?}", url, update);
                        client
                            .post(url)
                            .json(&update)
                            .send()
                            .map(|r| debug!("notifying {} succeed: {:#?}", url, r.status()))
                            .map_err(|e| warn!("notifying {} failed: {:?}", url, e))
                            .ok();
                    }
                }
            }),
            tx,
            num_urls,
        }
    }

    pub fn send_updates(&self, updates: &Vec<IndexUpdate>) {
        info!(
            "sending {} updates to {} urls",
            updates.len(),
            self.num_urls
        );

        // TODO implement filter support
        // XXX attach full tx info json to webhook request?
        for update in updates {
            self.tx.send(update.clone()).unwrap();
        }
    }
}
