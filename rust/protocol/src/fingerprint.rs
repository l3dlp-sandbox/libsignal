//
// Copyright 2020 Signal Messenger, LLC.
// SPDX-License-Identifier: AGPL-3.0-only
//

use std::fmt;
use std::fmt::Write;

use prost::Message;
use sha2::digest::Digest;
use sha2::Sha512;
use subtle::ConstantTimeEq;

use crate::{proto, IdentityKey, Result, SignalProtocolError};

#[derive(Debug, Clone)]
pub struct DisplayableFingerprint {
    local: String,
    remote: String,
}

impl fmt::Display for DisplayableFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.local < self.remote {
            write!(f, "{}{}", self.local, self.remote)
        } else {
            write!(f, "{}{}", self.remote, self.local)
        }
    }
}

fn get_encoded_string(fprint: &[u8]) -> Result<String> {
    if fprint.len() < 30 {
        return Err(SignalProtocolError::InvalidArgument(
            "DisplayableFingerprint created with short encoding".to_string(),
        ));
    }

    fn read5_mod_100k(fprint: &[u8]) -> u64 {
        assert_eq!(fprint.len(), 5);
        let x = fprint.iter().fold(0u64, |acc, &x| (acc << 8) | (x as u64));
        x % 100_000
    }

    let s = fprint.chunks_exact(5).take(6).map(read5_mod_100k).fold(
        String::with_capacity(5 * 6),
        |mut s, n| {
            write!(s, "{n:05}").expect("can always write to a String");
            s
        },
    );

    Ok(s)
}

