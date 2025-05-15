// Copyright (C) 2023  Braiins Systems s.r.o.

use std::error::Error;
use std::fmt;
use std::fmt::{Display, Formatter};

/// `Debug` is NOT implemented on purpose, because we want to avoid mistakes like `error!(err = ?err.error_chain());`.
/// In this case it prints an unreadable multiline debug representation of the error instead of the formatted display
/// representation, because `?` was used by mistake and `%` should've been used instead.
///
/// Another example is `println!("failed: {:?}", err.error_chain())`. `{}` should've been used instead of `{:?}`.
///
/// In general, using `.error_chain()` and then printing the debug representation doesn't make sense, because the
/// underlying error already implements `Debug`.
#[expect(missing_debug_implementations)]
#[derive(Clone)]
pub struct ErrorChainDisplay<'a>(&'a dyn Error);

impl Display for ErrorChainDisplay<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let mut current = self.0;
        Display::fmt(current, f)?;

        while let Some(err) = current.source() {
            // try to detect whether an error displays it's source error
            if current.to_string().contains(&err.to_string()) {
                current = err;
                continue;
            }

            f.write_str(": ")?;
            current = err;
            Display::fmt(current, f)?;
        }

        Ok(())
    }
}

pub trait ErrorChain {
    fn error_chain(&self) -> ErrorChainDisplay<'_>;
}

impl<E: Error> ErrorChain for E {
    fn error_chain(&self) -> ErrorChainDisplay<'_> {
        ErrorChainDisplay(self)
    }
}

pub trait ErrorChainAsRef {
    fn error_chain(&self) -> ErrorChainDisplay<'_>;
}

// this works for `anyhow::Error`
impl<A: AsRef<dyn Error>> ErrorChainAsRef for A {
    fn error_chain(&self) -> ErrorChainDisplay<'_> {
        ErrorChainDisplay(self.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use crate::error_chain::{ErrorChain, ErrorChainAsRef};
    use anyhow::{Context, anyhow};
    use reqwest::Client;
    use std::io;
    use std::io::ErrorKind;
    use thiserror::Error;

    #[test]
    fn works_with_anyhow() {
        let err = anyhow!("dummy error").context("wrapper").context("aaa");
        let chain = err.error_chain();
        assert_eq!(format!("{chain}"), format!("{err:#}"));
    }

    #[test]
    fn display_error_chain() {
        #[derive(Error, Debug)]
        enum CustomError {
            #[error("io error")]
            IO(#[from] io::Error),
        }

        #[derive(Error, Debug)]
        enum CustomError2 {
            #[error("custom error")]
            Custom(#[from] CustomError),
        }

        let err = CustomError2::Custom(CustomError::IO(io::Error::new(
            ErrorKind::Other,
            "dummy error",
        )));

        assert_eq!(format!("{err}"), "custom error");

        let chain = err.error_chain();
        assert_eq!(format!("{chain}"), "custom error: io error: dummy error");
    }

    #[tokio::test]
    async fn skip_duplicated_error_messages() {
        let err = Client::new()
            .get("http://127.127.127.127")
            .send()
            .await
            .map_err(reqwest::Error::without_url)
            .context("potato")
            .unwrap_err();

        let chain = err.error_chain();

        assert_eq!(format!("{err}"), "potato");
        assert_eq!(
            format!("{err:#}"),
            "potato: error sending request: error trying to connect: tcp connect error: Connection refused (os error 111): \
                                            error trying to connect: tcp connect error: Connection refused (os error 111): \
                                                                     tcp connect error: Connection refused (os error 111): \
                                                                                        Connection refused (os error 111)"
        );
        assert_eq!(
            format!("{chain}"),
            "potato: error sending request: error trying to connect: tcp connect error: Connection refused (os error 111)"
        );
    }
}
