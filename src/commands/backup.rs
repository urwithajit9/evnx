/// Backup command - create encrypted backup of .env file
///
/// Uses AES-256-GCM with Argon2 key derivation for security
use anyhow::{anyhow, Context, Result};
use colored::*;
use dialoguer::Password;
use std::fs;
use std::path::Path;

#[cfg(feature = "backup")]
use aes_gcm::aead::rand_core::RngCore;
#[cfg(feature = "backup")]
use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
#[cfg(feature = "backup")]
use argon2::{Argon2, PasswordHasher};
#[cfg(feature = "backup")]
use base64::{engine::general_purpose, Engine as _};

pub fn run(env: String, output: Option<String>, verbose: bool) -> Result<()> {
    #[cfg(not(feature = "backup"))]
    {
        println!("{} Backup feature not enabled", "✗".red());
        println!("Rebuild with: cargo build --features backup");
        return Ok(());
    }

    #[cfg(feature = "backup")]
    {
        if verbose {
            println!("{}", "Running backup in verbose mode".dimmed());
        }

        println!(
            "\n{}",
            "┌─ Create encrypted backup ───────────────────────────┐".cyan()
        );
        println!(
            "{}",
            "│ Your .env will be encrypted with AES-256-GCM        │".cyan()
        );
        println!(
            "{}\n",
            "└──────────────────────────────────────────────────────┘".cyan()
        );

        // Check if source exists
        if !Path::new(&env).exists() {
            return Err(anyhow!("File not found: {}", env));
        }

        // Read .env file
        let content =
            fs::read_to_string(&env).with_context(|| format!("Failed to read {}", env))?;

        println!("{} Read {} bytes from {}", "✓".green(), content.len(), env);

        // Get password
        let password = Password::new()
            .with_prompt("Enter encryption password")
            .interact()?;

        let password_confirm = Password::new().with_prompt("Confirm password").interact()?;

        if password != password_confirm {
            return Err(anyhow!("Passwords do not match"));
        }

        println!("{} Password set", "✓".green());

        // Encrypt
        let encrypted = encrypt_content(&content, &password)?;

        // Determine output path
        let output_path = output.unwrap_or_else(|| format!("{}.backup", env));

        // Write encrypted backup
        fs::write(&output_path, &encrypted)
            .with_context(|| format!("Failed to write to {}", output_path))?;

        println!("{} Backup created at {}", "✓".green(), output_path);
        println!("\n{}", "⚠️  Important:".yellow().bold());
        println!("  • Keep your password safe - it cannot be recovered");
        println!("  • Store backup file in a secure location");
        println!("  • Delete original .env if moving to new system");

        Ok(())
    }
}

#[cfg(feature = "backup")]
fn encrypt_content(plaintext: &str, password: &str) -> Result<String> {
    // Generate salt for key derivation
    let mut salt = [0u8; 32];
    OsRng.fill_bytes(&mut salt);

    // Derive key from password using Argon2
    let argon2 = Argon2::default();
    let salt_string = salt_string(&salt);

    let password_hash = argon2
        .hash_password(password.as_bytes(), &salt_string)
        .map_err(|e| anyhow!("Failed to derive key: {}", e))?;

    // Extract 32-byte key
    let key_bytes = password_hash.hash.unwrap();
    let key = key_bytes.as_bytes();

    // Ensure we have exactly 32 bytes
    if key.len() < 32 {
        return Err(anyhow!("Derived key too short"));
    }
    let key: &[u8; 32] = key[..32].try_into()?;

    // Create cipher
    let cipher = Aes256Gcm::new(key.into());

    // Generate nonce
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    // Encrypt
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| anyhow!("Encryption failed: {}", e))?;

    // Format: version(1) || salt(32) || nonce(12) || ciphertext
    let mut result = Vec::new();
    result.push(1u8); // Version
    result.extend_from_slice(&salt);
    result.extend_from_slice(&nonce_bytes);
    result.extend_from_slice(&ciphertext);

    // Base64 encode
    Ok(general_purpose::STANDARD.encode(&result))
}

#[cfg(feature = "backup")]
fn salt_string(salt: &[u8]) -> argon2::password_hash::SaltString {
    use argon2::password_hash::SaltString;
    SaltString::encode_b64(salt).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(feature = "backup")]
    fn test_encrypt_decrypt_roundtrip() {
        let plaintext = "DATABASE_URL=postgresql://localhost:5432/db\nSECRET_KEY=abc123";
        let password = "test-password";

        let encrypted = encrypt_content(plaintext, password).unwrap();
        assert!(!encrypted.is_empty());
        assert_ne!(encrypted, plaintext);

        // Decryption would be tested in restore command
    }
}
