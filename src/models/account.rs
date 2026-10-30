use super::*;

/// Who the client is currently acting as.
///
/// There is always someone: the first launch mints a guest rather than showing
/// a sign-in wall, so every code path downstream can assume an owner instead of
/// carrying an `Option` for the un-authenticated case.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub display_name: String,
    /// `None` for a guest. Its presence is what "has a real account" means.
    #[serde(default)]
    pub email: Option<String>,
    pub is_guest: bool,
}

impl Account {
    /// A guest has nothing to sign back in with, so clearing its cookie loses
    /// the library. That is worth saying out loud in the UI, and this is the
    /// test the UI asks.
    pub fn is_recoverable(&self) -> bool {
        !self.is_guest && self.email.is_some()
    }

    pub fn label(&self) -> &str {
        if let Some(email) = self.email.as_deref() {
            email
        } else {
            &self.display_name
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credentials {
    pub email: String,
    pub password: String,
}

/// Why a credential was refused, in the terms the form needs to render.
///
/// A bare string would do for display, but the form also has to decide *which
/// field* to mark, and parsing that back out of prose is how those two drift
/// apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CredentialProblem {
    EmailEmpty,
    EmailMalformed,
    EmailTaken,
    PasswordTooShort,
    PasswordTooLong,
}

impl CredentialProblem {
    pub fn message(self) -> &'static str {
        match self {
            Self::EmailEmpty => "Enter an email address.",
            Self::EmailMalformed => "That does not look like an email address.",
            Self::EmailTaken => "An account already uses that email.",
            Self::PasswordTooShort => "Use at least 8 characters.",
            Self::PasswordTooLong => "Use at most 512 characters.",
        }
    }
}

/// Argon2 is deliberately slow, so a password long enough to be a denial of
/// service is rejected before it reaches the hasher.
pub const PASSWORD_MIN_LENGTH: usize = 8;
pub const PASSWORD_MAX_LENGTH: usize = 512;

/// Shared by both sides so a client-side form and the server agree on what is
/// acceptable without the rules being written twice.
///
/// Deliberately not an RFC 5322 parser. The only thing this can usefully
/// establish is that the viewer typed something address-shaped; whether it
/// receives mail is not knowable here, and Tawny never sends any.
pub fn validate_email(email: &str) -> Result<String, CredentialProblem> {
    let normalized = email.trim().to_lowercase();
    if normalized.is_empty() {
        return Err(CredentialProblem::EmailEmpty);
    }
    let mut parts = normalized.split('@');
    let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err(CredentialProblem::EmailMalformed);
    };
    let domain_is_dotted = domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains("..");
    if local.is_empty() || domain.is_empty() || !domain_is_dotted {
        return Err(CredentialProblem::EmailMalformed);
    }
    if normalized.chars().any(char::is_whitespace) {
        return Err(CredentialProblem::EmailMalformed);
    }
    Ok(normalized)
}

pub fn validate_password(password: &str) -> Result<(), CredentialProblem> {
    // Not trimmed: leading and trailing spaces are legitimate password
    // characters, and silently removing them would lock out anyone who used
    // one deliberately.
    if password.chars().count() < PASSWORD_MIN_LENGTH {
        return Err(CredentialProblem::PasswordTooShort);
    }
    if password.chars().count() > PASSWORD_MAX_LENGTH {
        return Err(CredentialProblem::PasswordTooLong);
    }
    Ok(())
}

#[cfg(test)]
mod account_tests {
    use super::*;

    #[test]
    fn email_validation_normalizes_case_and_surrounding_space() {
        assert_eq!(
            validate_email("  Viewer@Example.COM "),
            Ok("viewer@example.com".into())
        );
    }

    #[test]
    fn email_validation_rejects_addresses_that_are_not_address_shaped() {
        for candidate in [
            "",
            "   ",
            "viewer",
            "viewer@",
            "@example.com",
            "viewer@example",
            "viewer@.com",
            "viewer@example.",
            "viewer@exa..mple.com",
            "one@two@example.com",
            "view er@example.com",
        ] {
            assert!(
                validate_email(candidate).is_err(),
                "expected {candidate:?} to be rejected"
            );
        }
    }

    #[test]
    fn password_length_is_bounded_at_both_ends() {
        assert_eq!(
            validate_password("short"),
            Err(CredentialProblem::PasswordTooShort)
        );
        assert!(validate_password("longenough").is_ok());
        assert_eq!(
            validate_password(&"x".repeat(PASSWORD_MAX_LENGTH + 1)),
            Err(CredentialProblem::PasswordTooLong)
        );
    }

    #[test]
    fn password_whitespace_is_significant() {
        // Eight characters only if the spaces count, which is the point.
        assert!(validate_password(" pass a ").is_ok());
    }

    #[test]
    fn only_a_real_account_is_recoverable() {
        let guest = Account {
            id: "app_user:abc".into(),
            display_name: "Guest".into(),
            email: None,
            is_guest: true,
        };
        assert!(!guest.is_recoverable());
        assert_eq!(guest.label(), "Guest");

        let member = Account {
            id: "app_user:abc".into(),
            display_name: "Guest".into(),
            email: Some("viewer@example.com".into()),
            is_guest: false,
        };
        assert!(member.is_recoverable());
        assert_eq!(member.label(), "viewer@example.com");
    }
}

// ---------------------------------------------------------------------------
// SponsorBlock
// ---------------------------------------------------------------------------
