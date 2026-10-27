//! Android-only token storage. The AES key never leaves Android Keystore;
//! only an authenticated ciphertext is written to the no-backup directory.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use jni::objects::{JByteArray, JObject, JString, JValue};
use jni::{JNIEnv, JavaVM};

use super::Tokens;

const ALIAS: &str = "dev.timetracker.google-tokens.v1";
const HEADER: &[u8] = b"TTK1";
const AAD: &[u8] = b"timetracker-google-tokens-v1";
const IV_LEN: usize = 12;
const TAG_LEN: usize = 16;
const MAX_FILE_SIZE: u64 = 64 * 1024;
static STORAGE: Mutex<()> = Mutex::new(());

// Local frames matter here: the sync worker remains attached to the VM and
// loads tokens repeatedly. Never retain JNI local references between calls.
fn with_jni<T>(f: impl FnOnce(&mut JNIEnv<'_>) -> jni::errors::Result<T>) -> io::Result<T> {
    // Dioxus initializes ndk-context before invoking the application's main.
    let context = ndk_context::android_context();
    let vm = unsafe { JavaVM::from_raw(context.vm().cast()) }.map_err(jni_error)?;
    let mut env = vm.attach_current_thread().map_err(jni_error)?;
    let result = env.with_local_frame(64, f);
    if result.is_err() {
        // A Java exception must not poison subsequent calls on this thread.
        let _ = env.exception_clear();
    }
    result.map_err(jni_error)
}

fn jni_error(error: jni::errors::Error) -> io::Error {
    io::Error::other(format!("Android token storage: {error}"))
}

fn path() -> io::Result<PathBuf> {
    with_jni(|env| {
        let context = ndk_context::android_context();
        // Borrow the runtime-owned context; do not delete its global reference.
        let activity = unsafe { JObject::from_raw(context.context().cast()) };
        let directory = env
            .call_method(&activity, "getNoBackupFilesDir", "()Ljava/io/File;", &[])?
            .l()?;
        let name = env
            .call_method(directory, "getAbsolutePath", "()Ljava/lang/String;", &[])?
            .l()?;
        let name: String = env.get_string(&JString::from(name))?.into();
        Ok(PathBuf::from(name).join("google-tokens.v1.enc"))
    })
}

fn key_store<'a>(env: &mut JNIEnv<'a>) -> jni::errors::Result<JObject<'a>> {
    let provider = env.new_string("AndroidKeyStore")?;
    let store = env
        .call_static_method(
            "java/security/KeyStore",
            "getInstance",
            "(Ljava/lang/String;)Ljava/security/KeyStore;",
            &[JValue::Object(&provider)],
        )?
        .l()?;
    env.call_method(
        &store,
        "load",
        "(Ljava/security/KeyStore$LoadStoreParameter;)V",
        &[JValue::Object(&JObject::null())],
    )?;
    Ok(store)
}

fn key<'a>(env: &mut JNIEnv<'a>, create: bool) -> jni::errors::Result<JObject<'a>> {
    let store = key_store(env)?;
    let alias = env.new_string(ALIAS)?;
    let existing = env
        .call_method(
            &store,
            "getKey",
            "(Ljava/lang/String;[C)Ljava/security/Key;",
            &[JValue::Object(&alias), JValue::Object(&JObject::null())],
        )?
        .l()?;
    if !existing.is_null() {
        return Ok(existing);
    }
    if !create {
        return Err(jni::errors::Error::NullPtr(
            "Google token encryption key is unavailable; reconnect Google",
        ));
    }

    let aes = env.new_string("AES")?;
    let provider = env.new_string("AndroidKeyStore")?;
    let generator = env
        .call_static_method(
            "javax/crypto/KeyGenerator",
            "getInstance",
            "(Ljava/lang/String;Ljava/lang/String;)Ljavax/crypto/KeyGenerator;",
            &[JValue::Object(&aes), JValue::Object(&provider)],
        )?
        .l()?;
    // PURPOSE_ENCRYPT | PURPOSE_DECRYPT = 1 | 2.
    let builder = env.new_object(
        "android/security/keystore/KeyGenParameterSpec$Builder",
        "(Ljava/lang/String;I)V",
        &[JValue::Object(&alias), JValue::Int(3)],
    )?;
    env.call_method(
        &builder,
        "setKeySize",
        "(I)Landroid/security/keystore/KeyGenParameterSpec$Builder;",
        &[JValue::Int(256)],
    )?;
    for (method, value) in [
        ("setBlockModes", "GCM"),
        ("setEncryptionPaddings", "NoPadding"),
    ] {
        let value = env.new_string(value)?;
        let values = env.new_object_array(1, "java/lang/String", &value)?;
        env.call_method(
            &builder,
            method,
            "([Ljava/lang/String;)Landroid/security/keystore/KeyGenParameterSpec$Builder;",
            &[JValue::Object(&values)],
        )?;
    }
    let spec = env
        .call_method(
            builder,
            "build",
            "()Landroid/security/keystore/KeyGenParameterSpec;",
            &[],
        )?
        .l()?;
    env.call_method(
        &generator,
        "init",
        "(Ljava/security/spec/AlgorithmParameterSpec;)V",
        &[JValue::Object(&spec)],
    )?;
    env.call_method(generator, "generateKey", "()Ljavax/crypto/SecretKey;", &[])?
        .l()
}

