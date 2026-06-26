    use super::*;

    #[test]
    fn encrypts_and_decrypts_secret() {
        let key = "test-secret-encryption-key";
        let encrypted = encrypt_secret("sk-bf-test", key).expect("encrypt");
        assert_ne!(encrypted, "sk-bf-test");
        assert_eq!(
            decrypt_secret(&encrypted, key).expect("decrypt"),
            "sk-bf-test"
        );
    }
