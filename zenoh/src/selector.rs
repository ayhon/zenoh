use zenoh_protocol_core::key_expr::{keyexpr, OwnedKeyExpr};

use crate::{prelude::KeyExpr, queryable::Query};

use std::{borrow::Cow, convert::TryFrom};

/// A selector is the combination of a [Key Expression](crate::prelude::KeyExpr), which defines the
/// set of keys that are relevant to an operation, and a `value_selector`, a set of key-value pairs
/// with a few uses:
/// - specifying arguments to a queryable, allowing the passing of Remote Procedure Call parameters
/// - filtering by value,
/// - filtering by metadata, such as the timestamp of a value,
///
/// When in string form, selectors look a lot like a URI, with similar semantics:
/// - the `key_expr` before the first `?` must be a valid key expression.
/// - the `selector` after the first `?` should be encoded like the query section of a URL:
///     - key-value pairs are separated by `&`,
///     - the key and value are separated by the first `=`,
///     - in the absence of `=`, the value is considered to be the empty string,
///     - both key and value should use percent-encoding to escape characters,
///     - defining a value for the same key twice is considered undefined behavior.
///
/// Zenoh intends to standardize the usage of a set of keys. To avoid conflicting with RPC parameters,
/// the Zenoh team has settled on reserving the set of keys that start with non-alphanumeric characters.
///
/// This document will summarize the standardized keys for which Zenoh provides helpers to facilitate
/// coherent behavior for some operations.
///
/// Queryable implementers are encouraged to prefer these standardized keys when implementing their
/// associated features, and to prefix their own keys to avoid having conflicting keys with other
/// queryables.
///
/// Here are the currently standardized keys for Zenoh:
/// - `_time`: used to express interest in only values dated within a certain time range, values for
///   this key must be readable by the Zenoh Time DSL for the value to be considered valid.
/// - `_filter`: *TBD* Zenoh intends to provide helper tools to allow the value associated with
///   this key to be treated as a predicate that the value should fulfill before being returned.
///   A DSL will be designed by the Zenoh team to express these predicates.
#[derive(Clone, PartialEq)]
pub struct Selector<'a> {
    /// The part of this selector identifying which keys should be part of the selection.
    pub key_expr: KeyExpr<'a>,
    /// the part of this selector identifying which values should be part of the selection.
    pub(crate) value_selector: Cow<'a, str>,
}

impl<'a> Selector<'a> {
    pub fn borrowing_clone(&'a self) -> Self {
        Selector {
            key_expr: self.key_expr.borrowing_clone(),
            value_selector: self.value_selector.as_ref().into(),
        }
    }
    pub fn into_owned(self) -> Selector<'static> {
        Selector {
            key_expr: self.key_expr.into_owned(),
            value_selector: self.value_selector.into_owned().into(),
        }
    }

    #[deprecated = "If you have ownership of this selector, prefer `Selector::into_owned`"]
    pub fn to_owned(&self) -> Selector<'static> {
        self.borrowing_clone().into_owned()
    }

    /// Returns this selectors components as a tuple.
    pub fn split(self) -> (KeyExpr<'a>, Cow<'a, str>) {
        (self.key_expr, self.value_selector)
    }

    /// Sets the `value_selector` part of this `Selector`.
    #[inline(always)]
    pub fn with_value_selector(mut self, value_selector: &'a str) -> Self {
        self.value_selector = value_selector.into();
        self
    }

    /// Gets the value selector as a raw string.
    pub fn value_selector(&self) -> &str {
        &self.value_selector
    }

    /// Returns the value selector as an iterator of key-value pairs, where any urlencoding has been decoded.
    pub fn decode_value_selector(
        &'a self,
    ) -> impl Iterator<Item = (Cow<'a, str>, Cow<'a, str>)> + Clone + 'a {
        self.value_selector().decode()
    }

    pub fn extend<'b, I, K, V>(&'b mut self, key_value_pairs: I)
    where
        I: IntoIterator,
        I::Item: std::borrow::Borrow<(K, V)>,
        K: AsRef<str> + 'b,
        V: AsRef<str> + 'b,
    {
        let it = key_value_pairs.into_iter();
        if let Cow::Borrowed(s) = self.value_selector {
            self.value_selector = Cow::Owned(s.to_owned())
        }
        let selector = if let Cow::Owned(s) = &mut self.value_selector {
            s
        } else {
            unsafe { std::hint::unreachable_unchecked() } // this is safe because we just replaced the borrowed variant
        };
        let mut encoder = form_urlencoded::Serializer::new(selector);
        encoder.extend_pairs(it).finish();
    }
}
pub trait ValueSelector<'a> {
    type Decoder: Iterator<Item = (Cow<'a, str>, Cow<'a, str>)> + Clone + 'a;
    fn decode(&'a self) -> Self::Decoder;
}
impl<'a> ValueSelector<'a> for str {
    type Decoder = form_urlencoded::Parse<'a>;
    fn decode(&'a self) -> Self::Decoder {
        form_urlencoded::parse(self.as_bytes())
    }
}

