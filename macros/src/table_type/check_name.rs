/// PostgreSQL silently truncates longer identifiers, which would make the
/// declared name differ from the stored name and produce permanent drift.
pub(crate) const MAX_IDENTIFIER_BYTES: usize = 63;

pub(crate) fn stable_check_name(prefix: &str, predicate: &str) -> String {
    stable_schema_name(prefix, predicate, "check")
}

pub(crate) fn stable_index_name(prefix: &str, predicate: Option<&str>, unique: bool) -> String {
    let identity = format!(
        "{}:{}",
        if unique { "unique" } else { "index" },
        predicate.unwrap_or_default()
    );

    let kind = if predicate.is_some() { "partial_idx" } else { "idx" };

    stable_schema_name(prefix, &identity, kind)
}

/// Builds a deterministic PostgreSQL identifier. FNV-1a is fixed here so
/// schema names never depend on process-randomized hashing or compiler details.
fn stable_schema_name(prefix: &str, identity: &str, kind: &str) -> String {
    let digest = identity.as_bytes()
        .iter()
        .fold(0xcbf29ce484222325u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });

    let suffix = format!("_{digest:016x}_{kind}");
    let max_prefix_bytes = MAX_IDENTIFIER_BYTES - suffix.len();
    let mut end = prefix.len()
        .min(max_prefix_bytes);

    while !prefix.is_char_boundary(end) {
        end -= 1;
    }

    let prefix = prefix[..end].trim_end_matches('_');

    format!("{prefix}{suffix}")
}
