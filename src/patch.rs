use serde::{Deserialize, Deserializer};

/// Deserializes a present nullable patch field as `Some(value)`, including
/// `Some(None)` for JSON null. The field's `default` handles omission as None.
pub fn deserialize_nullable_patch<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}
