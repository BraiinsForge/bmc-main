// Copyright (C) 2023  Braiins Systems s.r.o.

use url::Url;

pub trait UrlExt {
    /// Creates a new [`Url`] with `path` adjoined to `self`. This method was created as
    /// a replacement for `Url::join()`, which has unexpected/confusing behavior.
    /// <https://github.com/servo/rust-url/issues/333>
    ///
    /// # Example
    /// ```
    /// use url::Url;
    /// use tooling_std::url::UrlExt;
    ///
    /// let url = Url::parse("https://braiins.com/pool").unwrap();
    /// let new_url = url.join_path("foo").unwrap().join_path("bar/baz").unwrap();
    ///
    /// assert_eq!(new_url.as_str(), "https://braiins.com/pool/foo/bar/baz")
    ///
    /// ```
    fn join_path(&self, path: impl AsRef<str>) -> Result<Url, url::ParseError>;
}

impl UrlExt for Url {
    fn join_path(&self, path: impl AsRef<str>) -> Result<Url, url::ParseError> {
        let mut url = self.clone();
        url.path_segments_mut()
            .map_err(|()| url::ParseError::RelativeUrlWithCannotBeABaseBase)?
            .pop_if_empty()
            .extend(path.as_ref().split('/'));
        Ok(url)
    }
}
