// Embeds every license text the app distributes directly into the
// binary via include_str!. Paths are resolved at compile time relative
// to this file, so a missing file is a build error rather than a
// runtime surprise — the Acknowledgements panel is guaranteed to have
// content to show.
use serde::Serialize;

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
