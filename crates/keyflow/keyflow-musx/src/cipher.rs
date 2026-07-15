//! The MUSX PRNG-based stream cipher used to obfuscate `score.dat`.
//!
//! Port of `decrypt()` from `musx2mxl/musx2mxl.py`. The cipher is symmetric
//! (XOR keystream), so the same routine encrypts and decrypts in place.

const CIPHER_INITIAL_STATE: u32 = 0x2800_6D45;
const CIPHER_MULTIPLIER: u32 = 0x41C6_4E6D;
const CIPHER_INCREMENT: u32 = 0x3039;
const CIPHER_RESET_INTERVAL: usize = 0x2_0000;

/// Decrypt (or encrypt) `buffer` in place with the MUSX stream cipher.
pub fn decrypt(buffer: &mut [u8]) {
    let mut state = CIPHER_INITIAL_STATE;

    for (i, byte) in buffer.iter_mut().enumerate() {
        if i % CIPHER_RESET_INTERVAL == 0 {
            state = CIPHER_INITIAL_STATE;
        }

        state = state
            .wrapping_mul(CIPHER_MULTIPLIER)
            .wrapping_add(CIPHER_INCREMENT);
        let upper = state >> 16;
        let pseudo_random_byte = (upper + upper / 255) & 0xFF;
        *byte ^= pseudo_random_byte as u8;
    }
}
