//! What went wrong, typed so the transfer engine and the app can decide
//! without parsing text, and worded as the user sees it today.

use std::fmt;

/// A provider's failure. Every kind carries the message the user sees,
/// unchanged from today's wording; `Display` gives exactly that message, so
/// a command still returns it as its `String` error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The item isn't there (any more).
    NotFound(String),
    /// Something already has that name.
    Exists(String),
    /// The place, or this item in it, can't be changed.
    ReadOnly(String),
    /// The provider doesn't do this at all (“This action is not available
    /// for Google Drive items yet.”).
    Unsupported(String),
    /// The account needs a credential or a new sign-in.
    AuthRequired(String),
    /// The server or the network couldn't be reached.
    Unreachable(String),
    /// More than the provider takes (a listing over its limit, say).
    TooLarge(String),
    /// The user cancelled; the message is normally [`crate::CANCELLED`].
    Cancelled(String),
    /// The place changed under the operation, or is ambiguous (several
    /// items with one name on Drive).
    Conflict(String),
    /// Anything else, as the provider words it.
    Other(String),
}

impl Error {
    /// The cancel every long operation ends with: “Cancelled.”.
    pub fn cancelled() -> Self {
        Error::Cancelled(crate::CANCELLED.into())
    }

    /// The message the user sees.
    pub fn message(&self) -> &str {
        match self {
            Error::NotFound(m)
            | Error::Exists(m)
            | Error::ReadOnly(m)
            | Error::Unsupported(m)
            | Error::AuthRequired(m)
            | Error::Unreachable(m)
            | Error::TooLarge(m)
            | Error::Cancelled(m)
            | Error::Conflict(m)
            | Error::Other(m) => m,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for Error {}

impl From<Error> for String {
    fn from(error: Error) -> String {
        error.to_string()
    }
}

/// A refusal before anything was written is the error of the same meaning,
/// with the same words.
impl From<crate::Refusal> for Error {
    fn from(refusal: crate::Refusal) -> Self {
        use crate::Refusal;
        let message = refusal.to_string();
        match refusal {
            Refusal::Vanished(_) => Error::NotFound(message),
            Refusal::Exists(_) => Error::Exists(message),
            Refusal::SamePath
            | Refusal::IntoItself
            | Refusal::ReplaceWithChild
            | Refusal::NotAName(_) => Error::Conflict(message),
        }
    }
}

/// A provider's result.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Refusal;

    #[test]
    fn display_is_the_message_alone_for_every_kind() {
        let kinds: [fn(String) -> Error; 10] = [
            Error::NotFound,
            Error::Exists,
            Error::ReadOnly,
            Error::Unsupported,
            Error::AuthRequired,
            Error::Unreachable,
            Error::TooLarge,
            Error::Cancelled,
            Error::Conflict,
            Error::Other,
        ];
        for kind in kinds {
            let error = kind("Arbesk is read only.".into());
            assert_eq!(error.to_string(), "Arbesk is read only.");
            assert_eq!(String::from(error.clone()), "Arbesk is read only.");
            assert_eq!(error.message(), "Arbesk is read only.");
        }
    }

    #[test]
    fn a_cancel_reads_as_every_cancel_does() {
        assert_eq!(Error::cancelled(), Error::Cancelled("Cancelled.".into()));
        assert_eq!(Error::cancelled().to_string(), crate::CANCELLED);
    }

    #[test]
    fn a_refusal_keeps_its_words_and_gets_its_kind() {
        let cases = [
            (Refusal::Vanished("a".into()), Error::NotFound("“a” no longer exists".into())),
            (Refusal::Exists("a".into()), Error::Exists("Target already exists: a".into())),
            (Refusal::SamePath, Error::Conflict("The source and destination are the same".into())),
            (Refusal::IntoItself, Error::Conflict("Cannot copy or move a folder into itself".into())),
            (
                Refusal::ReplaceWithChild,
                Error::Conflict("Cannot replace a folder with an item inside it".into()),
            ),
            (
                Refusal::NotAName("/a/..".into()),
                Error::Conflict("“/a/..” is not a valid name for a local item.".into()),
            ),
        ];
        for (refusal, error) in cases {
            assert_eq!(Error::from(refusal), error);
        }
    }
}
