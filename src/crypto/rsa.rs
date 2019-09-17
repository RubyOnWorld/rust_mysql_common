use super::der;
use byteorder::{BigEndian, ByteOrder};
use num_bigint::BigUint;
use rand::Rng;
use sha1::Sha1;

/// Padding operation trait.
pub trait Padding {
    /// Padding operation for `input` bytes, where `k` is the length of modulus in octets.
    fn pub_pad(&mut self, input: impl AsRef<[u8]>, k: usize) -> Vec<u8>;
}

/// Padding, as described in PKCS #1: RSA Encryption Version 1.5 (rfc2313).
#[derive(Debug)]
pub struct Pkcs1Padding<T> {
    rng: T,
}

impl<T> Pkcs1Padding<T> {
    pub fn new(rng: T) -> Self {
        Self { rng }
    }
}

impl<T: Rng> Padding for Pkcs1Padding<T> {
    fn pub_pad(&mut self, input: impl AsRef<[u8]>, k: usize) -> Vec<u8> {
        let input = input.as_ref();
        let input_len = input.len();
        assert!(
            input_len < k - 11,
            "The length of the data D shall not be more than k-11 octets"
        );

        let mut output = vec![0u8; k];

        output[0] = 0x00;
        output[1] = 0x02;
        let ps_len = k - 3 - input_len;

        for i in 0..ps_len {
            let x = loop {
                match self.rng.gen::<u8>() {
                    0x00 => continue,
                    x => break x,
                }
            };
            output[i + 2] = x;
        }

        output[2 + ps_len] = 0x00;
        (&mut output[2 + ps_len + 1..]).copy_from_slice(input);
        output
    }
}

/// Padding, as described in PKCS #1: RSA Cryptography Specifications Version 2.0 (rfc2437).
#[derive(Debug)]
pub struct Pkcs1OaepPadding<T> {
    rng: T,
}

impl<T> Pkcs1OaepPadding<T> {
    /// Length of a SHA-1 hash digest.
    const HASH_LEN: usize = 20;

    pub fn new(rng: T) -> Self {
        Self { rng }
    }

    /// Mask Generation Function as defined in rfc2437.
    ///
    /// It will use SHA-1 as a hash function.
    fn mgf1(seed: &[u8], len: usize) -> Vec<u8> {
        if len > 2usize.pow(32) * Self::HASH_LEN {
            panic!("mask too long");
        }

        fn ceil_div(dividend: usize, divisor: usize) -> usize {
            let mut quotient = dividend / divisor;
            if dividend % divisor > 0 {
                quotient += 1;
            }
            quotient
        }

        let output = (0..ceil_div(len, Self::HASH_LEN))
            .map(|c| {
                let cs = &mut [0u8; 4];
                BigEndian::write_u32(cs, c as u32);
                Vec::from(&Sha1::from(&*[seed, cs].concat()).digest().bytes()[..])
            })
            .collect::<Vec<Vec<u8>>>()
            .concat();

        output[..len].into()
    }
}

