//! Nibble encoding for Merkle Patricia Trie keys (two nibbles per path byte).

/// Converts a CADO path byte slice into trie-db nibble key form.
pub fn path_to_nibbles(path: &[u8]) -> Vec<u8> {
    let mut nibbles = Vec::with_capacity(path.len() * 2);
    for &byte in path {
        nibbles.push(byte >> 4);
        nibbles.push(byte & 0x0F);
    }
    nibbles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_to_nibbles_encodes_bytes() {
        assert_eq!(path_to_nibbles(b"ab"), vec![6, 1, 6, 2]);
        assert_eq!(path_to_nibbles(&[0xFF]), vec![0x0F, 0x0F]);
    }
}
