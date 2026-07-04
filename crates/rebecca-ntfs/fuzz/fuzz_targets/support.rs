pub fn corpus_bytes(data: &[u8]) -> Vec<u8> {
    let Ok(text) = std::str::from_utf8(data) else {
        return data.to_vec();
    };
    let Some(hex) = text.trim_start().strip_prefix("hex:") else {
        return data.to_vec();
    };

    let mut nibbles = Vec::new();
    for byte in hex.bytes() {
        match byte {
            b'0'..=b'9' => nibbles.push(byte - b'0'),
            b'a'..=b'f' => nibbles.push(byte - b'a' + 10),
            b'A'..=b'F' => nibbles.push(byte - b'A' + 10),
            b' ' | b'\r' | b'\n' | b'\t' | b'_' => {}
            _ => return data.to_vec(),
        }
    }
    if nibbles.len() % 2 != 0 {
        return data.to_vec();
    }

    nibbles
        .chunks_exact(2)
        .map(|pair| (pair[0] << 4) | pair[1])
        .collect()
}
