use std::sync::mpsc;
use std::thread;

use ::reqwest::blocking as reqwest;

use crate::indexer::IndexChange;

pub struct WebHookNotifier {
    _thread: thread::JoinHandle<()>,
    tx: mpsc::Sender<Vec<IndexChange>>,
    num_urls: usize,
}

impl WebHookNotifier {
    pub fn start(urls: Vec<String>) -> Self {
        let (tx, rx) = mpsc::channel::<Vec<IndexChange>>();
        let num_urls = urls.len();

        Self {
            // Spawn a separate thread for sending HTTP requests
            // TODO use reqwest's non-blocking mode
            _thread: thread::spawn(move || {
                let client = reqwest::Client::new();
                while let Ok(changelog) = rx.recv() {
                    for url in &urls {
                        // XXX attach full tx info json to webhook request?
                        debug!("notifying {} with {} events", url, changelog.len());
                        client
                            .post(url)
                            .json(&changelog)
                            .send()
                            .map(|r| debug!("notifying {} succeed: {:?}", url, r.status()))
                            .map_err(|e| warn!("notifying {} failed: {:?}", url, e))
                            .ok();
                    }
                }
                trace!("webhooks shutting down");
            }),
            tx,
            num_urls,
        }
    }

    pub fn send_updates(&self, changelog: &[IndexChange]) {
        info!(
            "sending {} update(s) to {} url(s)",
            changelog.len(),
            self.num_urls
        );

        // TODO implement filter support
        self.tx.send(changelog.to_vec()).unwrap();
    }
}
