// Copyright 2024 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Query support.

use crate::{Charmap, CharmapIndex};

use super::super::{Collection, SourceCache};

use alloc::vec::Vec;
use parlance::Script;

use super::{
    super::{Attributes, Blob, FallbackKey, FamilyId, FamilyInfo, GenericFamily, Synthesis},
    Inner,
};

#[derive(Clone, Default)]
pub(super) struct QueryState {
    families: Vec<CachedFamily>,
    fallback_families: Vec<CachedFamily>,
}

impl QueryState {
    fn clear(&mut self) {
        self.families.clear();
        self.fallback_families.clear();
    }
}

/// State for font selection.
///
/// Instances of this can be obtained from [`Collection::query`].
pub struct Query<'a> {
    collection: &'a mut Inner,
    state: &'a mut QueryState,
    source_cache: &'a mut SourceCache,
    attributes: Attributes,
    fallbacks: Option<FallbackKey>,
}

impl<'a> Query<'a> {
    pub(super) fn new(collection: &'a mut Collection, source_cache: &'a mut SourceCache) -> Self {
        collection.query_state.clear();
        Self {
            collection: &mut collection.inner,
            state: &mut collection.query_state,
            source_cache,
            attributes: Attributes::default(),
            fallbacks: None,
        }
    }

    /// Sets the ordered sequence of families to match against.
    pub fn set_families<'f, I>(&mut self, families: I)
    where
        I: IntoIterator,
        I::Item: Into<QueryFamily<'f>>,
    {
        self.state.families.clear();
        for family in families {
            let family = family.into();
            match family {
                QueryFamily::Named(name) => {
                    if let Some(id) = self.collection.family_id(name) {
                        self.state.families.push(CachedFamily::new(id));
                    }
                },
                QueryFamily::Id(id) => {
                    self.state.families.push(CachedFamily::new(id));
                },
                QueryFamily::Generic(generic) => {
                    for id in self.collection.generic_families(generic) {
                        self.state.families.push(CachedFamily::new(id));
                    }
                },
            }
        }
    }

    /// Sets the primary attributes to match against.
    pub fn set_attributes(&mut self, attributes: Attributes) {
        if self.attributes != attributes {
            for family in &mut self.state.families {
                family.clear_fonts();
            }
            for family in &mut self.state.fallback_families {
                family.clear_fonts();
            }
            self.attributes = attributes;
        }
    }

    /// Sets the script and locale for fallback fonts.
    pub fn set_fallbacks(&mut self, key: impl Into<FallbackKey>) {
        let key = key.into();
        if self.fallbacks != Some(key) {
            self.state.fallback_families.clear();
            self.state.fallback_families.extend(
                self.collection
                    .fallback_families(key)
                    .map(CachedFamily::new),
            );
            // HACK: always add a Han font to the fallback list to capture
            // punctuation in the common script
            // See <https://github.com/linebender/parley/issues/597>
            self.state.fallback_families.extend(
                self.collection
                    .fallback_families(FallbackKey::new(Script::from_bytes(*b"Hani"), None))
                    .map(CachedFamily::new),
            );
            self.fallbacks = Some(key);
        }
    }

    /// Sets fallbacks for one text cluster. On Windows this asks DirectWrite
    /// for that cluster's actual codepoints when no explicit fallback is set.
    pub fn set_fallbacks_for_text(&mut self, key: impl Into<FallbackKey>, text: &str) {
        #[cfg(not(all(target_os = "windows", feature = "system")))]
        {
            let _ = text;
            self.set_fallbacks(key);
            return;
        }
        #[cfg(all(target_os = "windows", feature = "system"))]
        self.set_fallbacks_for_text_windows(key.into(), text);
    }

    #[cfg(all(target_os = "windows", feature = "system"))]
    fn set_fallbacks_for_text_windows(&mut self, key: FallbackKey, text: &str) {
        self.state.fallback_families.clear();
        self.state.fallback_families.extend(
            self.collection
                .fallback_families_for_text(key, text, self.attributes)
                .into_iter()
                .map(CachedFamily::new),
        );
        self.state.fallback_families.extend(
            self.collection
                .fallback_families(FallbackKey::new(Script::from_bytes(*b"Hani"), None))
                .map(CachedFamily::new),
        );
        // These families came from text rather than the script cache. A later
        // `set_fallbacks` for the same key must rebuild its script fallback.
        self.fallbacks = None;
    }

    /// Invokes the callback only for explicitly requested families.
    pub fn matches_primary_with(&mut self, f: impl FnMut(&QueryFont) -> QueryStatus) -> bool {
        Self::matches_families_with(
            self.collection,
            &mut self.state.families,
            self.source_cache,
            self.attributes,
            f,
        )
    }

    /// Invokes the callback only for fallback families.
    pub fn matches_fallbacks_with(&mut self, f: impl FnMut(&QueryFont) -> QueryStatus) -> bool {
        Self::matches_families_with(
            self.collection,
            &mut self.state.fallback_families,
            self.source_cache,
            self.attributes,
            f,
        )
    }

    /// Invokes the given callback with all fonts that match the current
    /// settings.
    ///
    /// Return [`QueryStatus::Stop`] to end iterating over the matching
    /// fonts or [`QueryStatus::Continue`] to continue iterating.
    pub fn matches_with(&mut self, mut f: impl FnMut(&QueryFont) -> QueryStatus) {
        if self.matches_primary_with(&mut f) {
            return;
        }
        self.matches_fallbacks_with(f);
    }

