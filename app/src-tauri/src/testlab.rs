//! The Test lab page: runs `ug-testlab` (the same engine as `ug testlab`
//! and the terminal UI), reports progress as `testlab-progress` events,
//! and saves the report where the user can find it.

use tauri::{AppHandle, Emitter};

fn options() -> ug_testlab::Options {
    let session = std::env::var_os("UG_SESSION").is_some_and(|v| v == "1");
    ug_testlab::Options::new("window", session)
}

#[tauri::command]
pub async fn testlab_run(app: AppHandle) -> Result<ug_testlab::Report, String> {
    ug_testlab::run(&options(), move |p| {
        let _ = app.emit("testlab-progress", &p);
    })
    .await
}

/// Puts back what an interrupted test left; the sentences say what.
#[tauri::command]
pub async fn testlab_recover() -> Vec<String> {
    ug_testlab::recover(&options()).await
}

/// Writes the report into Downloads; returns the file's path.
#[tauri::command]
pub fn testlab_save(text: String) -> Result<String, String> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let stamp = ug_core::playtime::local_time(now).replace([' ', ':'], "-");
    let path = ug_core::playtime::downloads_dir().join(format!("unix-guardian-test-report-{stamp}.txt"));
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path.display().to_string())
}

/// At start: a test the window was running when it closed or crashed is
/// put back before anything else happens.
pub fn recover_in_background() {
    tauri::async_runtime::spawn(async {
        for p in ug_testlab::recover(&options()).await {
            eprintln!("Unix Guardian test lab: put back from an interrupted test: {p}");
        }
    });
}