impl<T: Rng> Padding for Pkcs1OaepPadding<T> {
    /// Will pad input according to PKCS #1 v2 with encoding parameters equal to `[]`.
    fn pub_pad(&mut self, input: impl AsRef<[u8]>, k: usize) -> Vec<u8> {
        let input = input.as_ref();
        // 1. Skip because encoding parameters == []
        // 2. If ||M|| > emLen-2hLen-1 then output "message too long" and stop.
        if input.len() > k - 2 * Self::HASH_LEN - 1 {
            panic!("message too long");
        }
        // 3. Generate an octet string PS consisting of emLen-||M||-2hLen-1 zero
        //    octets. The length of PS may be 0.
        let mut ps = vec![0; k - input.len() - 2 * Self::HASH_LEN - 2];
        ps.push(0x01);
        // 4. Let pHash = Hash(P), an octet string of length hLen.
        let p_hash = Vec::from(&Sha1::default().digest().bytes()[..]);
        // 5. Concatenate pHash, PS, the message M, and other padding to form a
        //    data block DB as: DB = pHash || PS || 01 || M
        let db = [&*p_hash, &*ps, input].concat();
        // 6. Generate a random octet string seed of length hLen.
        let seed: Vec<_> = (0..Self::HASH_LEN).map(|_| self.rng.gen()).collect();
        // 7. Let dbMask = MGF(seed, emLen-hLen).
        let db_mask = Self::mgf1(&*seed, k - Self::HASH_LEN);
        // 8. Let maskedDB = DB \xor dbMask.
        let masked_db: Vec<_> = db
            .into_iter()
            .zip(db_mask.into_iter())
            .map(|(a, b)| a ^ b)
            .collect();
        // 9. Let seedMask = MGF(maskedDB, hLen).
        let seed_mask = Self::mgf1(&*masked_db, Self::HASH_LEN);
        // 10. Let maskedSeed = seed \xor seedMask.
        let masked_seed: Vec<_> = seed
            .into_iter()
            .zip(seed_mask.into_iter())
            .map(|(a, b)| a ^ b)
            .collect();
        // 11. Let EM = maskedSeed || maskedDB.
        [&*masked_seed, &*masked_db].concat()
    }
}

#[derive(Debug)]
pub struct PublicKey {
    modulus: BigUint,
    exponent: BigUint,
}

impl PublicKey {
    /// Basic constructor.
    pub fn new(modulus: BigUint, exponent: BigUint) -> PublicKey {
        PublicKey { modulus, exponent }
    }

    /// Will parse public key from pem representation.
    ///
    /// # Panic
    ///
    /// Will panic in case of bad pem data.
    pub fn from_pem(pem_data: impl AsRef<[u8]>) -> PublicKey {
        let (der, file_type) = der::pem_to_der(pem_data);
        let (modulus, exponent) = der::parse_pub_key(&*der, file_type);
        PublicKey::new(modulus, exponent)
    }

    /// Returns number of octets in the modulus.
    pub fn num_octets(&self) -> usize {
        (self.modulus.bits() + 6) >> 3
    }

    /// Returns modulus of the public key.
    pub fn modulus(&self) -> &BigUint {
        &self.modulus
    }

    /// Returns exponent of the public key.
    pub fn exponent(&self) -> &BigUint {
        &self.exponent
    }