impl DisplayableFingerprint {
    pub fn new(local: &[u8], remote: &[u8]) -> Result<Self> {
        Ok(Self {
            local: get_encoded_string(local)?,
            remote: get_encoded_string(remote)?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ScannableFingerprint {
    version: u32,
    local_fingerprint: Vec<u8>,
    remote_fingerprint: Vec<u8>,
}

impl ScannableFingerprint {
    fn new(version: u32, local_fprint: &[u8], remote_fprint: &[u8]) -> Self {
        Self {
            version,
            local_fingerprint: local_fprint[..32].to_vec(),
            remote_fingerprint: remote_fprint[..32].to_vec(),
        }
    }

    pub fn deserialize(protobuf: &[u8]) -> Result<Self> {
        let fingerprint = proto::fingerprint::CombinedFingerprints::decode(protobuf)
            .map_err(|_| SignalProtocolError::FingerprintParsingError)?;

        Ok(Self {
            version: fingerprint
                .version
                .ok_or(SignalProtocolError::FingerprintParsingError)?,
            local_fingerprint: fingerprint
                .local_fingerprint
                .ok_or(SignalProtocolError::FingerprintParsingError)?
                .content
                .ok_or(SignalProtocolError::FingerprintParsingError)?,
            remote_fingerprint: fingerprint
                .remote_fingerprint
                .ok_or(SignalProtocolError::FingerprintParsingError)?
                .content
                .ok_or(SignalProtocolError::FingerprintParsingError)?,
        })
    }

    pub fn serialize(&self) -> Result<Vec<u8>> {
        let combined_fingerprints = proto::fingerprint::CombinedFingerprints {
            version: Some(self.version),
            local_fingerprint: Some(proto::fingerprint::LogicalFingerprint {
                content: Some(self.local_fingerprint.to_owned()),
            }),
            remote_fingerprint: Some(proto::fingerprint::LogicalFingerprint {
                content: Some(self.remote_fingerprint.to_owned()),
            }),
        };

        Ok(combined_fingerprints.encode_to_vec())
    }

    pub fn compare(&self, combined: &[u8]) -> Result<bool> {
        let combined = proto::fingerprint::CombinedFingerprints::decode(combined)
            .map_err(|_| SignalProtocolError::FingerprintParsingError)?;

        let their_version = combined.version.unwrap_or(0);

        if their_version != self.version {
            return Err(SignalProtocolError::FingerprintVersionMismatch(
                their_version,
                self.version,
            ));
        }

        let same1 = combined
            .local_fingerprint
            .as_ref()
            .ok_or(SignalProtocolError::FingerprintParsingError)?
            .content
            .as_ref()
            .ok_or(SignalProtocolError::FingerprintParsingError)?
            .ct_eq(&self.remote_fingerprint);
        let same2 = combined
            .remote_fingerprint
            .as_ref()
            .ok_or(SignalProtocolError::FingerprintParsingError)?
            .content
            .as_ref()
            .ok_or(SignalProtocolError::FingerprintParsingError)?
            .ct_eq(&self.local_fingerprint);

        Ok(same1.into() && same2.into())
    }
}

#[derive(Debug, Clone)]
pub struct Fingerprint {
    pub display: DisplayableFingerprint,
    pub scannable: ScannableFingerprint,
}

impl Fingerprint {
    fn get_fingerprint(
        iterations: u32,
        local_id: &[u8],
        local_key: &IdentityKey,
    ) -> Result<Vec<u8>> {
        if iterations <= 1 || iterations > 1000000 {
            return Err(SignalProtocolError::InvalidArgument(format!(
                "Invalid fingerprint iterations {iterations}"
            )));
        }

        let fingerprint_version = [0u8, 0u8]; // 0x0000
        let key_bytes = local_key.serialize();

        let mut sha512 = Sha512::new();

        // iteration=0
        // Explicitly pass a slice to avoid generating multiple versions of update().
        sha512.update(&fingerprint_version[..]);
        sha512.update(&key_bytes);
        sha512.update(local_id);
        sha512.update(&key_bytes);
        let mut buf = sha512.finalize();

        for _i in 1..iterations {
            let mut sha512 = Sha512::new();
            // Explicitly pass a slice to avoid generating multiple versions of update().
            sha512.update(&buf[..]);
            sha512.update(&key_bytes);
            buf = sha512.finalize();
        }

        Ok(buf.to_vec())
    }

    pub fn new(
        version: u32,
        iterations: u32,
        local_id: &[u8],
        local_key: &IdentityKey,
        remote_id: &[u8],
        remote_key: &IdentityKey,
    ) -> Result<Fingerprint> {
        let local_fingerprint = Fingerprint::get_fingerprint(iterations, local_id, local_key)?;
        let remote_fingerprint = Fingerprint::get_fingerprint(iterations, remote_id, remote_key)?;

        Ok(Fingerprint {
            display: DisplayableFingerprint::new(&local_fingerprint, &remote_fingerprint)?,
            scannable: ScannableFingerprint::new(version, &local_fingerprint, &remote_fingerprint),
        })
    }

    pub fn display_string(&self) -> Result<String> {
        Ok(format!("{}", self.display))
    }
}

#[cfg(test)]
mod test {
    use const_str::hex;
    use rand::TryRngCore as _;

    use super::*;

    const ALICE_IDENTITY: &[u8] =
        &hex!("0506863bc66d02b40d27b8d49ca7c09e9239236f9d7d25d6fcca5ce13c7064d868");
    const BOB_IDENTITY: &[u8] =
        &hex!("05f781b6fb32fed9ba1cf2de978d4d5da28dc34046ae814402b5c0dbd96fda907b");

    const DISPLAYABLE_FINGERPRINT_V1: &str =
        "300354477692869396892869876765458257569162576843440918079131";
    const ALICE_SCANNABLE_FINGERPRINT_V1 : &str = "080112220a201e301a0353dce3dbe7684cb8336e85136cdc0ee96219494ada305d62a7bd61df1a220a20d62cbf73a11592015b6b9f1682ac306fea3aaf3885b84d12bca631e9d4fb3a4d";
    const BOB_SCANNABLE_FINGERPRINT_V1   : &str = "080112220a20d62cbf73a11592015b6b9f1682ac306fea3aaf3885b84d12bca631e9d4fb3a4d1a220a201e301a0353dce3dbe7684cb8336e85136cdc0ee96219494ada305d62a7bd61df";

    const ALICE_SCANNABLE_FINGERPRINT_V2 : &str = "080212220a201e301a0353dce3dbe7684cb8336e85136cdc0ee96219494ada305d62a7bd61df1a220a20d62cbf73a11592015b6b9f1682ac306fea3aaf3885b84d12bca631e9d4fb3a4d";
    const BOB_SCANNABLE_FINGERPRINT_V2   : & str = "080212220a20d62cbf73a11592015b6b9f1682ac306fea3aaf3885b84d12bca631e9d4fb3a4d1a220a201e301a0353dce3dbe7684cb8336e85136cdc0ee96219494ada305d62a7bd61df";

    const ALICE_STABLE_ID: &str = "+14152222222";
    const BOB_STABLE_ID: &str = "+14153333333";

    #[test]
    fn fingerprint_encodings() -> Result<()> {
        let l = vec![0x12; 32];
        let r = vec![0xBA; 32];

        let fprint2 = ScannableFingerprint::new(2, &l, &r);
        let proto2 = fprint2.serialize()?;

        let expected2_encoding =
            "080212220a20".to_owned() + &"12".repeat(32) + "1a220a20" + &"ba".repeat(32);
        assert_eq!(hex::encode(proto2), expected2_encoding);

        Ok(())
    }

    #[test]
    fn fingerprint_test_v1() -> Result<()> {
        // testVectorsVersion1 in Java

        let a_key = IdentityKey::decode(ALICE_IDENTITY)?;
        let b_key = IdentityKey::decode(BOB_IDENTITY)?;

        let version = 1;
        let iterations = 5200;

        let a_fprint = Fingerprint::new(
            version,
            iterations,
            ALICE_STABLE_ID.as_bytes(),
            &a_key,
            BOB_STABLE_ID.as_bytes(),
            &b_key,
        )?;

        let b_fprint = Fingerprint::new(
            version,
            iterations,
            BOB_STABLE_ID.as_bytes(),
            &b_key,
            ALICE_STABLE_ID.as_bytes(),
            &a_key,
        )?;

        assert_eq!(
            hex::encode(a_fprint.scannable.serialize()?),
            ALICE_SCANNABLE_FINGERPRINT_V1
        );
        assert_eq!(
            hex::encode(b_fprint.scannable.serialize()?),
            BOB_SCANNABLE_FINGERPRINT_V1
        );

        assert_eq!(format!("{}", a_fprint.display), DISPLAYABLE_FINGERPRINT_V1);
        assert_eq!(format!("{}", b_fprint.display), DISPLAYABLE_FINGERPRINT_V1);

        assert_eq!(
            hex::encode(a_fprint.scannable.serialize()?),
            ALICE_SCANNABLE_FINGERPRINT_V1
        );
        assert_eq!(
            hex::encode(b_fprint.scannable.serialize()?),
            BOB_SCANNABLE_FINGERPRINT_V1
        );

        Ok(())
    }

    #[test]
    fn fingerprint_test_v2() -> Result<()> {
        // testVectorsVersion2 in Java

        let a_key = IdentityKey::decode(ALICE_IDENTITY)?;
        let b_key = IdentityKey::decode(BOB_IDENTITY)?;

        let version = 2;
        let iterations = 5200;

        let a_fprint = Fingerprint::new(
            version,
            iterations,
            ALICE_STABLE_ID.as_bytes(),
            &a_key,
            BOB_STABLE_ID.as_bytes(),
            &b_key,
        )?;

        let b_fprint = Fingerprint::new(
            version,
            iterations,
            BOB_STABLE_ID.as_bytes(),
            &b_key,
            ALICE_STABLE_ID.as_bytes(),
            &a_key,
        )?;

        assert_eq!(
            hex::encode(a_fprint.scannable.serialize()?),
            ALICE_SCANNABLE_FINGERPRINT_V2
        );
        assert_eq!(
            hex::encode(b_fprint.scannable.serialize()?),
            BOB_SCANNABLE_FINGERPRINT_V2
        );

        // unchanged vs v1
        assert_eq!(format!("{}", a_fprint.display), DISPLAYABLE_FINGERPRINT_V1);
        assert_eq!(format!("{}", b_fprint.display), DISPLAYABLE_FINGERPRINT_V1);

        assert_eq!(
            hex::encode(a_fprint.scannable.serialize()?),
            ALICE_SCANNABLE_FINGERPRINT_V2
        );
        assert_eq!(
            hex::encode(b_fprint.scannable.serialize()?),
            BOB_SCANNABLE_FINGERPRINT_V2
        );

        Ok(())
    }

    #[test]
    fn fingerprint_matching_identifiers() -> Result<()> {
        // testMatchingFingerprints

        use rand::rngs::OsRng;

        use crate::IdentityKeyPair;

        let a_key_pair = IdentityKeyPair::generate(&mut OsRng.unwrap_err());
        let b_key_pair = IdentityKeyPair::generate(&mut OsRng.unwrap_err());

        let a_key = a_key_pair.identity_key();
        let b_key = b_key_pair.identity_key();

        let version = 1;
        let iterations = 1024;

        let a_fprint = Fingerprint::new(
            version,
            iterations,
            ALICE_STABLE_ID.as_bytes(),
            a_key,
            BOB_STABLE_ID.as_bytes(),
            b_key,
        )?;

        let b_fprint = Fingerprint::new(
            version,
            iterations,
            BOB_STABLE_ID.as_bytes(),
            b_key,
            ALICE_STABLE_ID.as_bytes(),
            a_key,
        )?;

        assert_eq!(
            format!("{}", a_fprint.display),
            format!("{}", b_fprint.display)
        );
        assert_eq!(format!("{}", a_fprint.display).len(), 60);

        assert!(a_fprint
            .scannable
            .compare(&b_fprint.scannable.serialize()?)?);
        assert!(b_fprint
            .scannable
            .compare(&a_fprint.scannable.serialize()?)?);

        // Java is missing this test
        assert!(!a_fprint
            .scannable
            .compare(&a_fprint.scannable.serialize()?)?);
        assert!(!b_fprint
            .scannable
            .compare(&b_fprint.scannable.serialize()?)?);

        Ok(())
    }

    #[test]
    fn fingerprint_mismatching_fingerprints() -> Result<()> {
        use rand::rngs::OsRng;

        use crate::IdentityKeyPair;

        let mut rng = OsRng.unwrap_err();
        let a_key_pair = IdentityKeyPair::generate(&mut rng);
        let b_key_pair = IdentityKeyPair::generate(&mut rng);
        let m_key_pair = IdentityKeyPair::generate(&mut rng); // mitm

        let a_key = a_key_pair.identity_key();
        let b_key = b_key_pair.identity_key();
        let m_key = m_key_pair.identity_key();

        let version = 1;
        let iterations = 1024;

        let a_fprint = Fingerprint::new(
            version,
            iterations,
            ALICE_STABLE_ID.as_bytes(),
            a_key,
            BOB_STABLE_ID.as_bytes(),
            m_key,
        )?;

        let b_fprint = Fingerprint::new(
            version,
            iterations,
            BOB_STABLE_ID.as_bytes(),
            b_key,
            ALICE_STABLE_ID.as_bytes(),
            a_key,
        )?;

        assert_ne!(
            format!("{}", a_fprint.display),
            format!("{}", b_fprint.display)
        );

        assert!(!a_fprint
            .scannable
            .compare(&b_fprint.scannable.serialize()?)?);
        assert!(!b_fprint
            .scannable
            .compare(&a_fprint.scannable.serialize()?)?);

        Ok(())
    }

    #[test]
    fn fingerprint_mismatching_identifiers() -> Result<()> {
        use rand::rngs::OsRng;

        use crate::IdentityKeyPair;

        let mut rng = OsRng.unwrap_err();
        let a_key_pair = IdentityKeyPair::generate(&mut rng);
        let b_key_pair = IdentityKeyPair::generate(&mut rng);

        let a_key = a_key_pair.identity_key();
        let b_key = b_key_pair.identity_key();

        let version = 1;
        let iterations = 1024;

        let a_fprint = Fingerprint::new(
            version,
            iterations,
            "+141512222222".as_bytes(),
            a_key,
            BOB_STABLE_ID.as_bytes(),
            b_key,
        )?;

        let b_fprint = Fingerprint::new(
            version,
            iterations,
            BOB_STABLE_ID.as_bytes(),
            b_key,
            ALICE_STABLE_ID.as_bytes(),
            a_key,
        )?;

        assert_ne!(
            format!("{}", a_fprint.display),
            format!("{}", b_fprint.display)
        );

        assert!(!a_fprint
            .scannable
            .compare(&b_fprint.scannable.serialize()?)?);
        assert!(!b_fprint
            .scannable
            .compare(&a_fprint.scannable.serialize()?)?);

        Ok(())
    }

    #[test]
    fn fingerprint_mismatching_versions() -> Result<()> {
        let a_key = IdentityKey::decode(ALICE_IDENTITY)?;
        let b_key = IdentityKey::decode(BOB_IDENTITY)?;

        let iterations = 5200;

        let a_fprint_v1 = Fingerprint::new(
            1,
            iterations,
            ALICE_STABLE_ID.as_bytes(),
            &a_key,
            BOB_STABLE_ID.as_bytes(),
            &b_key,
        )?;

        let a_fprint_v2 = Fingerprint::new(
            2,
            iterations,
            BOB_STABLE_ID.as_bytes(),
            &b_key,
            ALICE_STABLE_ID.as_bytes(),
            &a_key,
        )?;

        // Display fingerprint doesn't change
        assert_eq!(
            format!("{}", a_fprint_v1.display),
            format!("{}", a_fprint_v2.display)
        );

        // Scannable fingerprint does
        assert_ne!(
            hex::encode(a_fprint_v1.scannable.serialize()?),
            hex::encode(a_fprint_v2.scannable.serialize()?)
        );

        Ok(())
    }
}
