//! Verify yt-dlp's detached OpenPGP signature of its checksum list.
//!
//! The GitHub asset digest and `SHA2-256SUMS` both come from the same release,
//! so only this signature proves the list came from the yt-dlp signing key.
//! Accepted subset: a v4 binary-document RSA signature with SHA-256 or SHA-512.
//! Trailing packets, other versions and other algorithms are rejected.
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use ring::signature::{
    RsaPublicKeyComponents, RSA_PKCS1_2048_8192_SHA256, RSA_PKCS1_2048_8192_SHA512,
};

/// Primary key AC0C BBE6 848D 6A87 3464  AF4E 57CF 6593 3B5A 7581 from
/// https://github.com/yt-dlp/yt-dlp/blob/master/public.key
const YT_DLP_RSA_MODULUS_BASE64: &str = "9Kxfc4xjwLdLYZbeQtXm83HAFV+zW9b/nqkI5PlhGXUxE1QoN1gDOZ4AK5R3Zv32as1IDvqQGKwetBinoO331A3/ktFxShGXXkS2HgGqqioUom1WpQ47zgTs7bt1utOxyxE0AKgO8Ez7iLdlqpqS+tDSHMq9uy8apoHTd5jb2doizvmz9Jykfwvj4BPR+MrcsA2tdszMej5mQp1Tjjrul2RBZnVMcXUPXxQ6nfS3ASmNiwznEmKNbGmNGFomVh7c1aGbtZDs450uRY6v8gRySyJgx+6TcR5u7Pz52rq6ZkBbQxa7pLftpYQ56SlJhq/yP9yB6h/1/vZz6EuLoqkOR6W5QfASvuFn6WohIO+S577jj32SDymMl2v9AO0KIzMIDqw/PKM0/lQpCpJCfoo12e4BzY5nAWlHSiu3wyBDmCOty8nOkFpBkE4oBX30lF1hwwjvWlzYT93o6hVoBjLQlLK25HCZFrlmO6uQEcv9OR20jNyuA1PXqrZ8ZoldVkPJ17iPSYCUEb4eT05J8uS7cas3Jqr1jvU4xwW67NeATxDvX3F8Si/whQW81w0r7wIAK3aqpTKJ3EP8onE5Xy+qsyfBChYqlteEDE3QJ+krrmU6HVwk4iJ8KE+1o47hTyinb3x9Rj+G56nc058y8QkgyBLX8nz7CF/vRiOyZJP82Y0=";
const YT_DLP_RSA_EXPONENT: [u8; 3] = [1, 0, 1];
const SIGNATURE_PACKET_TAG: u8 = 2;
const MAX_SIGNATURE_BYTES: usize = 8192;

struct SignaturePacket<'a> {
    hash_algorithm: u8,
    hashed_section: &'a [u8],
    signature: Vec<u8>,
}

fn signature_body(input: &[u8]) -> Option<&[u8]> {
    let header = *input.first()?;
    if header & 0x80 == 0 {
        return None;
    }
    let (tag, length, offset) = if header & 0x40 != 0 {
        let first = *input.get(1)? as usize;
        match first {
            0..=191 => (header & 0x3f, first, 2),
            192..=223 => (
                header & 0x3f,
                ((first - 192) << 8) + *input.get(2)? as usize + 192,
                3,
            ),
            255 => (
                header & 0x3f,
                u32::from_be_bytes(input.get(2..6)?.try_into().ok()?) as usize,
                6,
            ),
            _ => return None,
        }
    } else {
        let length_bytes = match header & 3 {
            0 => 1,
            1 => 2,
            2 => 4,
            _ => return None,
        };
        let length = input
            .get(1..1 + length_bytes)?
            .iter()
            .fold(0_usize, |total, byte| (total << 8) | *byte as usize);
        ((header >> 2) & 0x0f, length, 1 + length_bytes)
    };
    if tag != SIGNATURE_PACKET_TAG || offset.checked_add(length)? != input.len() {
        return None;
    }
    input.get(offset..)
}

