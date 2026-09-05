// Copyright 2024 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::string::String;
use alloc::sync::Arc;
use alloc::{vec, vec::Vec};
use hashbrown::HashMap;
use std::{
    ffi::{OsString, c_void},
    os::windows::ffi::OsStringExt,
    path::PathBuf,
};
use windows::Win32::Foundation::STATUS_NOT_FOUND;
use windows::{
    Win32::Graphics::DirectWrite::{
        DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH, DWRITE_FONT_STRETCH_NORMAL,
        DWRITE_FONT_STYLE, DWRITE_FONT_STYLE_ITALIC, DWRITE_FONT_STYLE_NORMAL,
        DWRITE_FONT_STYLE_OBLIQUE, DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_REGULAR,
        DWRITE_READING_DIRECTION, DWRITE_READING_DIRECTION_LEFT_TO_RIGHT, DWriteCreateFactory,
        IDWriteFactory, IDWriteFactory2, IDWriteFont, IDWriteFontCollection, IDWriteFontFace,
        IDWriteFontFallback, IDWriteFontFamily, IDWriteFontFile, IDWriteLocalFontFileLoader,
        IDWriteNumberSubstitution, IDWriteTextAnalysisSource, IDWriteTextAnalysisSource_Impl,
    },
    core::{Interface, OutRef, PCWSTR, implement},
};

use super::{
    FallbackKey, FamilyId, FamilyInfo, FamilyNameMap, FontInfo, GenericFamily, GenericFamilyMap,
    ScriptExt, SourcePathMap,
};
use crate::{Attributes, FontStyle};

const DEFAULT_GENERIC_FAMILIES: &[(GenericFamily, &[&str])] = &[
    (GenericFamily::Serif, &["Times New Roman"]),
    (GenericFamily::SansSerif, &["Arial"]),
    (GenericFamily::Monospace, &["Consolas"]),
    (GenericFamily::Cursive, &["Comic Sans MS"]),
    (GenericFamily::Fantasy, &["Impact"]),
    (GenericFamily::SystemUi, &["Segoe UI"]),
    (GenericFamily::Emoji, &["Segoe UI Emoji"]),
    (GenericFamily::Math, &["Cambria Math"]),
    (GenericFamily::FangSong, &["FangSong"]),
];

/// Raw access to the collection of local system fonts.
pub(crate) struct SystemFonts {
    pub(crate) name_map: Arc<FamilyNameMap>,
    pub(crate) generic_families: Arc<GenericFamilyMap>,
    source_cache: SourcePathMap,
    family_map: HashMap<FamilyId, Option<FamilyInfo>>,
    dwrite_fonts: DWriteSystemFonts,
}

impl SystemFonts {
    pub(crate) fn new() -> Self {
        let dwrite_fonts = DWriteSystemFonts::new(false).unwrap();
        let mut name_map = FamilyNameMap::default();
        for family in dwrite_fonts.families() {
            if let Some(names) = family.names() {
                let [first_name, other_names @ ..] = names.as_slice() else {
                    continue;
                };
                let id = name_map.get_or_insert(first_name).id();
                for other_name in other_names {
                    name_map.add_alias(id, other_name);
                }
            }
        }
        let mut generic_families = GenericFamilyMap::default();
        for (family, names) in DEFAULT_GENERIC_FAMILIES {
            generic_families.set(
                *family,
                names
                    .iter()
                    .filter_map(|name| name_map.get(name))
                    .map(|name| name.id()),
            );
        }
        Self {
            name_map: Arc::new(name_map),
            generic_families: Arc::new(generic_families),
            source_cache: SourcePathMap::default(),
            family_map: HashMap::default(),
            dwrite_fonts,
        }
    }

    pub(crate) fn family(&mut self, id: FamilyId) -> Option<FamilyInfo> {
        match self.family_map.get(&id) {
            Some(Some(family)) => return Some(family.clone()),
            Some(None) => return None,
            _ => {},
        }
        let name = self.name_map.get_by_id(id)?;
        let mut fonts = smallvec::SmallVec::<[FontInfo; 4]>::default();
        if let Some(family) = self.dwrite_fonts.family_by_name(name.name()) {
            for font in family.fonts() {
                if let Some(font) = FontInfo::from_dwrite(&font, &mut self.source_cache) {
                    if !fonts
                        .iter()
                        .any(|f| f.source().id() == font.source().id() && f.index() == font.index())
                    {
                        fonts.push(font);
                    }
                }
            }
            if !fonts.is_empty() {
                let family = FamilyInfo::new(name.clone(), fonts);
                self.family_map.insert(id, Some(family.clone()));
                return Some(family);
            }
        }
        self.family_map.insert(id, None);
        None
    }

