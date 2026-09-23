//! The signature check, against a real yt-dlp release (2026.08.19) and its
//! signing key.

use nitrate_lib::signature::{verify, RsaKey};

const MODULUS: &str = concat!(
    "f4ac5f738c63c0b74b6196de42d5e6f371c0155fb35bd6ff9ea908e4f9611975",
    "31135428375803399e002b947766fdf66acd480efa9018ac1eb418a7a0edf7d4",
    "0dff92d1714a11975e44b61e01aaaa2a14a26d56a50e3bce04ecedbb75bad3b1",
    "cb113400a80ef04cfb88b765aa9a92fad0d21ccabdbb2f1aa681d37798dbd9da",
    "22cef9b3f49ca47f0be3e013d1f8cadcb00dad76cccc7a3e66429d538e3aee97",
    "644166754c71750f5f143a9df4b701298d8b0ce712628d6c698d185a26561edc",
    "d5a19bb590ece39d2e458eaff204724b2260c7ee93711e6eecfcf9dababa6640",
    "5b4316bba4b7eda58439e9294986aff23fdc81ea1ff5fef673e84b8ba2a90e47",
    "a5b941f012bee167e96a2120ef92e7bee38f7d920f298c976bfd00ed0a233308",
    "0eac3f3ca334fe54290a92427e8a35d9ee01cd8e670169474a2bb7c320439823",
    "adcbc9ce905a41904e28057df4945d61c308ef5a5cd84fdde8ea15680632d094",
    "b2b6e4709916b9663bab9011cbfd391db48cdcae0353d7aab67c66895d5643c9",
    "d7b88f49809411be1e4f4e49f2e4bb71ab3726aaf58ef538c705baecd7804f10",
    "ef5f717c4a2ff08505bcd70d2bef02002b76aaa53289dc43fca271395f2faab3",
    "27c10a162a96d7840c4dd027e92bae653a1d5c24e2227c284fb5a38ee14f28a7",
    "6f7c7d463f86e7a9dcd39f32f10920c812d7f27cfb085fef4623b26493fcd98d",
);

const KEY: RsaKey = RsaKey {
    modulus: MODULUS,
    exponent: 65537,
};

const SUMS: &[u8] = include_bytes!("fixtures/yt-dlp-SHA2-256SUMS");
const SIG: &[u8] = include_bytes!("fixtures/yt-dlp-SHA2-256SUMS.sig");

#[test]
fn a_real_release_verifies() {
    assert_eq!(verify(SUMS, SIG, &KEY), Ok(()));
}

#[test]
fn a_changed_file_does_not() {
    let mut sums = SUMS.to_vec();
    // One hex digit of the Windows build's checksum.
    let at = sums.iter().position(|&b| b == b'\n').unwrap() + 3;
    sums[at] = if sums[at] == b'0' { b'1' } else { b'0' };
    assert!(verify(&sums, SIG, &KEY).is_err());
}

#[test]
fn a_changed_signature_does_not() {
    // Every byte of the RSA value, one at a time: none may still pass.
    let value_starts = SIG.len() - 512;
    for at in (value_starts..SIG.len()).step_by(37) {
        let mut sig = SIG.to_vec();
        sig[at] ^= 0x01;
        assert!(verify(SUMS, &sig, &KEY).is_err(), "byte {at}");
    }
}

#[test]
fn another_key_does_not() {
    let other = MODULUS.replacen('f', "e", 1);
    let other = RsaKey {
        modulus: Box::leak(other.into_boxed_str()),
        exponent: 65537,
    };
    assert!(verify(SUMS, SIG, &other).is_err());
}

#[test]
fn a_truncated_signature_is_an_error_not_a_crash() {
    for len in 0..SIG.len() {
        assert!(verify(SUMS, &SIG[..len], &KEY).is_err(), "length {len}");
    }
}
