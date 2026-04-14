use rusqlite::params;

use crate::models::{AlbumColorInfo, UltraBlurColors, VibrantPalette};

use super::db::{CacheDatabase, CacheError};

impl CacheDatabase {
    /// UltraBlur colors and cached vibrant palette for an album.
    pub fn album_colors(&self, source_id: &str) -> Result<AlbumColorInfo, CacheError> {
        let conn = self.conn.lock();
        let r: Result<(Option<String>, Option<String>), _> = conn.query_row(
            "SELECT ultraBlurColors, vibrantPalette FROM albums WHERE sourceId = ?1",
            params![source_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        );
        match r {
            Ok((colors_json, palette_json)) => {
                let colors = match colors_json {
                    Some(j) => Some(serde_json::from_str::<UltraBlurColors>(&j)?),
                    None => None,
                };
                let palette = match palette_json {
                    Some(j) => Some(serde_json::from_str::<VibrantPalette>(&j)?),
                    None => None,
                };
                Ok(AlbumColorInfo { colors, palette })
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                Ok(AlbumColorInfo { colors: None, palette: None })
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Store vibrant palette JSON computed client-side.
    pub fn set_album_palette(
        &self,
        source_id: &str,
        palette: &VibrantPalette,
    ) -> Result<(), CacheError> {
        let json = serde_json::to_string(palette)?;
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE albums SET vibrantPalette = ?1 WHERE sourceId = ?2",
            params![json, source_id],
        )?;
        Ok(())
    }
}
