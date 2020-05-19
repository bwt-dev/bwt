use core::fmt::Display;

pub use anyhow::{Context, Error, Result};

pub trait OptionExt<T> {
    fn or_err<D>(self, context: D) -> Result<T>
    where
        D: Display + Send + Sync + 'static;

    fn req(self) -> Result<T>;
}

impl<T> OptionExt<T> for Option<T> {
    fn or_err<D>(self, context: D) -> Result<T>
    where
        D: Display + Send + Sync + 'static,
    {
        self.context(context)
    }

    fn req(self) -> Result<T> {
        self.context("missing required option")
    }
}

pub fn fmt_error_chain(err: &Error) -> String {
    err.chain()
        .map(|e| e.to_string())
        .collect::<Vec<String>>()
        .join(": ")
}
