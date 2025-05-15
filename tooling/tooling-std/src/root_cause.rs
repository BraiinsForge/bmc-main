// Copyright (C) 2025  Braiins Systems s.r.o.

use std::error::Error;
use std::fmt::{Display, Formatter};
use std::{fmt, io};

/// `Debug` is NOT implemented on purpose, because we want to avoid mistakes like `error!(err = ?err.root_cause());`.
/// In this case it prints an unreadable multiline debug representation of the error instead of the formatted display
/// representation, because `?` was used by mistake and `%` should've been used instead.
///
/// Another example is `println!("failed: {:?}", err.root_cause())`. `{}` should've been used instead of `{:?}`.
///
/// In general, using `.root_cause()` and then printing the debug representation doesn't make sense, because the
/// underlying error already implements `Debug`.
#[expect(missing_debug_implementations)]
#[derive(Clone)]
pub struct RootCauseDisplay<'a>(&'a (dyn Error + 'static));

impl Display for RootCauseDisplay<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let mut current = self.0;

        while let Some(err) = current.source() {
            current = err;
        }

        if let Some(io_error) = current.downcast_ref::<io::Error>() {
            Display::fmt(&io_error.kind(), f)?;
        } else {
            Display::fmt(current, f)?;
        }

        Ok(())
    }
}

pub trait RootCause {
    fn root_cause(&self) -> RootCauseDisplay<'_>;
}

impl<E: Error + 'static> RootCause for E {
    fn root_cause(&self) -> RootCauseDisplay<'_> {
        RootCauseDisplay(self)
    }
}

pub trait RootCauseAsRef {
    fn root_cause(&self) -> RootCauseDisplay<'_>;
}

// this works for `anyhow::Error`
impl<A: AsRef<dyn Error>> RootCauseAsRef for A {
    fn root_cause(&self) -> RootCauseDisplay<'_> {
        RootCauseDisplay(self.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use crate::root_cause::RootCause;
    use anyhow::{Context, anyhow};
    use reqwest::Client;
    use std::io;
    use std::io::ErrorKind;
    use thiserror::Error;

    #[test]
    fn works_with_anyhow() {
        let err = anyhow!("dummy error").context("wrapper").context("aaa");
        assert_eq!(err.root_cause().to_string(), "dummy error");
    }

    #[test]
    fn display_root_cause() {
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
            ErrorKind::NetworkDown,
            "dummy error",
        )));

        assert_eq!(format!("{err}"), "custom error");

        let root_cause = err.root_cause();
        assert_eq!(format!("{root_cause}"), "network down");
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

        let root_cause = err.root_cause();

        assert_eq!(format!("{err}"), "potato");
        assert_eq!(
            format!("{err:#}"),
            "potato: error sending request: error trying to connect: tcp connect error: Connection refused (os error 111): \
                                            error trying to connect: tcp connect error: Connection refused (os error 111): \
                                                                     tcp connect error: Connection refused (os error 111): \
                                                                                        Connection refused (os error 111)"
        );
        assert_eq!(format!("{root_cause}"), "Connection refused (os error 111)");
    }
}