    /// Will encrypt block with public key.
    ///
    /// # Panic
    ///
    /// Will panic if block is too long for key or padding.
    pub fn encrypt_block(&self, block: impl AsRef<[u8]>, mut pad: impl Padding) -> Vec<u8> {
        let enc_block = pad.pub_pad(block, self.num_octets());
        let enc_int = BigUint::from_bytes_be(&*enc_block);
        let rsa = enc_int.modpow(self.exponent(), self.modulus());
        let mut rsa_bytes = rsa.to_bytes_be();
        // is this needed?
        while rsa_bytes.len() < self.num_octets() {
            rsa_bytes.insert(0, 0);
        }
        rsa_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::adapter::ReadRng;

    const SEED: &[u8; 64] = b"\x03\x2e\x45\x32\x6f\xa8\x59\xa7\x2e\xc2\x35\xac\xff\x92\x9b\x15\xd1\
    \x37\x2e\x30\xb2\x07\x25\x5f\x06\x11\xb8\xf7\x85\xd7\x64\x37\x41\x52\xe0\xac\x00\x9e\x50\x9e\
    \x7b\xa3\x0c\xd2\xf1\x77\x8e\x11\x3b\x64\xe1\x35\xcf\x4e\x22\x92\xc7\x5e\xfe\x52\x88\xed\xfd\
    \xa4";
    const MASK: &[u8; 128] =
        b"\x5f\x8d\xe1\x05\xb5\xe9\x6b\x2e\x49\x0d\xde\xcb\xd1\x47\xdd\x1d\xef\
    \x7e\x3b\x8e\x0e\x6a\x26\xeb\x7b\x95\x6c\xcb\x8b\x3b\xdc\x1c\xa9\x75\xbc\x57\xc3\x98\x9e\x8f\
    \xba\xd3\x1a\x22\x46\x55\xd8\x00\xc4\x69\x54\x84\x0f\xf3\x20\x52\xcd\xf0\xd6\x40\x56\x2b\xdf\
    \xad\xfa\x26\x3c\xfc\xcf\x3c\x52\xb2\x9f\x2a\xf4\xa1\x86\x99\x59\xbc\x77\xf8\x54\xcf\x15\xbd\
    \x7a\x25\x19\x29\x85\xa8\x42\xdb\xff\x8e\x13\xef\xee\x5b\x7e\x7e\x55\xbb\xe4\xd3\x89\x64\x7c\
    \x68\x6a\x9a\x9a\xb3\xfb\x88\x9b\x2d\x77\x67\xd3\x83\x7e\xea\x4e\x0a\x2f\x04";

    #[test]
    fn mgf1() {
        let mask = Pkcs1OaepPadding::<()>::mgf1(&SEED[..], 128);
        assert_eq!(mask, &MASK[..]);
    }

    #[test]
    fn rsa_pkcs() {
        let modulus = vec![
            0xa8, 0xb3, 0xb2, 0x84, 0xaf, 0x8e, 0xb5, 0x0b, 0x38, 0x70, 0x34, 0xa8, 0x60, 0xf1,
            0x46, 0xc4, 0x91, 0x9f, 0x31, 0x87, 0x63, 0xcd, 0x6c, 0x55, 0x98, 0xc8, 0xae, 0x48,
            0x11, 0xa1, 0xe0, 0xab, 0xc4, 0xc7, 0xe0, 0xb0, 0x82, 0xd6, 0x93, 0xa5, 0xe7, 0xfc,
            0xed, 0x67, 0x5c, 0xf4, 0x66, 0x85, 0x12, 0x77, 0x2c, 0x0c, 0xbc, 0x64, 0xa7, 0x42,
            0xc6, 0xc6, 0x30, 0xf5, 0x33, 0xc8, 0xcc, 0x72, 0xf6, 0x2a, 0xe8, 0x33, 0xc4, 0x0b,
            0xf2, 0x58, 0x42, 0xe9, 0x84, 0xbb, 0x78, 0xbd, 0xbf, 0x97, 0xc0, 0x10, 0x7d, 0x55,
            0xbd, 0xb6, 0x62, 0xf5, 0xc4, 0xe0, 0xfa, 0xb9, 0x84, 0x5c, 0xb5, 0x14, 0x8e, 0xf7,
            0x39, 0x2d, 0xd3, 0xaa, 0xff, 0x93, 0xae, 0x1e, 0x6b, 0x66, 0x7b, 0xb3, 0xd4, 0x24,
            0x76, 0x16, 0xd4, 0xf5, 0xba, 0x10, 0xd4, 0xcf, 0xd2, 0x26, 0xde, 0x88, 0xd3, 0x9f,
            0x16, 0xfb,
        ];
        let exponent = vec![0x01, 0x00, 0x01];

        let msg1 = vec![
            0x66, 0x28, 0x19, 0x4e, 0x12, 0x07, 0x3d, 0xb0, 0x3b, 0xa9, 0x4c, 0xda, 0x9e, 0xf9,
            0x53, 0x23, 0x97, 0xd5, 0x0d, 0xba, 0x79, 0xb9, 0x87, 0x00, 0x4a, 0xfe, 0xfe, 0x34,
        ];
        let seed1 = vec![
            0x01, 0x00, 0x00, 0x00, 0x73, 0x00, 0x00, 0x00, 0x41, 0x00, 0x00, 0x00, 0xae, 0x00,
            0x00, 0x00, 0x38, 0x00, 0x00, 0x00, 0x75, 0x00, 0x00, 0x00, 0xd5, 0x00, 0x00, 0x00,
            0xf8, 0x00, 0x00, 0x00, 0x71, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0xf8, 0x00,
            0x00, 0x00, 0xcc, 0x00, 0x00, 0x00, 0x4f, 0x00, 0x00, 0x00, 0xa9, 0x00, 0x00, 0x00,
            0xb9, 0x00, 0x00, 0x00, 0xbc, 0x00, 0x00, 0x00, 0x15, 0x00, 0x00, 0x00, 0x6b, 0x00,
            0x00, 0x00, 0xb0, 0x00, 0x00, 0x00, 0x46, 0x00, 0x00, 0x00, 0x28, 0x00, 0x00, 0x00,
            0xfc, 0x00, 0x00, 0x00, 0xcd, 0x00, 0x00, 0x00, 0xb2, 0x00, 0x00, 0x00, 0xf4, 0x00,
            0x00, 0x00, 0xf1, 0x00, 0x00, 0x00, 0x1e, 0x00, 0x00, 0x00, 0x90, 0x00, 0x00, 0x00,
            0x5b, 0x00, 0x00, 0x00, 0xd3, 0x00, 0x00, 0x00, 0xa1, 0x00, 0x00, 0x00, 0x55, 0x00,
            0x00, 0x00, 0xd3, 0x00, 0x00, 0x00, 0x76, 0x00, 0x00, 0x00, 0xf5, 0x00, 0x00, 0x00,
            0x93, 0x00, 0x00, 0x00, 0xbd, 0x00, 0x00, 0x00, 0x73, 0x00, 0x00, 0x00, 0x04, 0x00,
            0x00, 0x00, 0x21, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x74, 0x00, 0x00, 0x00,
            0xeb, 0x00, 0x00, 0x00, 0xa0, 0x00, 0x00, 0x00, 0x8a, 0x00, 0x00, 0x00, 0x5e, 0x00,
            0x00, 0x00, 0x22, 0x00, 0x00, 0x00, 0xbc, 0x00, 0x00, 0x00, 0xcc, 0x00, 0x00, 0x00,
            0xb4, 0x00, 0x00, 0x00, 0xc9, 0x00, 0x00, 0x00, 0xd3, 0x00, 0x00, 0x00, 0x88, 0x00,
            0x00, 0x00, 0x2a, 0x00, 0x00, 0x00, 0x93, 0x00, 0x00, 0x00, 0xa5, 0x00, 0x00, 0x00,
            0x4d, 0x00, 0x00, 0x00, 0xb0, 0x00, 0x00, 0x00, 0x22, 0x00, 0x00, 0x00, 0xf5, 0x00,
            0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0xd1, 0x00, 0x00, 0x00, 0x63, 0x00, 0x00, 0x00,
            0x38, 0x00, 0x00, 0x00, 0xb6, 0x00, 0x00, 0x00, 0xb7, 0x00, 0x00, 0x00, 0xce, 0x00,
            0x00, 0x00, 0x16, 0x00, 0x00, 0x00, 0xdc, 0x00, 0x00, 0x00, 0x7f, 0x00, 0x00, 0x00,
            0x4b, 0x00, 0x00, 0x00, 0xbf, 0x00, 0x00, 0x00, 0x9a, 0x00, 0x00, 0x00, 0x96, 0x00,
            0x00, 0x00, 0xb5, 0x00, 0x00, 0x00, 0x97, 0x00, 0x00, 0x00, 0x72, 0x00, 0x00, 0x00,
            0xd6, 0x00, 0x00, 0x00, 0x60, 0x00, 0x00, 0x00, 0x6e, 0x00, 0x00, 0x00, 0x97, 0x00,
            0x00, 0x00, 0x47, 0x00, 0x00, 0x00, 0xc7, 0x00, 0x00, 0x00, 0x64, 0x00, 0x00, 0x00,
            0x9b, 0x00, 0x00, 0x00, 0xf9, 0x00, 0x00, 0x00, 0xe0, 0x00, 0x00, 0x00, 0x83, 0x00,
            0x00, 0x00, 0xdb, 0x00, 0x00, 0x00, 0x98, 0x00, 0x00, 0x00, 0x18, 0x00, 0x00, 0x00,
            0x84, 0x00, 0x00, 0x00, 0xa9, 0x00, 0x00, 0x00, 0x54, 0x00, 0x00, 0x00, 0xab, 0x00,
            0x00, 0x00, 0x3c, 0x00, 0x00, 0x00, 0x6f, 0x00, 0x00, 0x00,
        ];
        let cipher_text1 = vec![
            0x50, 0xb4, 0xc1, 0x41, 0x36, 0xbd, 0x19, 0x8c, 0x2f, 0x3c, 0x3e, 0xd2, 0x43, 0xfc,
            0xe0, 0x36, 0xe1, 0x68, 0xd5, 0x65, 0x17, 0x98, 0x4a, 0x26, 0x3c, 0xd6, 0x64, 0x92,
            0xb8, 0x08, 0x04, 0xf1, 0x69, 0xd2, 0x10, 0xf2, 0xb9, 0xbd, 0xfb, 0x48, 0xb1, 0x2f,
            0x9e, 0xa0, 0x50, 0x09, 0xc7, 0x7d, 0xa2, 0x57, 0xcc, 0x60, 0x0c, 0xce, 0xfe, 0x3a,
            0x62, 0x83, 0x78, 0x9d, 0x8e, 0xa0, 0xe6, 0x07, 0xac, 0x58, 0xe2, 0x69, 0x0e, 0xc4,
            0xeb, 0xc1, 0x01, 0x46, 0xe8, 0xcb, 0xaa, 0x5e, 0xd4, 0xd5, 0xcc, 0xe6, 0xfe, 0x7b,
            0x0f, 0xf9, 0xef, 0xc1, 0xea, 0xbb, 0x56, 0x4d, 0xbf, 0x49, 0x82, 0x85, 0xf4, 0x49,
            0xee, 0x61, 0xdd, 0x7b, 0x42, 0xee, 0x5b, 0x58, 0x92, 0xcb, 0x90, 0x60, 0x1f, 0x30,
            0xcd, 0xa0, 0x7b, 0xf2, 0x64, 0x89, 0x31, 0x0b, 0xcd, 0x23, 0xb5, 0x28, 0xce, 0xab,
            0x3c, 0x31,
        ];

        let public_key = PublicKey::new(
            BigUint::from_bytes_be(&modulus),
            BigUint::from_bytes_be(&exponent),
        );

        let rng = ReadRng::new(&*seed1);
        let pad = Pkcs1Padding::new(rng);

        let cipher_text = public_key.encrypt_block(msg1, pad);
        assert_eq!(cipher_text, cipher_text1);
    }

    #[test]
    fn rsa_oaep() {
        let modulus = vec![
            0xbb, 0xf8, 0x2f, 0x09, 0x06, 0x82, 0xce, 0x9c, 0x23, 0x38, 0xac, 0x2b, 0x9d, 0xa8,
            0x71, 0xf7, 0x36, 0x8d, 0x07, 0xee, 0xd4, 0x10, 0x43, 0xa4, 0x40, 0xd6, 0xb6, 0xf0,
            0x74, 0x54, 0xf5, 0x1f, 0xb8, 0xdf, 0xba, 0xaf, 0x03, 0x5c, 0x02, 0xab, 0x61, 0xea,
            0x48, 0xce, 0xeb, 0x6f, 0xcd, 0x48, 0x76, 0xed, 0x52, 0x0d, 0x60, 0xe1, 0xec, 0x46,
            0x19, 0x71, 0x9d, 0x8a, 0x5b, 0x8b, 0x80, 0x7f, 0xaf, 0xb8, 0xe0, 0xa3, 0xdf, 0xc7,
            0x37, 0x72, 0x3e, 0xe6, 0xb4, 0xb7, 0xd9, 0x3a, 0x25, 0x84, 0xee, 0x6a, 0x64, 0x9d,
            0x06, 0x09, 0x53, 0x74, 0x88, 0x34, 0xb2, 0x45, 0x45, 0x98, 0x39, 0x4e, 0xe0, 0xaa,
            0xb1, 0x2d, 0x7b, 0x61, 0xa5, 0x1f, 0x52, 0x7a, 0x9a, 0x41, 0xf6, 0xc1, 0x68, 0x7f,
            0xe2, 0x53, 0x72, 0x98, 0xca, 0x2a, 0x8f, 0x59, 0x46, 0xf8, 0xe5, 0xfd, 0x09, 0x1d,
            0xbd, 0xcb,
        ];
        let exponent = vec![0x11];
        let msg = vec![
            0xd4, 0x36, 0xe9, 0x95, 0x69, 0xfd, 0x32, 0xa7, 0xc8, 0xa0, 0x5b, 0xbc, 0x90, 0xd3,
            0x2c, 0x49,
        ];
        let seed: Vec<u8> = vec![
            0xaa, 0x00, 0x00, 0x00, 0xfd, 0x00, 0x00, 0x00, 0x12, 0x00, 0x00, 0x00, 0xf6, 0x00,
            0x00, 0x00, 0x59, 0x00, 0x00, 0x00, 0xca, 0x00, 0x00, 0x00, 0xe6, 0x00, 0x00, 0x00,
            0x34, 0x00, 0x00, 0x00, 0x89, 0x00, 0x00, 0x00, 0xb4, 0x00, 0x00, 0x00, 0x79, 0x00,
            0x00, 0x00, 0xe5, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0x6d, 0x00, 0x00, 0x00,
            0xde, 0x00, 0x00, 0x00, 0xc2, 0x00, 0x00, 0x00, 0xf0, 0x00, 0x00, 0x00, 0x6c, 0x00,
            0x00, 0x00, 0xb5, 0x00, 0x00, 0x00, 0x8f, 0x00, 0x00, 0x00,
        ];
        let correct_cipher_text = vec![
            0x12, 0x53, 0xe0, 0x4d, 0xc0, 0xa5, 0x39, 0x7b, 0xb4, 0x4a, 0x7a, 0xb8, 0x7e, 0x9b,
            0xf2, 0xa0, 0x39, 0xa3, 0x3d, 0x1e, 0x99, 0x6f, 0xc8, 0x2a, 0x94, 0xcc, 0xd3, 0x00,
            0x74, 0xc9, 0x5d, 0xf7, 0x63, 0x72, 0x20, 0x17, 0x06, 0x9e, 0x52, 0x68, 0xda, 0x5d,
            0x1c, 0x0b, 0x4f, 0x87, 0x2c, 0xf6, 0x53, 0xc1, 0x1d, 0xf8, 0x23, 0x14, 0xa6, 0x79,
            0x68, 0xdf, 0xea, 0xe2, 0x8d, 0xef, 0x04, 0xbb, 0x6d, 0x84, 0xb1, 0xc3, 0x1d, 0x65,
            0x4a, 0x19, 0x70, 0xe5, 0x78, 0x3b, 0xd6, 0xeb, 0x96, 0xa0, 0x24, 0xc2, 0xca, 0x2f,
            0x4a, 0x90, 0xfe, 0x9f, 0x2e, 0xf5, 0xc9, 0xc1, 0x40, 0xe5, 0xbb, 0x48, 0xda, 0x95,
            0x36, 0xad, 0x87, 0x00, 0xc8, 0x4f, 0xc9, 0x13, 0x0a, 0xde, 0xa7, 0x4e, 0x55, 0x8d,
            0x51, 0xa7, 0x4d, 0xdf, 0x85, 0xd8, 0xb5, 0x0d, 0xe9, 0x68, 0x38, 0xd6, 0x06, 0x3e,
            0x09, 0x55,
        ];

        let public_key = PublicKey::new(
            BigUint::from_bytes_be(&modulus),
            BigUint::from_bytes_be(&exponent),
        );

        let rng = ReadRng::new(&*seed);
        let pad = Pkcs1OaepPadding::new(rng);

        let cipher_text = public_key.encrypt_block(msg, pad);
        assert_eq!(cipher_text, correct_cipher_text);
    }
}
