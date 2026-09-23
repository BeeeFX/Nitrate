//! Checking a detached OpenPGP signature against one pinned RSA key.
//!
//! Just enough of RFC 4880 for yt-dlp's releases: a version 4 signature over
//! a binary document, by an RSA key, hashed with SHA-256 or SHA-512. A general
//! OpenPGP library would take ninety-odd crates to do this one thing.
//!
//! The RSA step builds the only encoding a valid signature can produce and
//! compares the whole thing, byte for byte, rather than parsing what the
//! signature decrypts to. Parsing it is where forged signatures have always
//! slipped through.
//!
//! Everything is bounds-checked: the release build aborts on a panic, and a
//! malformed signature has to be an error, not a crash.

use num_bigint::BigUint;
use sha2::{Digest, Sha256, Sha512};

pub struct RsaKey {
    /// The modulus, as hex.
    pub modulus: &'static str,
    pub exponent: u32,
}

/// ASN.1 DigestInfo prefixes, from RFC 8017 §9.2.
const SHA256_PREFIX: &[u8] = &[
    0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01, 0x05,
    0x00, 0x04, 0x20,
];
const SHA512_PREFIX: &[u8] = &[
    0x30, 0x51, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x03, 0x05,
    0x00, 0x04, 0x40,
];

/// Whether `signature` is `key`'s signature over `data`.
pub fn verify(data: &[u8], signature: &[u8], key: &RsaKey) -> Result<(), String> {
    let bad = |why: &str| format!("The signature {why}.");

    let (tag, body) = packet(signature).ok_or_else(|| bad("is malformed"))?;
    if tag != 2 {
        return Err(bad("isn't a signature"));
    }

    let mut rest = body;
    let head = take(&mut rest, 4).ok_or_else(|| bad("is malformed"))?;
    let (version, kind, algorithm, hash) = (head[0], head[1], head[2], head[3]);
    if version != 4 {
        return Err(bad("is a version this can't check"));
    }
    if kind != 0x00 {
        return Err(bad("isn't over a file"));
    }
    if algorithm != 1 {
        return Err(bad("isn't an RSA signature"));
    }

    let hashed_len = u16_be(take(&mut rest, 2)).ok_or_else(|| bad("is malformed"))?;
    take(&mut rest, hashed_len).ok_or_else(|| bad("is malformed"))?;
    // Everything from the version to the end of the hashed subpackets is
    // signed along with the data.
    let signed_part = &body[..6 + hashed_len];

    let unhashed_len = u16_be(take(&mut rest, 2)).ok_or_else(|| bad("is malformed"))?;
    take(&mut rest, unhashed_len).ok_or_else(|| bad("is malformed"))?;
    let quick_check = take(&mut rest, 2).ok_or_else(|| bad("is malformed"))?;
    let bits = u16_be(take(&mut rest, 2)).ok_or_else(|| bad("is malformed"))?;
    let value = take(&mut rest, bits.div_ceil(8)).ok_or_else(|| bad("is malformed"))?;

    let mut trailer = signed_part.to_vec();
    trailer.extend_from_slice(&[0x04, 0xff]);
    trailer.extend_from_slice(&(signed_part.len() as u32).to_be_bytes());

    let (digest, prefix): (Vec<u8>, &[u8]) = match hash {
        8 => (
            Sha256::new()
                .chain_update(data)
                .chain_update(&trailer)
                .finalize()
                .to_vec(),
            SHA256_PREFIX,
        ),
        10 => (
            Sha512::new()
                .chain_update(data)
                .chain_update(&trailer)
                .finalize()
                .to_vec(),
            SHA512_PREFIX,
        ),
        _ => return Err(bad("uses a hash this can't check")),
    };
    if digest[..2] != *quick_check {
        return Err(bad("doesn't match the file"));
    }

    let modulus =
        BigUint::parse_bytes(key.modulus.as_bytes(), 16).ok_or_else(|| bad("key is malformed"))?;
    let width = modulus.bits().div_ceil(8) as usize;
    let s = BigUint::from_bytes_be(value);
    if s >= modulus {
        return Err(bad("is malformed"));
    }
    let decoded = s
        .modpow(&BigUint::from(key.exponent), &modulus)
        .to_bytes_be();

    // EMSA-PKCS1-v1_5: 00 01 FF…FF 00 DigestInfo digest, the full key width.
    let filler = width
        .checked_sub(3 + prefix.len() + digest.len())
        .ok_or_else(|| bad("key is too short"))?;
    let mut expected = vec![0x00, 0x01];
    expected.extend(std::iter::repeat_n(0xff, filler));
    expected.push(0x00);
    expected.extend_from_slice(prefix);
    expected.extend_from_slice(&digest);

    // `to_bytes_be` drops leading zeros, and the encoding always has one.
    let mut padded = vec![0u8; width.saturating_sub(decoded.len())];
    padded.extend_from_slice(&decoded);

    if padded == expected {
        Ok(())
    } else {
        Err(bad("doesn't match the file"))
    }
}

/// The first packet's tag and body, in either header format.
fn packet(bytes: &[u8]) -> Option<(u8, &[u8])> {
    let mut rest = bytes;
    let first = *take(&mut rest, 1)?.first()?;
    if first & 0x80 == 0 {
        return None;
    }
    let (tag, len) = if first & 0x40 != 0 {
        let tag = first & 0x3f;
        let octet = *take(&mut rest, 1)?.first()? as usize;
        let len = match octet {
            0..=191 => octet,
            192..=223 => ((octet - 192) << 8) + *take(&mut rest, 1)?.first()? as usize + 192,
            255 => u32::from_be_bytes(take(&mut rest, 4)?.try_into().ok()?) as usize,
            _ => return None,
        };
        (tag, len)
    } else {
        let tag = (first >> 2) & 0x0f;
        let len = match first & 0x03 {
            0 => *take(&mut rest, 1)?.first()? as usize,
            1 => u16_be(take(&mut rest, 2))?,
            2 => u32::from_be_bytes(take(&mut rest, 4)?.try_into().ok()?) as usize,
            _ => return None,
        };
        (tag, len)
    };
    Some((tag, take(&mut rest, len)?))
}

fn take<'a>(rest: &mut &'a [u8], n: usize) -> Option<&'a [u8]> {
    if rest.len() < n {
        return None;
    }
    let (head, tail) = rest.split_at(n);
    *rest = tail;
    Some(head)
}

fn u16_be(bytes: Option<&[u8]>) -> Option<usize> {
    let bytes = bytes?;
    Some(u16::from_be_bytes([*bytes.first()?, *bytes.get(1)?]) as usize)
}
