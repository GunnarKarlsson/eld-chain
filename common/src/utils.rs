use crate::error::EldError;
use serde::Serialize;

pub fn to_json_string<T: Serialize>(value: &T) -> Result<String, EldError> {
    serde_json::to_string(value).map_err(|e| EldError::BasicValidationError {
        details: e.to_string(),
    })
}

pub fn to_json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, EldError> {
    serde_json::to_vec(value).map_err(|e| EldError::BasicValidationError {
        details: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    #[derive(Serialize)]
    struct Sample {
        name: String,
        count: u32,
    }

    #[test]
    fn to_json_string_serializes_struct() {
        let sample = Sample {
            name: "eld".to_string(),
            count: 2,
        };
        let json = to_json_string(&sample).expect("serialize to string");
        assert_eq!(json, r#"{"name":"eld","count":2}"#);
    }

    #[test]
    fn to_json_bytes_matches_string_utf8() {
        let sample = Sample {
            name: "eld".to_string(),
            count: 2,
        };
        let bytes = to_json_bytes(&sample).expect("serialize to bytes");
        let json = to_json_string(&sample).expect("serialize to string");
        assert_eq!(bytes, json.as_bytes());
    }
}
