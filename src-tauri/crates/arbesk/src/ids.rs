//! A random account id, the same shape the app uses for its other accounts:
//! 16 lower-case hex digits, or the clock if the system has no randomness.

pub(crate) fn new_account_id() -> String {
    let mut bytes = [0u8; 8];
    if getrandom::fill(&mut bytes).is_err() {
        // Extremely unlikely; the clock still gives a usable number.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        return format!("{nanos:x}");
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