impl std::fmt::Debug for Selector<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "sel\"{}\"", self)
    }
}

impl std::fmt::Display for Selector<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}?{}", self.key_expr, self.value_selector)
    }
}

impl<'a> From<&Selector<'a>> for Selector<'a> {
    fn from(s: &Selector<'a>) -> Self {
        s.clone()
    }
}

impl TryFrom<String> for Selector<'_> {
    type Error = zenoh_core::Error;
    fn try_from(mut s: String) -> Result<Self, Self::Error> {
        match s.find('?') {
            Some(qmark_position) => {
                let value_selector = s[qmark_position + 1..].to_owned();
                s.truncate(qmark_position);
                Ok(KeyExpr::try_from(s)?.with_owned_value_selector(value_selector))
            }
            None => Ok(KeyExpr::try_from(s)?.into()),
        }
    }
}

impl<'a> TryFrom<&'a str> for Selector<'a> {
    type Error = zenoh_core::Error;
    fn try_from(s: &'a str) -> Result<Self, Self::Error> {
        match s.find('?') {
            Some(qmark_position) => {
                let value_selector = &s[qmark_position + 1..];
                Ok(KeyExpr::try_from(&s[..qmark_position])?.with_value_selector(value_selector))
            }
            None => Ok(KeyExpr::try_from(s)?.into()),
        }
    }
}

impl<'a> TryFrom<&'a String> for Selector<'a> {
    type Error = zenoh_core::Error;
    fn try_from(s: &'a String) -> Result<Self, Self::Error> {
        Self::try_from(s.as_str())
    }
}

impl<'a> From<&'a Query> for Selector<'a> {
    fn from(q: &'a Query) -> Self {
        Selector {
            key_expr: q.key_expr.borrowing_clone(),
            value_selector: (&q.value_selector).into(),
        }
    }
}

impl<'a> From<&KeyExpr<'a>> for Selector<'a> {
    fn from(key_selector: &KeyExpr<'a>) -> Self {
        Self {
            key_expr: key_selector.clone(),
            value_selector: "".into(),
        }
    }
}

impl<'a> From<&'a keyexpr> for Selector<'a> {
    fn from(key_selector: &'a keyexpr) -> Self {
        Self {
            key_expr: key_selector.into(),
            value_selector: "".into(),
        }
    }
}

impl<'a> From<&'a OwnedKeyExpr> for Selector<'a> {
    fn from(key_selector: &'a OwnedKeyExpr) -> Self {
        Self {
            key_expr: key_selector.into(),
            value_selector: "".into(),
        }
    }
}

impl From<OwnedKeyExpr> for Selector<'static> {
    fn from(key_selector: OwnedKeyExpr) -> Self {
        Self {
            key_expr: key_selector.into(),
            value_selector: "".into(),
        }
    }
}

impl<'a> From<KeyExpr<'a>> for Selector<'a> {
    fn from(key_selector: KeyExpr<'a>) -> Self {
        Self {
            key_expr: key_selector,
            value_selector: "".into(),
        }
    }
}