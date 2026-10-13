//! Prints every installed game found under `$HOME`, as JSON.
//!
//! `cargo run -p ug-games --example scan`
fn main() {
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .expect("HOME set");
    let games = ug_games::scan_installed(&home);
    println!("{}", serde_json::to_string_pretty(&games).unwrap());
    eprintln!("{} installed games", games.len());
}