    pub(crate) fn fallback(&mut self, key: impl Into<FallbackKey>) -> Option<FamilyId> {
        // DirectWrite does not have a function to get the default font for a script and locale pair
        // so here we provide a sample of the intended script instead.
        let key = key.into();
        let text = key.script().sample()?;
        let locale = key.locale_str();
        let family_name = self.dwrite_fonts.family_name_for_text(text, locale)?;
        self.name_map.get(&family_name).map(|name| name.id())
    }

    /// Resolves one complete text range through DirectWrite's codepoint-aware
    /// fallback API. Partial ranges and scaled fallbacks cannot be represented
    /// by Fontique's family-only query API, so callers retain their existing
    /// fallback route for those cases.
    pub(crate) fn fallback_for_text(
        &mut self,
        key: impl Into<FallbackKey>,
        text: &str,
        attributes: Attributes,
    ) -> Option<FamilyId> {
        let key = key.into();
        let family_name =
            self.dwrite_fonts
                .family_name_for_complete_text(text, key.locale_str(), attributes)?;
        self.name_map.get(&family_name).map(|name| name.id())
    }
}

impl FontInfo {
    fn from_dwrite(font: &DWriteFont, paths: &mut SourcePathMap) -> Option<Self> {
        let path = font.file_path()?;
        let index = font.index();
        let data = paths.get_or_insert(&path);
        Self::from_source(data, index)
    }
}

struct DWriteSystemFonts {
    collection: IDWriteFontCollection,
    fallback: Option<IDWriteFontFallback>,
    map_buf: Vec<u16>,
}

impl DWriteSystemFonts {
    fn new(update: bool) -> windows_core::Result<Self> {
        unsafe {
            let factory = DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED)?;
            let mut collection: Option<IDWriteFontCollection> = None;
            factory.GetSystemFontCollection(&mut collection, update)?;
            let collection = collection.ok_or_else(|| {
                windows_core::Error::new(STATUS_NOT_FOUND.to_hresult(), "no collection")
            })?;
            let fallback = factory
                .cast::<IDWriteFactory2>()
                .and_then(|f| f.GetSystemFontFallback())
                .ok();
            Ok(Self {
                collection,
                fallback,
                map_buf: vec![],
            })
        }
    }

    fn get(&self, index: u32) -> Option<DWriteFontFamily> {
        unsafe {
            self.collection
                .GetFontFamily(index)
                .ok()
                .map(DWriteFontFamily)
        }
    }

    fn family_by_name(&self, name: &str) -> Option<DWriteFontFamily> {
        let mut index = 0;
        #[expect(
            clippy::default_trait_access,
            reason = "The type this resolves to doesn't seem to be nameable."
        )]
        // The documentation for `FindFamilyName` seems to be:
        // https://microsoft.github.io/windows-docs-rs/doc/windows/Win32/Graphics/DirectWrite/trait.IDWriteFontCollection_Impl.html#tymethod.FindFamilyName
        // And we need a value of type BOOL, which you'll notice is allegedly not actually public (i.e. it has no link).
        let mut exists = Default::default();
        let mut name_buf = smallvec::SmallVec::<[u16; 128]>::default();
        name_buf.extend(name.encode_utf16());
        name_buf.push(0);
        unsafe {
            self.collection
                .FindFamilyName(PCWSTR::from_raw(name_buf.as_ptr()), &mut index, &mut exists)
                .ok()?;
        }
        self.get(index)
    }

    fn families(&self) -> impl Iterator<Item = DWriteFontFamily> {
        let this = self.collection.clone();
        unsafe {
            let count = this.GetFontFamilyCount();
            (0..count).filter_map(move |index| this.GetFontFamily(index).ok().map(DWriteFontFamily))
        }
    }

    fn family_name_for_text(&mut self, text: &str, locale: Option<&str>) -> Option<String> {
        self.map_buf.clear();
        self.map_buf.extend(text.encode_utf16());
        let text_len = self.map_buf.len();
        if let Some(locale) = locale {
            self.map_buf.extend(locale.encode_utf16());
        }
        self.map_buf.push(0);
        let (text, locale) = self.map_buf.split_at(text_len);
        let source: IDWriteTextAnalysisSource = TextSource { text, locale }.into();
        unsafe {
            let mut cur_offset = 0;
            while cur_offset < text_len {
                let mut mapped_len = 0;
                let mut mapped_font = None;
                if let Some(fallback) = self.fallback.as_ref() {
                    if fallback
                        .MapCharacters(
                            &source,
                            cur_offset as u32,
                            (text_len - cur_offset) as u32,
                            &self.collection,
                            None,
                            DWRITE_FONT_WEIGHT_REGULAR,
                            DWRITE_FONT_STYLE_NORMAL,
                            DWRITE_FONT_STRETCH_NORMAL,
                            &mut mapped_len,
                            &mut mapped_font,
                            &mut 1.0,
                        )
                        .is_ok()
                    {
                        if let Some(font) = mapped_font {
                            return family_name_from_font(font);
                        }
                    }
                }
                cur_offset += 1;
            }
        }
        None
    }

    fn family_name_for_complete_text(
        &mut self,
        text: &str,
        locale: Option<&str>,
        attributes: Attributes,
    ) -> Option<String> {
        self.map_buf.clear();
        self.map_buf.extend(text.encode_utf16());
        let text_len = self.map_buf.len();
        if let Some(locale) = locale {
            self.map_buf.extend(locale.encode_utf16());
        }
        self.map_buf.push(0);
        let (text, locale) = self.map_buf.split_at(text_len);
        let source: IDWriteTextAnalysisSource = TextSource { text, locale }.into();
        unsafe {
            let fallback = self.fallback.as_ref()?;
            let mut mapped_len = 0;
            let mut mapped_font = None;
            let mut scale = 1.0;
            fallback
                .MapCharacters(
                    &source,
                    0,
                    text_len as u32,
                    &self.collection,
                    None,
                    dwrite_weight(attributes),
                    dwrite_style(attributes),
                    dwrite_stretch(attributes),
                    &mut mapped_len,
                    &mut mapped_font,
                    &mut scale,
                )
                .ok()?;
            if mapped_len as usize != text_len || scale != 1.0 {
                return None;
            }
            family_name_from_font(mapped_font?)
        }
    }
}