fn parse_signature(input: &[u8]) -> Option<SignaturePacket<'_>> {
    let body = signature_body(input)?;
    let (version, signature_type, public_key_algorithm, hash_algorithm) =
        (*body.first()?, *body.get(1)?, *body.get(2)?, *body.get(3)?);
    if version != 4 || signature_type != 0 || public_key_algorithm != 1 {
        return None;
    }
    let hashed_length = u16::from_be_bytes(body.get(4..6)?.try_into().ok()?) as usize;
    let hashed_end = 6_usize.checked_add(hashed_length)?;
    let hashed_section = body.get(..hashed_end)?;
    let unhashed_length =
        u16::from_be_bytes(body.get(hashed_end..hashed_end + 2)?.try_into().ok()?) as usize;
    // Skip the unhashed subpackets and the 16-bit hash prefix before the MPI.
    let mpi_start = hashed_end
        .checked_add(2)?
        .checked_add(unhashed_length)?
        .checked_add(2)?;
    let bits = u16::from_be_bytes(body.get(mpi_start..mpi_start + 2)?.try_into().ok()?) as usize;
    let signature = body.get(mpi_start + 2..)?;
    if signature.len() != bits.div_ceil(8) {
        return None;
    }
    Some(SignaturePacket {
        hash_algorithm,
        hashed_section,
        signature: signature.to_vec(),
    })
}

pub(crate) fn verify_checksum_list(checksums: &[u8], signature: &[u8]) -> Result<(), String> {
    const INVALID: &str = "Підпис списку контрольних сум yt-dlp не пройшов перевірку";
    if signature.len() > MAX_SIGNATURE_BYTES {
        return Err(INVALID.into());
    }
    let packet = parse_signature(signature).ok_or(INVALID)?;
    let modulus = BASE64_STANDARD
        .decode(YT_DLP_RSA_MODULUS_BASE64)
        .map_err(|_| INVALID)?;
    // An MPI drops leading zero bytes; RSA verification expects the full width.
    let padding = modulus
        .len()
        .checked_sub(packet.signature.len())
        .ok_or(INVALID)?;
    let mut padded = vec![0_u8; padding];
    padded.extend_from_slice(&packet.signature);
    let mut message = Vec::with_capacity(checksums.len() + packet.hashed_section.len() + 6);
    message.extend_from_slice(checksums);
    message.extend_from_slice(packet.hashed_section);
    message.extend_from_slice(&[0x04, 0xff]);
    message.extend_from_slice(&(packet.hashed_section.len() as u32).to_be_bytes());
    let algorithm = match packet.hash_algorithm {
        8 => &RSA_PKCS1_2048_8192_SHA256,
        10 => &RSA_PKCS1_2048_8192_SHA512,
        _ => return Err(INVALID.into()),
    };
    RsaPublicKeyComponents {
        n: modulus.as_slice(),
        e: YT_DLP_RSA_EXPONENT.as_slice(),
    }
    .verify(algorithm, &message, &padded)
    .map_err(|_| INVALID.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL_SUMS: &[u8] = include_bytes!("testdata/yt-dlp-SHA2-256SUMS");
    const REAL_SIGNATURE: &[u8] = include_bytes!("testdata/yt-dlp-SHA2-256SUMS.sig");

    #[test]
    fn accepts_a_real_official_nightly_signature() {
        assert!(verify_checksum_list(REAL_SUMS, REAL_SIGNATURE).is_ok());
    }

    #[test]
    fn rejects_a_changed_checksum_list() {
        let mut changed = REAL_SUMS.to_vec();
        changed[0] ^= 1;
        assert!(verify_checksum_list(&changed, REAL_SIGNATURE).is_err());
    }

    #[test]
    fn rejects_damaged_or_extended_signatures() {
        let mut flipped = REAL_SIGNATURE.to_vec();
        *flipped.last_mut().unwrap() ^= 1;
        assert!(verify_checksum_list(REAL_SUMS, &flipped).is_err());
        let mut trailing = REAL_SIGNATURE.to_vec();
        trailing.push(0);
        assert!(verify_checksum_list(REAL_SUMS, &trailing).is_err());
        assert!(verify_checksum_list(REAL_SUMS, &REAL_SIGNATURE[..40]).is_err());
        assert!(verify_checksum_list(REAL_SUMS, b"").is_err());
        assert!(verify_checksum_list(REAL_SUMS, &vec![0; MAX_SIGNATURE_BYTES + 1]).is_err());
    }
}
