// iridium-core: domain types shared across all crates.
// Populated incrementally — see docs/adr/ for design decisions.

/// Supported output image formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ImageFormat {
    Raw,
    Ewf,
    Aff,
}

/// Supported hash algorithms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum HashAlg {
    Md5,
    Sha1,
    Sha256,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_format_roundtrip() {
        let fmt = ImageFormat::Ewf;
        let json = serde_json::to_string(&fmt).unwrap();
        assert_eq!(serde_json::from_str::<ImageFormat>(&json).unwrap(), fmt);
    }
}