unsafe fn family_name_from_font(font: IDWriteFont) -> Option<String> {
    unsafe {
        let family = font.GetFontFamily().ok()?;
        let names = family.GetFamilyNames().ok()?;
        let name_len = names.GetStringLength(0).ok()? as usize;
        let mut name_buf = smallvec::SmallVec::<[u16; 128]>::default();
        name_buf.resize(name_len + 1, 0);
        names.GetString(0, &mut name_buf).ok()?;
        name_buf.pop();
        Some(String::from_utf16_lossy(&name_buf))
    }
}

fn dwrite_weight(attributes: Attributes) -> DWRITE_FONT_WEIGHT {
    DWRITE_FONT_WEIGHT(attributes.weight.value().round().clamp(1.0, 999.0) as i32)
}

fn dwrite_style(attributes: Attributes) -> DWRITE_FONT_STYLE {
    match attributes.style {
        FontStyle::Normal => DWRITE_FONT_STYLE_NORMAL,
        FontStyle::Italic => DWRITE_FONT_STYLE_ITALIC,
        FontStyle::Oblique(_) => DWRITE_FONT_STYLE_OBLIQUE,
    }
}

fn dwrite_stretch(attributes: Attributes) -> DWRITE_FONT_STRETCH {
    let ratio = attributes.width.ratio();
    let value = if ratio <= 0.5625 {
        1
    } else if ratio <= 0.6875 {
        2
    } else if ratio <= 0.8125 {
        3
    } else if ratio <= 0.9375 {
        4
    } else if ratio <= 1.0625 {
        5
    } else if ratio <= 1.1875 {
        6
    } else if ratio <= 1.375 {
        7
    } else if ratio <= 1.75 {
        8
    } else {
        9
    };
    DWRITE_FONT_STRETCH(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FontWeight, FontWidth};

    #[test]
    fn directwrite_mapping_preserves_requested_attributes() {
        let attributes = Attributes::new(
            FontWidth::EXPANDED,
            FontStyle::Oblique(Some(12.0)),
            FontWeight::new(650.0),
        );
        assert_eq!(dwrite_weight(attributes), DWRITE_FONT_WEIGHT(650));
        assert_eq!(dwrite_style(attributes), DWRITE_FONT_STYLE_OBLIQUE);
        assert_eq!(dwrite_stretch(attributes), DWRITE_FONT_STRETCH(7));
    }
}

#[derive(Clone)]
struct DWriteFontFamily(IDWriteFontFamily);

