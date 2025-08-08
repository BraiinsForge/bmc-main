// Copyright (C) 2025  Braiins Systems s.r.o.

//! `Debug` is NOT implemented for `DisplayChain` and `DisplayRootCause` on purpose, because we want to avoid mistakes
//! like `error!(err = ?err.display_chain());`. In this case it prints an unreadable multiline debug representation of
//! the display object instead of the formatted display representation, because `?` was used by mistake and `%`
//! should've been used instead.
//!
//! Another example is `println!("failed: {:?}", err.display_chain())`. `{}` should've been used instead of `{:?}`.
//!
//! In general, using `.display_chain()` or `.display_root_cause()` and then printing the debug representation doesn't
//! make sense, because the underlying error already implements `Debug`.

use std::error::Error;
use std::fmt::{Display, Formatter};
use std::io::ErrorKind;
use std::{fmt, io};

#[rustfmt::skip] // rustfmt destroys formatting of higher order macros
macro_rules! define_display_chain_log_macro {
    ($name:ident, $level:ident) => {
        #[macro_export]
        macro_rules! $name {
            ($error:expr) => {{
                use $crate::error_display::ErrorDisplay as _;
                use $crate::error_display::ErrorDisplayAsRef as _;
                let error = ($error).display_chain();
                ::tracing::$level!(%error);
            }};
        }
    };
}

define_display_chain_log_macro!(log_chain_error, error);
define_display_chain_log_macro!(log_chain_warn, warn);
define_display_chain_log_macro!(log_chain_info, info);
define_display_chain_log_macro!(log_chain_debug, debug);
define_display_chain_log_macro!(log_chain_trace, trace);

pub trait ErrorDisplay {
    fn display_chain(&self) -> DisplayChain<'_>;
    fn display_root_cause(&self) -> DisplayRootCause<'_>;
}

impl<E: Error + 'static> ErrorDisplay for E {
    fn display_chain(&self) -> DisplayChain<'_> {
        DisplayChain(self)
    }

    fn display_root_cause(&self) -> DisplayRootCause<'_> {
        DisplayRootCause(self)
    }
}

pub trait ErrorDisplayAsRef {
    fn display_chain(&self) -> DisplayChain<'_>;
    fn display_root_cause(&self) -> DisplayRootCause<'_>;
}

// this works for `anyhow::Error`
impl<A: AsRef<dyn Error>> ErrorDisplayAsRef for A {
    fn display_chain(&self) -> DisplayChain<'_> {
        DisplayChain(self.as_ref())
    }

    fn display_root_cause(&self) -> DisplayRootCause<'_> {
        DisplayRootCause(self.as_ref())
    }
}

#[expect(missing_debug_implementations)]
#[derive(Clone)]
pub struct DisplayChain<'a>(&'a (dyn Error + 'static));

impl Display for DisplayChain<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let mut current = self.0;

        fmt_error(f, current)?;

        while let Some(err) = current.source() {
            // try to detect whether an error displays it's source error
            // TODO: remove this after upgrading hyper to v1 (https://github.com/seanmonstar/reqwest/pull/2199, https://github.com/hyperium/hyper/pull/3312)
            if current.to_string().contains(&err.to_string()) {
                current = err;
                continue;
            }

            f.write_str(": ")?;
            current = err;

            fmt_error(f, current)?;
        }

        Ok(())
    }
}

#[expect(missing_debug_implementations)]
#[derive(Clone)]
pub struct DisplayRootCause<'a>(&'a (dyn Error + 'static));

impl Display for DisplayRootCause<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let mut current = self.0;

        while let Some(err) = current.source() {
            current = err;
        }

        fmt_error(f, current)?;

        Ok(())
    }
}

fn fmt_error(f: &mut Formatter<'_>, error: &(dyn Error + 'static)) -> fmt::Result {
    match error.downcast_ref::<io::Error>() {
        Some(io_error) if io_error.kind() != ErrorKind::Other => {
            Display::fmt(&io_error.kind(), f)?;
        }
        _ => {
            Display::fmt(error, f)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests_chain {
    use crate::error_display::{ErrorDisplay, ErrorDisplayAsRef};
    use anyhow::{Context, anyhow};
    use reqwest::Client;
    use std::io;
    use std::io::ErrorKind;
    use thiserror::Error;

    #[test]
    fn works_with_anyhow() {
        let err = anyhow!("dummy error").context("wrapper").context("aaa");
        let chain = err.display_chain();
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

        let chain = err.display_chain();
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

        let chain = err.display_chain();

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

#[cfg(test)]
mod tests_root_cause {
    use crate::error_display::{ErrorDisplay, ErrorDisplayAsRef};
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

        let root_cause = err.display_root_cause();
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

        let root_cause = err.display_root_cause();

        assert_eq!(format!("{err}"), "potato");
        assert_eq!(
            format!("{err:#}"),
            "potato: error sending request: error trying to connect: tcp connect error: Connection refused (os error 111): \
                                            error trying to connect: tcp connect error: Connection refused (os error 111): \
                                                                     tcp connect error: Connection refused (os error 111): \
                                                                                        Connection refused (os error 111)"
        );
        assert_eq!(format!("{root_cause}"), "connection refused");
    }
}