fn crypt(input: &[u8], iv: Option<&[u8]>) -> io::Result<(Vec<u8>, Vec<u8>)> {
    with_jni(|env| {
        let key = key(env, iv.is_none())?;
        let transformation = env.new_string("AES/GCM/NoPadding")?;
        let cipher = env
            .call_static_method(
                "javax/crypto/Cipher",
                "getInstance",
                "(Ljava/lang/String;)Ljavax/crypto/Cipher;",
                &[JValue::Object(&transformation)],
            )?
            .l()?;
        if let Some(iv) = iv {
            let iv = env.byte_array_from_slice(iv)?;
            let spec = env.new_object(
                "javax/crypto/spec/GCMParameterSpec",
                "(I[B)V",
                &[JValue::Int(128), JValue::Object(&iv)],
            )?;
            env.call_method(
                &cipher,
                "init",
                "(ILjava/security/Key;Ljava/security/spec/AlgorithmParameterSpec;)V",
                &[JValue::Int(2), JValue::Object(&key), JValue::Object(&spec)],
            )?;
        } else {
            // Let the provider generate a fresh random IV for every encryption.
            env.call_method(
                &cipher,
                "init",
                "(ILjava/security/Key;)V",
                &[JValue::Int(1), JValue::Object(&key)],
            )?;
        }
        let aad = env.byte_array_from_slice(AAD)?;
        env.call_method(&cipher, "updateAAD", "([B)V", &[JValue::Object(&aad)])?;
        let input = env.byte_array_from_slice(input)?;
        let output = env
            .call_method(&cipher, "doFinal", "([B)[B", &[JValue::Object(&input)])?
            .l()?;
        let iv = env.call_method(&cipher, "getIV", "()[B", &[])?.l()?;
        Ok((
            env.convert_byte_array(JByteArray::from(output))?,
            env.convert_byte_array(JByteArray::from(iv))?,
        ))
    })
}

fn read_bounded(path: &Path) -> io::Result<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_FILE_SIZE + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FILE_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Google token file is too large",
        ));
    }
    Ok(bytes)
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

fn remove_legacy() -> io::Result<()> {
    if let Some(path) = super::token_path() {
        remove_if_present(&path)?;
        remove_if_present(&path.with_extension("toml.tmp"))?;
    }
    Ok(())
}

fn save_at(path: &Path, tokens: &Tokens) -> io::Result<()> {
    let text = toml::to_string(tokens).map_err(io::Error::other)?;
    let (ciphertext, iv) = crypt(text.as_bytes(), None)?;
    if iv.len() != IV_LEN {
        return Err(io::Error::other(
            "Android returned an unexpected GCM IV length",
        ));
    }
    let mut bytes = HEADER.to_vec();
    bytes.extend_from_slice(&iv);
    bytes.extend_from_slice(&ciphertext);
    let temporary = path.with_extension("enc.tmp");
    let result = (|| {
        use std::os::unix::fs::OpenOptionsExt;
        fs::create_dir_all(path.parent().expect("token directory"))?;
        remove_if_present(&temporary)?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        // Do not delete the old token until its encrypted replacement is durable.
        fs::File::open(path.parent().expect("token directory"))?.sync_all()?;
        remove_legacy()
    })();
    if result.is_err() {
        let _ = remove_if_present(&temporary);
    }
    result
}

pub(super) fn save(tokens: &Tokens) -> io::Result<()> {
    let _guard = STORAGE
        .lock()
        .map_err(|_| io::Error::other("token storage lock poisoned"))?;
    save_at(&path()?, tokens)
}

pub(super) fn load() -> io::Result<Option<Tokens>> {
    let _guard = STORAGE
        .lock()
        .map_err(|_| io::Error::other("token storage lock poisoned"))?;
    let path = path()?;
    match read_bounded(&path) {
        Ok(bytes) => {
            if bytes.len() < HEADER.len() + IV_LEN + TAG_LEN || !bytes.starts_with(HEADER) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Invalid encrypted Google token file",
                ));
            }
            let split = HEADER.len() + IV_LEN;
            let (plaintext, _) = crypt(&bytes[split..], Some(&bytes[HEADER.len()..split]))?;
            let tokens = toml::from_str(std::str::from_utf8(&plaintext).map_err(io::Error::other)?)
                .map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Invalid decrypted Google tokens",
                    )
                })?;
            remove_legacy()?;
            Ok(Some(tokens))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            // One-time upgrade from the old plaintext file. Encryption failure
            // is an error, never a reason to continue using plaintext storage.
            let Some(legacy) = super::token_path() else {
                return Ok(None);
            };
            let bytes = match read_bounded(&legacy) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    // A crash in an older version may have left only the
                    // plaintext temporary file. It must not linger in backups.
                    remove_legacy()?;
                    return Ok(None);
                }
                Err(error) => return Err(error),
            };
            let tokens = toml::from_str(std::str::from_utf8(&bytes).map_err(io::Error::other)?)
                .map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "Invalid legacy Google tokens")
                })?;
            save_at(&path, &tokens)?;
            Ok(Some(tokens))
        }
        Err(error) => Err(error),
    }
}

pub(super) fn clear() -> io::Result<()> {
    let _guard = STORAGE
        .lock()
        .map_err(|_| io::Error::other("token storage lock poisoned"))?;
    let path = path()?;
    remove_if_present(&path)?;
    remove_if_present(&path.with_extension("enc.tmp"))?;
    remove_legacy()?;
    with_jni(|env| {
        let store = key_store(env)?;
        let alias = env.new_string(ALIAS)?;
        env.call_method(
            store,
            "deleteEntry",
            "(Ljava/lang/String;)V",
            &[JValue::Object(&alias)],
        )?;
        Ok(())
    })
}
