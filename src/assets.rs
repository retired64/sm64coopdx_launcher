use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::path::Path;

pub struct FontCache<'ttf> {
    fonts: HashMap<u16, sdl2::ttf::Font<'ttf, 'static>>,
}

impl<'ttf> FontCache<'ttf> {
    pub fn new(
        ttf: &'ttf sdl2::ttf::Sdl2TtfContext,
        font_path: &Path,
        sizes: &[u16],
    ) -> Result<Self, String> {
        let mut fonts = HashMap::new();
        for &size in sizes {
            log::debug!("Caching font at size {}pt", size);
            fonts.insert(size, ttf.load_font(font_path, size)?);
        }
        Ok(Self { fonts })
    }

    pub fn get_font(
        &mut self,
        ttf: &'ttf sdl2::ttf::Sdl2TtfContext,
        font_path: &Path,
        size: u16,
    ) -> Result<&sdl2::ttf::Font<'ttf, 'static>, String> {
        match self.fonts.entry(size) {
            Entry::Occupied(entry) => Ok(entry.into_mut()),
            Entry::Vacant(entry) => {
                log::debug!("Font cache MISS: inserting size {}pt", size);
                let font = ttf.load_font(font_path, size)?;
                Ok(entry.insert(font))
            }
        }
    }
}