impl DWriteFontFamily {
    fn names(&self) -> Option<Vec<String>> {
        let mut names = vec![];
        unsafe {
            let family_names = self.0.GetFamilyNames().ok()?;
            let count = family_names.GetCount();
            let mut buf = vec![];
            for i in 0..count {
                let Ok(len) = family_names.GetStringLength(i) else {
                    continue;
                };
                buf.clear();
                buf.resize(len as usize + 1, 0);
                if family_names.GetString(i, &mut buf).is_err() {
                    continue;
                }
                buf.pop();
                names.push(String::from_utf16_lossy(&buf));
            }
        }
        Some(names)
    }

    fn fonts(&self) -> impl Iterator<Item = DWriteFont> {
        let this = self.0.clone();
        unsafe {
            let count = self.0.GetFontCount();
            (0..count).filter_map(move |index| {
                let font = this.GetFont(index).ok()?;
                // We don't want fonts with simulations
                if font.GetSimulations().0 != 0 {
                    return None;
                }
                DWriteFont::new(&font)
            })
        }
    }
}

// Note, this is a font face. We don't care about the font.
#[derive(Clone)]
struct DWriteFont(IDWriteFontFace);

impl DWriteFont {
    fn new(font: &IDWriteFont) -> Option<Self> {
        unsafe { Some(Self(font.CreateFontFace().ok()?)) }
    }

    fn file_path(&self) -> Option<PathBuf> {
        unsafe {
            // We only care about fonts with a single file.
            let mut file: Option<IDWriteFontFile> = None;
            self.0.GetFiles(&mut 1, Some(&mut file)).ok()?;
            let file = file?;
            // Now the ugly stuff...
            let mut ref_key: *mut c_void = core::ptr::null_mut();
            let mut ref_key_size: u32 = 0;
            file.GetReferenceKey(&mut ref_key, &mut ref_key_size).ok()?;
            let loader = file.GetLoader().ok()?;
            let local_loader: IDWriteLocalFontFileLoader = loader.cast().ok()?;
            let file_path_len = local_loader
                .GetFilePathLengthFromKey(ref_key, ref_key_size)
                .ok()?;
            let mut file_path_buf = vec![0; file_path_len as usize + 1];
            local_loader
                .GetFilePathFromKey(ref_key, ref_key_size, &mut file_path_buf)
                .ok()?;
            if let Some(&0) = file_path_buf.last() {
                file_path_buf.pop();
            }
            Some(PathBuf::from(OsString::from_wide(&file_path_buf)))
        }
    }

    fn index(&self) -> u32 {
        unsafe { self.0.GetIndex() }
    }
}

#[implement(IDWriteTextAnalysisSource)]
struct TextSource<'a> {
    text: &'a [u16],
    locale: &'a [u16],
}

impl IDWriteTextAnalysisSource_Impl for TextSource_Impl<'_> {
    fn GetLocaleName(
        &self,
        textposition: u32,
        textlength: *mut u32,
        localename: *mut *mut u16,
    ) -> windows::core::Result<()> {
        unsafe {
            *textlength = (self.text.len() as u32).saturating_sub(textposition);
            *localename = core::mem::transmute::<*const u16, *mut u16>(self.locale.as_ptr());
        }
        Ok(())
    }

    fn GetNumberSubstitution(
        &self,
        textposition: u32,
        textlength: *mut u32,
        numbersubstitution: OutRef<'_, IDWriteNumberSubstitution>,
    ) -> windows::core::Result<()> {
        numbersubstitution.write(None)?;
        unsafe {
            *textlength = (self.text.len() as u32).saturating_sub(textposition);
        }
        Ok(())
    }

    fn GetParagraphReadingDirection(&self) -> DWRITE_READING_DIRECTION {
        DWRITE_READING_DIRECTION_LEFT_TO_RIGHT
    }

    fn GetTextAtPosition(
        &self,
        textposition: u32,
        textstring: *mut *mut u16,
        textlength: *mut u32,
    ) -> windows::core::Result<()> {
        unsafe {
            let text = self.text.get(textposition as usize..).unwrap_or_default();
            *textlength = text.len() as _;
            *textstring = core::mem::transmute::<*const u16, *mut u16>(text.as_ptr());
        }
        Ok(())
    }

    fn GetTextBeforePosition(
        &self,
        textposition: u32,
        textstring: *mut *mut u16,
        textlength: *mut u32,
    ) -> windows::core::Result<()> {
        unsafe {
            let text = self.text.get(..textposition as usize).unwrap_or_default();
            *textlength = text.len() as _;
            *textstring = core::mem::transmute::<*const u16, *mut u16>(text.as_ptr());
        }
        Ok(())
    }
}