    fn matches_families_with(
        collection: &mut Inner,
        families: &mut [CachedFamily],
        source_cache: &mut SourceCache,
        attributes: Attributes,
        mut f: impl FnMut(&QueryFont) -> QueryStatus,
    ) -> bool {
        for family in families {
            match &mut family.family {
                Entry::Error => continue,
                Entry::Ok(..) => {},
                status @ Entry::Vacant => {
                    if let Some(info) = collection.family(family.id) {
                        *status = Entry::Ok(info);
                    } else {
                        *status = Entry::Error;
                        continue;
                    }
                },
            }
            let Entry::Ok(family_info) = &family.family else {
                continue;
            };
            let mut best_index = None;
            if let Some(font) = load_font(
                family_info,
                attributes,
                &mut family.best,
                false,
                source_cache,
            ) {
                best_index = Some(font.family.1);
                if f(font) == QueryStatus::Stop {
                    return true;
                }
            }
            // Don't invoke for the default font if it's the same as the
            // best match.
            if best_index == Some(family_info.default_font_index()) {
                continue;
            }
            if let Some(font) = load_font(
                family_info,
                attributes,
                &mut family.default,
                true,
                source_cache,
            ) {
                if f(font) == QueryStatus::Stop {
                    return true;
                }
            }
        }
        false
    }
}

impl Drop for Query<'_> {
    fn drop(&mut self) {
        self.state.clear();
    }
}

#[cfg(all(test, target_os = "windows", feature = "system"))]
mod tests {
    use super::*;
    use crate::{CollectionOptions, Script};

    #[test]
    fn text_fallback_does_not_stick_to_a_same_key_script_query() {
        let key = FallbackKey::new(Script::from_bytes(*b"Latn"), None);
        let mut collection = Collection::new(CollectionOptions::default());
        let expected = collection.fallback_families(key).collect::<Vec<_>>();
        if expected.is_empty() {
            return;
        }
        let mut cache = SourceCache::default();
        let mut query = collection.query(&mut cache);
        query.set_fallbacks_for_text(key, "\u{25be}");
        query.set_fallbacks(key);
        assert_eq!(query.state.fallback_families[0].id, expected[0]);
    }
}

/// Determines whether a font query operation will continue.
///
/// See [`Query::matches_with`].
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum QueryStatus {
    /// Query should continue with the next font.
    Continue,
    /// Query should stop.
    Stop,
}

/// Family descriptor for a font query.
///
/// This allows [`Query::set_families`] to
/// take a variety of family types.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum QueryFamily<'a> {
    /// A named font family.
    Named(&'a str),
    /// An identifier for a font family.
    Id(FamilyId),
    /// A generic font family.
    Generic(GenericFamily),
}

impl<'a> From<&'a str> for QueryFamily<'a> {
    fn from(value: &'a str) -> Self {
        Self::Named(value)
    }
}

impl From<FamilyId> for QueryFamily<'static> {
    fn from(value: FamilyId) -> Self {
        Self::Id(value)
    }
}

impl From<GenericFamily> for QueryFamily<'static> {
    fn from(value: GenericFamily) -> Self {
        Self::Generic(value)
    }
}

/// Candidate font generated by a [`Query`].
#[derive(Clone, Debug)]
pub struct QueryFont {
    /// Family identifier and index of the font in the family font list.
    pub family: (FamilyId, usize),
    /// Blob containing the font data.
    pub blob: Blob<u8>,
    /// Index of a font in a font collection (`ttc`) file.
    pub index: u32,
    /// Synthesis suggestions for this font based on the requested attributes.
    pub synthesis: Synthesis,
    /// Data used for constructing a character map for this font.
    pub charmap_index: CharmapIndex,
}

impl QueryFont {
    /// Attempts to construct a [Charmap] for this font.
    pub fn charmap(&self) -> Option<Charmap<'_>> {
        self.charmap_index.charmap(self.blob.as_ref())
    }
}

fn load_font<'a>(
    family: &FamilyInfo,
    attributes: Attributes,
    font: &'a mut Entry<QueryFont>,
    is_default: bool,
    source_cache: &mut SourceCache,
) -> Option<&'a QueryFont> {
    match font {
        Entry::Error => None,
        Entry::Ok(font) => Some(font),
        status @ Entry::Vacant => {
            // Set to error in case we fail. This simplifies
            // the following code.
            *status = Entry::Error;
            let family_index = if is_default {
                family.default_font_index()
            } else {
                family.match_index(attributes.width, attributes.style, attributes.weight, true)?
            };
            let font_info = family.fonts().get(family_index)?;
            let blob = font_info.load(Some(source_cache))?;
            let blob_index = font_info.index();
            let synthesis =
                font_info.synthesis(attributes.width, attributes.style, attributes.weight);
            *status = Entry::Ok(QueryFont {
                family: (family.id(), family_index),
                blob: blob.clone(),
                index: blob_index,
                synthesis,
                charmap_index: font_info.charmap_index(),
            });
            if let Entry::Ok(font) = status {
                Some(font)
            } else {
                None
            }
        },
    }
}

#[derive(Clone)]
struct CachedFamily {
    id: FamilyId,
    family: Entry<FamilyInfo>,
    best: Entry<QueryFont>,
    default: Entry<QueryFont>,
}

impl CachedFamily {
    fn new(id: FamilyId) -> Self {
        Self {
            id,
            family: Entry::Vacant,
            best: Entry::Vacant,
            default: Entry::Vacant,
        }
    }

    fn clear_fonts(&mut self) {
        self.best = Entry::Vacant;
        self.default = Entry::Vacant;
    }
}

#[derive(Clone)]
enum Entry<T> {
    Ok(T),
    Vacant,
    Error,
}
