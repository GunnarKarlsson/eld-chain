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

    fn vec_u8_to_u32(bytes: Vec<u8>) -> u32 {
        assert!(
            bytes.len() <= 4,
            "Vec<u8> must not be larger than 4 bytes to convert to u32"
        );
        let mut result: u32 = 0;
        for &byte in &bytes {
            result = (result << 8) | (byte as u32);
        }
        result
    }

    fn vec_u8_to_u32_safe(bytes: Vec<u8>) -> Result<u32, EldError> {
        if bytes.len() > 4 {
            return Err(EldError::ValidationError {
                field: "bytes".to_string(),
                value: format!("{} bytes", bytes.len()),
                details: "Vec<u8> must not be larger than 4 bytes to convert to u32".to_string(),
            });
        }
        let mut result: u32 = 0;
        for &byte in &bytes {
            result = (result << 8) | (byte as u32);
        }
        Ok(result)
    }

    fn vec_u8_to_u32_little_endian(bytes: Vec<u8>) -> u32 {
        assert!(
            bytes.len() <= 4,
            "Vec<u8> must not be larger than 4 bytes to convert to u32"
        );
        let mut result: u32 = 0;
        for &byte in bytes.iter().rev() {
            result = (result << 8) | (byte as u32);
        }
        result
    }

    #[test]
    fn test_vec_u8_to_u32_basic() {
        assert_eq!(vec_u8_to_u32(vec![0x12]), 0x12);
        assert_eq!(vec_u8_to_u32(vec![0x12, 0x34]), 0x1234);
        assert_eq!(vec_u8_to_u32(vec![0x12, 0x34, 0x56]), 0x123456);
        assert_eq!(vec_u8_to_u32(vec![0x12, 0x34, 0x56, 0x78]), 0x12345678);
        assert_eq!(vec_u8_to_u32(vec![]), 0); // Empty vector
    }

    #[test]
    #[should_panic(expected = "Vec<u8> must not be larger than 4 bytes")]
    fn test_vec_u8_to_u32_too_long() {
        vec_u8_to_u32(vec![0x12, 0x34, 0x56, 0x78, 0x90]);
    }

    #[test]
    fn test_vec_u8_to_u32_safe() {
        assert_eq!(
            vec_u8_to_u32_safe(vec![0x12]).expect("Failed to convert single byte to u32"),
            0x12
        );
        assert_eq!(
            vec_u8_to_u32_safe(vec![0x12, 0x34]).expect("Failed to convert two bytes to u32"),
            0x1234
        );
        assert_eq!(
            vec_u8_to_u32_safe(vec![0x12, 0x34, 0x56])
                .expect("Failed to convert three bytes to u32"),
            0x123456
        );
        assert_eq!(
            vec_u8_to_u32_safe(vec![0x12, 0x34, 0x56, 0x78])
                .expect("Failed to convert four bytes to u32"),
            0x12345678
        );
        assert_eq!(
            vec_u8_to_u32_safe(vec![]).expect("Failed to convert empty vector to u32"),
            0
        );

        assert!(vec_u8_to_u32_safe(vec![0x12, 0x34, 0x56, 0x78, 0x90]).is_err());
        match vec_u8_to_u32_safe(vec![0x12, 0x34, 0x56, 0x78, 0x90]).unwrap_err() {
            EldError::ValidationError {
                field,
                value,
                details,
            } => {
                assert_eq!(field, "bytes");
                assert_eq!(value, "5 bytes");
                assert_eq!(
                    details,
                    "Vec<u8> must not be larger than 4 bytes to convert to u32"
                );
            }
            _ => panic!("Expected ValidationError"),
        }
    }

    #[test]
    fn test_vec_u8_to_u32_little_endian() {
        assert_eq!(vec_u8_to_u32_little_endian(vec![0x12]), 0x12);
        assert_eq!(vec_u8_to_u32_little_endian(vec![0x12, 0x34]), 0x3412);
        assert_eq!(
            vec_u8_to_u32_little_endian(vec![0x12, 0x34, 0x56]),
            0x563412
        );
        assert_eq!(
            vec_u8_to_u32_little_endian(vec![0x12, 0x34, 0x56, 0x78]),
            0x78563412
        );
        assert_eq!(vec_u8_to_u32_little_endian(vec![]), 0);
    }

    #[test]
    #[should_panic(expected = "Vec<u8> must not be larger than 4 bytes")]
    fn test_vec_u8_to_u32_little_endian_too_long() {
        vec_u8_to_u32_little_endian(vec![0x12, 0x34, 0x56, 0x78, 0x90]);
    }

    #[test]
    fn test_max_values() {
        // Test maximum u8 value (255) in different positions
        assert_eq!(vec_u8_to_u32(vec![0xFF]), 0xFF);
        assert_eq!(vec_u8_to_u32(vec![0xFF, 0xFF]), 0xFFFF);
        assert_eq!(vec_u8_to_u32(vec![0xFF, 0xFF, 0xFF]), 0xFFFFFF);
        assert_eq!(vec_u8_to_u32(vec![0xFF, 0xFF, 0xFF, 0xFF]), 0xFFFFFFFF);
    }
}
