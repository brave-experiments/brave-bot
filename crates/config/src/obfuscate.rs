// Reversible byte masking, shared by the build script and the runtime.
//
// This is obfuscation, not encryption. The keystream is derived from constants in
// this file, which ships in the same binary, so anyone willing to read the source
// recovers the bytes. What it buys is that `strings` on the binary, or a grep of a
// core dump, does not hand a credential over for free.
//
// The build script includes this file directly, so it must stay dependency-free.

/// Arbitrary constant. Changing it changes every masked byte.
const MASK_SEED: u64 = 0x5d1a_c3e7_08b4_9f26;

/// XOR the bytes with a splitmix64 keystream.
///
/// Symmetric: masking a masked value returns the original, since the keystream
/// depends only on the length, which masking preserves.
pub(crate) fn mask(bytes: &[u8]) -> Vec<u8> {
    let mut stream = Keystream::for_length(bytes.len());
    bytes.iter().map(|b| b ^ stream.byte()).collect()
}

struct Keystream(u64);

impl Keystream {
    /// Folding the length in means two values that share a prefix do not share a
    /// masked prefix unless they are also the same length.
    fn for_length(len: usize) -> Self {
        Self(MASK_SEED ^ (len as u64).wrapping_mul(0xff51_afd7_ed55_8ccd))
    }

    fn byte(&mut self) -> u8 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        (z ^ (z >> 31)) as u8
    }
}
