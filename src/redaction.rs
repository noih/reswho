pub(crate) const SENSITIVE_NAMES: &[&str] = &[
    "password",
    "passwd",
    "pwd",
    "passphrase",
    "passkey",
    "token",
    "secret",
    "credential",
    "authorization",
    "apikey",
    "accesskey",
    "privatekey",
    "cookie",
    "header",
    "session",
    "auth",
    "bearer",
];

pub(crate) fn normalized_option(value: &str) -> String {
    value
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect()
}

pub(crate) fn sensitive_option(value: &str) -> bool {
    let normalized = normalized_option(value);
    SENSITIVE_NAMES.iter().any(|key| normalized.contains(key))
        || matches!(normalized.as_str(), "key" | "pass" | "user" | "env")
        || matches!(value, "-p" | "-u" | "-H" | "-a" | "-E")
}
