// Embeds every license text the app distributes directly into the
// binary via include_str!. Paths are resolved at compile time relative
// to this file, so a missing file is a build error rather than a
// runtime surprise. The Acknowledgements panel renders a curated
// summary in-app and links out for the full third-party list, but the
// embedded copies remain available so an offline viewer can be wired in
// without changing the build.
use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

use super::CmdResult;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcknowledgementsText {
    pub mit_license: &'static str,
    pub notice: &'static str,
    pub third_party: &'static str,
    pub lgpl: &'static str,
    pub mpl: &'static str,
}

#[tauri::command]
pub fn get_acknowledgements_text() -> CmdResult<AcknowledgementsText> {
    Ok(AcknowledgementsText {
        mit_license: include_str!("../../../LICENSE"),
        notice: include_str!("../../../NOTICE.md"),
        third_party: include_str!("../../../THIRD_PARTY_LICENSES.md"),
        lgpl: include_str!("../../../licenses/LICENSE.LGPL-2.1"),
        mpl: include_str!("../../../licenses/LICENSE.MPL-2.0"),
    })
}

// Routes through `tauri-plugin-opener` so iOS uses `UIApplication.open`
// rather than the no-op `open` crate path. Restricted to https URLs so a
// hijacked renderer can't hand off `file://`, `mailto:`, or a custom
// URI scheme registered on the user's machine — the in-app callers
// only ever link to fixed GitHub URLs anyway.
#[tauri::command]
pub fn open_external_url(app: AppHandle, url: String) -> CmdResult<()> {
    if !url.starts_with("https://") {
        return Err("only https URLs are allowed".into());
    }
    app.opener()
        .open_url(&url, None::<&str>)
        .map_err(|e| e.to_string())
}
