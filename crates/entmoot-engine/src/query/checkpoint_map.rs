//! JSON checkpoints encode compound map keys as pairs, not lossy strings.
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{collections::HashMap, hash::Hash};

pub fn serialize<K: Serialize, V: Serialize, S: Serializer>(
    map: &HashMap<K, V>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    map.iter().collect::<Vec<_>>().serialize(serializer)
}

pub fn deserialize<
    'de,
    K: Deserialize<'de> + Eq + Hash,
    V: Deserialize<'de>,
    D: Deserializer<'de>,
>(
    deserializer: D,
) -> Result<HashMap<K, V>, D::Error> {
    let pairs = Vec::<(K, V)>::deserialize(deserializer)?;
    let size = pairs.len();
    let result: HashMap<_, _> = pairs.into_iter().collect();
    if result.len() != size {
        return Err(serde::de::Error::custom("duplicate checkpoint map key"));
    }
    Ok(result)
}
