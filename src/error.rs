use core::fmt::Display;
use std::result::Result as StdResult;

pub use failure::{Context, Error, ResultExt};

pub type Result<T> = StdResult<T, Error>;

pub trait OptionExt<T> {
    fn or_err<D>(self, context: D) -> StdResult<T, Context<D>>
    where
        D: Display + Send + Sync + 'static;

    fn req(self) -> StdResult<T, Context<&'static str>>;
}

impl<T> OptionExt<T> for Option<T> {
    fn or_err<D>(self, context: D) -> StdResult<T, Context<D>>
    where
        D: Display + Send + Sync + 'static,
    {
        self.ok_or_else(|| Context::new(context))
    }

    fn req(self) -> StdResult<T, Context<&'static str>> {
        self.ok_or_else(|| Context::new("missing required option"))
    }
}
