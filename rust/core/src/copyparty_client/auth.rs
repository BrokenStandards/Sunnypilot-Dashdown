//! copyparty authentication. Anonymous or a password sent via the `PW:` header
//! (kept out of the URL so it isn't logged). Passwords are redacted in tracing.

/// How to authenticate to a copyparty server.
#[derive(Clone)]
pub enum Credentials {
    Anonymous,
    Password(String),
}

impl Credentials {
    /// `None`/empty ⇒ anonymous.
    pub fn from_optional(pw: Option<&str>) -> Self {
        match pw {
            Some(p) if !p.is_empty() => Credentials::Password(p.to_string()),
            _ => Credentials::Anonymous,
        }
    }
}

// Never print the password.
impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Credentials::Anonymous => f.write_str("Anonymous"),
            Credentials::Password(_) => f.write_str("Password(***)"),
        }
    }
}

/// Attach the `PW:` header when a password is set.
pub(crate) fn apply_auth(
    rb: reqwest::RequestBuilder,
    creds: &Credentials,
) -> reqwest::RequestBuilder {
    match creds {
        Credentials::Password(p) => rb.header("PW", p),
        Credentials::Anonymous => rb,
    }
}
