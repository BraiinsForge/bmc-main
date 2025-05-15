// Copyright (C) 2023  Braiins Systems s.r.o.

use serde::{Deserialize, Deserializer};

pub fn trim_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value: String = Deserialize::deserialize(deserializer)?;
    Ok(value.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trim_string() {
        #[derive(Deserialize)]
        struct Foo {
            #[serde(deserialize_with = "trim_string")]
            bar: String,
        }
        let json = r#"{"bar": "   baz   "}"#;
        let foo = serde_json::from_str::<Foo>(json).expect("BUG: failed to deserialize JSON");
        assert_eq!(foo.bar, "baz");
    }
}
