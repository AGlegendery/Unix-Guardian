//! Signs release files for the self-update, without installing minisign.
//! The signatures are ordinary minisign ones (`minisign -Vm` checks them).
//!
//!   cargo run -p ug-core --example sign -- keygen DIR [--no-password]
//!       DIR/unix-guardian.key (secret: keep it off the repository) and
//!       DIR/unix-guardian.pub; prints the public key for
//!       ug_core::update::PUBLIC_KEY.
//!   cargo run -p ug-core --example sign -- KEY FILE... [--no-password]
//!       writes FILE.minisig next to each file.
//!
//! The password is asked for, or read from UG_SIGN_PASSWORD; tests use
//! --no-password.

use std::io::Write;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let bare = args.iter().any(|a| a == "--no-password");
    args.retain(|a| a != "--no-password");
    let password = || std::env::var("UG_SIGN_PASSWORD").ok();
    let r = match args.as_slice() {
        [k, dir] if k == "keygen" => keygen(std::path::Path::new(dir), bare, password()),
        [key, files @ ..] if !files.is_empty() => sign(key, files, bare, password()),
        _ => Err("usage: sign keygen DIR [--no-password] | sign KEY FILE... [--no-password]".into()),
    };
    if let Err(e) = r {
        eprintln!("sign: {e}");
        std::process::exit(1);
    }
}

fn keygen(dir: &std::path::Path, bare: bool, password: Option<String>) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let sk_path = dir.join("unix-guardian.key");
    if sk_path.exists() {
        return Err(format!("{} exists already; not overwriting a key", sk_path.display()));
    }
    let kp = if bare {
        minisign::KeyPair::generate_unencrypted_keypair()
    } else {
        minisign::KeyPair::generate_encrypted_keypair(password)
    }
    .map_err(|e| e.to_string())?;
    let sk = kp.sk.to_box(Some("Unix Guardian release key")).map_err(|e| e.to_string())?.into_string();
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode_600()
        .open(&sk_path)
        .map_err(|e| e.to_string())?;
    f.write_all(sk.as_bytes()).map_err(|e| e.to_string())?;
    let pk = kp.pk.to_box().map_err(|e| e.to_string())?.into_string();
    std::fs::write(dir.join("unix-guardian.pub"), pk).map_err(|e| e.to_string())?;
    println!("{}", kp.pk.to_base64());
    Ok(())
}

fn sign(key: &str, files: &[String], bare: bool, password: Option<String>) -> Result<(), String> {
    let sk = if bare {
        let b = minisign::SecretKeyBox::from_string(&std::fs::read_to_string(key).map_err(|e| format!("{key}: {e}"))?).map_err(|e| e.to_string())?;
        b.into_unencrypted_secret_key()
    } else {
        minisign::SecretKey::from_file(key, password)
    }
    .map_err(|e| e.to_string())?;
    for f in files {
        let data = std::fs::File::open(f).map_err(|e| format!("{f}: {e}"))?;
        let name = std::path::Path::new(f).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let sig = minisign::sign(None, &sk, data, Some(&format!("file:{name}")), None).map_err(|e| e.to_string())?;
        std::fs::write(format!("{f}.minisig"), sig.to_string()).map_err(|e| e.to_string())?;
        println!("signed {f}");
    }
    Ok(())
}

trait Mode600 {
    fn mode_600(&mut self) -> &mut Self;
}

impl Mode600 for std::fs::OpenOptions {
    fn mode_600(&mut self) -> &mut Self {
        use std::os::unix::fs::OpenOptionsExt;
        self.mode(0o600)
    }
}
