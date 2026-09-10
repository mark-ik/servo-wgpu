/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Env-gated DOM mutation capture for the scripted tier.

use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use genet_scripted_dom::{CapturedNodeId, NodeId, NodeIdentityError, ScriptedDom};
use layout_dom_api::{CapturedQualName, DomMutation, LayoutDom, LayoutDomMut};
use serde::{Deserialize, Serialize};

fn capture_dir() -> Option<&'static PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| std::env::var_os("GENET_DOM_CAPTURE_DIR").map(PathBuf::from))
        .as_ref()
}

fn capture_viewport_seed() -> io::Result<(u32, u32)> {
    Ok((
        capture_dimension("GENET_DOM_CAPTURE_WIDTH", 1280)?,
        capture_dimension("GENET_DOM_CAPTURE_HEIGHT", 720)?,
    ))
}

fn capture_dimension(name: &str, default: u32) -> io::Result<u32> {
    let Some(raw) = std::env::var_os(name) else {
        return Ok(default);
    };
    let value = raw.into_string().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name} must be valid UTF-8"),
        )
    })?;
    let parsed = value.parse::<u32>().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name} must be a positive integer"),
        )
    })?;
    if parsed == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name} must be greater than zero"),
        ));
    }
    Ok(parsed)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum DomCaptureRecord {
    SessionStart {
        snapshot_html: String,
        stylesheets: Vec<String>,
        layout_width: u32,
        layout_height: u32,
    },
    MutationBatch {
        mutations: Vec<RecordedMutation>,
        layout: Option<RecordedLayoutBatch>,
    },
}

pub(crate) struct DomCaptureRecorder {
    writer: BufWriter<File>,
    stylesheets: Vec<String>,
    layout_width: u32,
    layout_height: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum RecordedMutation {
    Inserted {
        node: CapturedNodeId,
        parent: CapturedNodeId,
        next_sibling: Option<CapturedNodeId>,
        outer_html: String,
    },
    Removed {
        node: CapturedNodeId,
        former_parent: CapturedNodeId,
        still_live: bool,
    },
    AttributeChanged {
        node: CapturedNodeId,
        name: CapturedQualName,
        old_value: Option<String>,
        new_value: Option<String>,
    },
    CharacterDataChanged {
        node: CapturedNodeId,
        new_data: String,
    },
    SubtreeReplaced {
        node: CapturedNodeId,
        new_inner_html: String,
    },
    /// An atomic in-tree move (`move_before`): the subtree survives, so no
    /// serialized HTML rides along — replay re-parents the live node.
    Moved {
        node: CapturedNodeId,
        from_parent: CapturedNodeId,
        to_parent: CapturedNodeId,
        next_sibling: Option<CapturedNodeId>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum RecordedApplied {
    Unchanged,
    RepaintOnly,
    Restyled,
    Spliced,
    FullRecompute,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct RecordedViewport {
    width: i32,
    height: i32,
    scroll_x_bits: u32,
    scroll_y_bits: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct RecordedLayoutBatch {
    applied: RecordedApplied,
    fragment_digest: u64,
    viewport: RecordedViewport,
}

impl RecordedMutation {
    fn capture(dom: &ScriptedDom, mutation: &DomMutation<NodeId>) -> io::Result<Self> {
        let capture_id = |id| {
            dom.try_capture_node_identity(id)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
        };
        // Retained source journals may name an exported node. Its birth-arena
        // serial is valid, but current-value readback from this store is not.
        // Removed needs only historical identities and its current liveness.
        let readback_node = match mutation {
            DomMutation::Inserted { node, .. }
            | DomMutation::AttributeChanged { node, .. }
            | DomMutation::CharacterDataChanged { node }
            | DomMutation::SubtreeReplaced { node }
            | DomMutation::Moved { node, .. } => Some(*node),
            DomMutation::Removed { .. } => None,
        };
        if readback_node.is_some_and(|node| !dom.is_live(node)) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "capture mutation target is no longer in this document",
            ));
        }
        Ok(match mutation {
            DomMutation::Inserted { node, parent } => Self::Inserted {
                node: capture_id(*node)?,
                parent: capture_id(*parent)?,
                next_sibling: dom.next_sibling(*node).map(capture_id).transpose()?,
                outer_html: dom.outer_html(*node),
            },
            DomMutation::Removed {
                node,
                former_parent,
            } => Self::Removed {
                node: capture_id(*node)?,
                former_parent: capture_id(*former_parent)?,
                still_live: dom.is_live(*node),
            },
            DomMutation::AttributeChanged {
                node,
                name,
                old_value,
            } => Self::AttributeChanged {
                node: capture_id(*node)?,
                name: name.into(),
                old_value: old_value.clone(),
                new_value: dom
                    .attribute(*node, &name.ns, &name.local)
                    .map(ToString::to_string),
            },
            DomMutation::CharacterDataChanged { node } => Self::CharacterDataChanged {
                node: capture_id(*node)?,
                new_data: dom.text(*node).unwrap_or_default().to_string(),
            },
            DomMutation::SubtreeReplaced { node } => Self::SubtreeReplaced {
                node: capture_id(*node)?,
                new_inner_html: dom.inner_html(*node),
            },
            DomMutation::Moved {
                node,
                from_parent,
                to_parent,
            } => Self::Moved {
                node: capture_id(*node)?,
                from_parent: capture_id(*from_parent)?,
                to_parent: capture_id(*to_parent)?,
                next_sibling: dom.next_sibling(*node).map(capture_id).transpose()?,
            },
        })
    }
}

/// Why a captured identity could not be translated into the replaying arena.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ReplayError {
    /// The record names an arena this document never imported from, so no
    /// serial in it can be resolved here. Reminting the bare serial would pick
    /// whichever node this arena happens to have allocated at that index.
    UnknownOrigin(CapturedNodeId),
    /// The origin is known but the identity is not usable here (never
    /// allocated, retired, or out of range).
    Unresolvable(CapturedNodeId, NodeIdentityError),
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownOrigin(id) => write!(
                f,
                "captured node {}:{} names an arena this document never imported from",
                id.arena, id.serial
            ),
            Self::Unresolvable(id, error) => {
                write!(f, "captured node {}:{}: {error}", id.arena, id.serial)
            },
        }
    }
}
impl std::error::Error for ReplayError {}

impl RecordedMutation {
    /// Translate one captured identity through the replaying arena's import
    /// registry. Local origins resolve by serial; an imported origin resolves
    /// only when this arena actually recorded the adoption that brought it in.
    fn translate(dom: &ScriptedDom, captured: CapturedNodeId) -> Result<NodeId, ReplayError> {
        dom.try_remint_node_identity(captured)
            .map_err(|error| match error {
                NodeIdentityError::UnknownOriginArena => ReplayError::UnknownOrigin(captured),
                other => ReplayError::Unresolvable(captured, other),
            })
    }

    /// The node this record is about, in the replaying arena's identity space.
    pub(crate) fn replay_node(&self, dom: &ScriptedDom) -> Result<NodeId, ReplayError> {
        let node = match self {
            Self::Inserted { node, .. }
            | Self::Removed { node, .. }
            | Self::AttributeChanged { node, .. }
            | Self::CharacterDataChanged { node, .. }
            | Self::SubtreeReplaced { node, .. }
            | Self::Moved { node, .. } => *node,
        };
        Self::translate(dom, node)
    }

    /// Every identity this record names, in record order. A replayer needs all
    /// of them resolved before it touches the document, so one unknown origin
    /// refuses the record rather than half-applying it.
    pub(crate) fn replay_ids(&self, dom: &ScriptedDom) -> Result<Vec<NodeId>, ReplayError> {
        let captured: Vec<CapturedNodeId> = match self {
            Self::Inserted {
                node,
                parent,
                next_sibling,
                ..
            } => std::iter::once(*node)
                .chain(std::iter::once(*parent))
                .chain(*next_sibling)
                .collect(),
            Self::Removed {
                node,
                former_parent,
                ..
            } => vec![*node, *former_parent],
            Self::AttributeChanged { node, .. }
            | Self::CharacterDataChanged { node, .. }
            | Self::SubtreeReplaced { node, .. } => vec![*node],
            Self::Moved {
                node,
                from_parent,
                to_parent,
                next_sibling,
            } => std::iter::once(*node)
                .chain(std::iter::once(*from_parent))
                .chain(std::iter::once(*to_parent))
                .chain(*next_sibling)
                .collect(),
        };
        captured
            .into_iter()
            .map(|id| Self::translate(dom, id))
            .collect()
    }
}

impl DomCaptureRecorder {
    pub(crate) fn from_env(
        dom: &mut ScriptedDom,
        stylesheets: &[String],
    ) -> io::Result<Option<Self>> {
        let Some(dir) = capture_dir() else {
            return Ok(None);
        };
        Self::open_in_dir(dir, dom, stylesheets).map(Some)
    }

    fn open_in_dir(dir: &Path, dom: &mut ScriptedDom, stylesheets: &[String]) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let path = dir.join(session_file_name());
        Self::open_at_path(&path, dom, stylesheets)
    }

    pub(crate) fn open_at_path(
        path: &Path,
        dom: &mut ScriptedDom,
        stylesheets: &[String],
    ) -> io::Result<Self> {
        let (layout_width, layout_height) = capture_viewport_seed()?;
        let mut recorder = Self {
            writer: BufWriter::new(File::create(path)?),
            stylesheets: stylesheets.to_vec(),
            layout_width,
            layout_height,
        };
        recorder.write_record(&DomCaptureRecord::SessionStart {
            snapshot_html: dom.inner_html(dom.document()),
            stylesheets: recorder.stylesheets.clone(),
            layout_width: recorder.layout_width,
            layout_height: recorder.layout_height,
        })?;
        // The initial snapshot is post-parse DOM state, so the bootstrap clone
        // mutations are baseline, not replayable deltas.
        let mut bootstrap = Vec::new();
        dom.drain_mutations(&mut bootstrap);
        Ok(recorder)
    }

    pub(crate) fn record_pending(&mut self, dom: &mut ScriptedDom) -> io::Result<usize> {
        // Validate the entire batch before consuming its mutation journal or
        // writing any record. Imported identities require a capture translation
        // protocol; refuse them through the recorder's ordinary disable path.
        let (_, pending) = dom.pending_mutations();
        if pending.is_empty() {
            return Ok(0);
        }
        let mutations = pending
            .iter()
            .map(|m| RecordedMutation::capture(dom, m))
            .collect::<io::Result<Vec<_>>>()?;
        let mut pending = Vec::new();
        dom.drain_mutations(&mut pending);
        // Layout parity capture (a shadow `genet_layout::IncrementalLayout` kept
        // beside the recorder, applying each batch to record fragment-digest +
        // viewport alongside the DOM mutations) was retired with the
        // genet-layout route: this crate never declared the `render` feature
        // that code was gated on, so it neither compiled nor ran. Rather than
        // resurrect a shadow layout for a route that no longer exists, this
        // recorder now records DOM mutations only; `layout` stays in the wire
        // format for backward-compatible deserialization of old captures.
        let layout = None;
        self.write_record(&DomCaptureRecord::MutationBatch { mutations, layout })?;
        Ok(pending.len())
    }

    fn write_record(&mut self, record: &DomCaptureRecord) -> io::Result<()> {
        let bytes = postcard::to_stdvec(record)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
        let len = u32::try_from(bytes.len()).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "dom capture record too large")
        })?;
        self.writer.write_all(&len.to_le_bytes())?;
        self.writer.write_all(&bytes)?;
        self.writer.flush()
    }
}

#[cfg(test)]
pub(crate) fn read_capture_records(path: &Path) -> io::Result<Vec<DomCaptureRecord>> {
    use std::io::Read;
    let mut file = File::open(path)?;
    let mut out = Vec::new();
    loop {
        let mut len_bytes = [0u8; 4];
        match file.read_exact(&mut len_bytes) {
            Ok(()) => {},
            Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(err) => return Err(err),
        }
        let len = u32::from_le_bytes(len_bytes) as usize;
        let mut bytes = vec![0u8; len];
        file.read_exact(&mut bytes)?;
        let record = postcard::from_bytes(&bytes)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
        out.push(record);
    }
    Ok(out)
}

fn session_file_name() -> String {
    format!("dom-capture-{}.postcard", now_millis())
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use layout_dom_api::{LayoutDomMut, LocalName, Namespace, QualName};
    use std::sync::atomic::{AtomicU64, Ordering};

    fn qual(local: &str) -> QualName {
        QualName::new(None, Namespace::from(""), LocalName::from(local))
    }

    fn temp_capture_path() -> PathBuf {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let unique = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("genet-dom-capture-{}-{unique}.bin", now_millis()))
    }

    #[test]
    fn recorder_records_an_adopted_node_and_replay_resolves_the_same_live_node() {
        // An imported node's serial belongs to the arena that minted it, so the
        // record names that arena and replay translates through the importing
        // store's registry. This case used to be refused outright.
        let mut source = ScriptedDom::new();
        let source_arena = source.arena_id();
        let imported = source.create_element(qual("p"));
        let text = source.create_text("retained");
        source.append_child(imported, text);
        source.drain_mutations(&mut Vec::new());
        let mut dom = ScriptedDom::new();
        let root = dom.document();
        let local = dom.create_element(qual("main"));
        dom.append_child(root, local);
        source
            .transfer_detached_subtree_to(&mut dom, imported)
            .unwrap();
        assert!(dom.imported_arenas().any(|arena| arena == source_arena));
        let path = temp_capture_path();
        let mut recorder = DomCaptureRecorder::open_at_path(&path, &mut dom, &[]).unwrap();
        dom.append_child(local, imported);
        dom.set_attribute(imported, qual("id"), "adopted");
        assert_eq!(recorder.record_pending(&mut dom).unwrap(), 2);
        let records = read_capture_records(&path).unwrap();
        let DomCaptureRecord::MutationBatch { mutations, .. } = &records[1] else {
            panic!("unexpected record: {:?}", records[1]);
        };
        let RecordedMutation::AttributeChanged { node, .. } = &mutations[1] else {
            panic!("unexpected mutation: {:?}", mutations[1]);
        };
        assert_eq!(
            node.arena, source_arena,
            "the record names its origin arena"
        );
        assert_ne!(node.arena, dom.arena_id());
        for mutation in mutations {
            assert_eq!(mutation.replay_node(&dom).unwrap(), imported);
            assert!(
                mutation
                    .replay_ids(&dom)
                    .unwrap()
                    .iter()
                    .all(|id| dom.is_live(*id))
            );
        }
        assert_eq!(dom.text(text), Some("retained"));
        drop(recorder);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn replay_refuses_a_serial_from_an_unregistered_arena() {
        // Both stores independently allocate the same low serials, which is the
        // silent-corruption case: reminting the bare serial here would resolve
        // to this arena's own unrelated node.
        let mut source = ScriptedDom::new();
        let node = source.create_element(qual("p"));
        source.append_child(source.document(), node);
        let mut muts = Vec::new();
        source.drain_mutations(&mut muts);
        let record = RecordedMutation::capture(&source, &muts[0]).unwrap();
        let RecordedMutation::Inserted { node: captured, .. } = &record else {
            panic!("unexpected mutation: {record:?}");
        };
        let mut elsewhere = ScriptedDom::new();
        let decoy = elsewhere.create_element(qual("div"));
        elsewhere.append_child(elsewhere.document(), decoy);
        assert_eq!(decoy.local_index(), node.local_index(), "serials collide");
        assert_eq!(
            record.replay_node(&elsewhere),
            Err(ReplayError::UnknownOrigin(*captured))
        );
        assert!(matches!(
            record.replay_ids(&elsewhere),
            Err(ReplayError::UnknownOrigin(_))
        ));
        // The same record replays correctly in the arena that minted it.
        assert_eq!(record.replay_node(&source).unwrap(), node);
    }

    #[test]
    fn recorder_refuses_exported_readback_without_consuming_source_journal() {
        let mut source = ScriptedDom::new();
        let node = source.create_element(qual("p"));
        let root = source.document();
        source.append_child(root, node);
        let path = temp_capture_path();
        let mut recorder = DomCaptureRecorder::open_at_path(&path, &mut source, &[]).unwrap();
        source.set_attribute(node, qual("id"), "exported");
        source.remove_child(node);
        let mut destination = ScriptedDom::new();
        source
            .transfer_detached_subtree_preserving_mutations_to(&mut destination, node)
            .unwrap();
        let before = source.pending_mutations().1.len();
        let error = recorder.record_pending(&mut source).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("no longer in this document"));
        assert_eq!(source.pending_mutations().1.len(), before);
        assert_eq!(read_capture_records(&path).unwrap().len(), 1);
        assert_eq!(
            destination.attribute(node, &Namespace::from(""), &LocalName::from("id")),
            Some("exported")
        );
        destination.append_child(destination.document(), node);
        destination.set_attribute(node, qual("id"), "still-usable");
        assert!(
            destination
                .inner_html(destination.document())
                .contains("still-usable")
        );
        // Historical removal remains representable even after physical export.
        let removal = RecordedMutation::capture(
            &source,
            &DomMutation::Removed {
                node,
                former_parent: root,
            },
        )
        .unwrap();
        assert!(matches!(
            removal,
            RecordedMutation::Removed {
                still_live: false,
                ..
            }
        ));
        drop(recorder);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn recorder_writes_snapshot_then_replayable_batches() {
        let mut dom = ScriptedDom::new();
        let root = dom.document();
        let body = dom.create_element(qual("body"));
        dom.append_child(root, body);

        let sheets = Vec::new();
        let path = temp_capture_path();
        let mut recorder = DomCaptureRecorder::open_at_path(&path, &mut dom, &sheets).unwrap();

        dom.set_attribute(body, qual("id"), "main");
        assert_eq!(recorder.record_pending(&mut dom).unwrap(), 1);

        let records = read_capture_records(&path).unwrap();
        let (layout_width, layout_height) = capture_viewport_seed().unwrap();
        assert_eq!(
            records[0],
            DomCaptureRecord::SessionStart {
                snapshot_html: "<body></body>".to_string(),
                stylesheets: sheets,
                layout_width,
                layout_height,
            }
        );
        match &records[1] {
            DomCaptureRecord::MutationBatch { mutations, layout } => {
                assert_eq!(
                    mutations,
                    &vec![RecordedMutation::AttributeChanged {
                        node: dom.try_capture_node_identity(body).unwrap(),
                        name: (&qual("id")).into(),
                        old_value: None,
                        new_value: Some("main".to_string()),
                    }]
                );
                // Layout-parity capture (the shadow `IncrementalLayout` this
                // recorder used to keep alongside the DOM mutation stream) was
                // retired with the genet-layout route; `layout` is always
                // `None` now. See `record_pending`.
                assert!(layout.is_none(), "layout parity capture is retired");
            },
            other => panic!("unexpected record: {other:?}"),
        }

        let _ = fs::remove_file(path);
    }

    #[test]
    fn recorder_captures_insert_position_and_removed_liveness() {
        let mut dom = ScriptedDom::new();
        let root = dom.document();
        let a = dom.create_element(qual("a"));
        let c = dom.create_element(qual("c"));
        dom.append_child(root, a);
        dom.append_child(root, c);

        let sheets = Vec::new();
        let path = temp_capture_path();
        let mut recorder = DomCaptureRecorder::open_at_path(&path, &mut dom, &sheets).unwrap();

        let b = dom.create_element(qual("b"));
        dom.insert_before(root, b, Some(c));
        assert_eq!(recorder.record_pending(&mut dom).unwrap(), 1);

        dom.remove_child(b);
        dom.remove(c);
        assert_eq!(recorder.record_pending(&mut dom).unwrap(), 2);

        let records = read_capture_records(&path).unwrap();
        match &records[1] {
            DomCaptureRecord::MutationBatch { mutations, .. } => {
                assert_eq!(
                    mutations,
                    &vec![RecordedMutation::Inserted {
                        node: dom.try_capture_node_identity(b).unwrap(),
                        parent: dom.try_capture_node_identity(root).unwrap(),
                        next_sibling: Some(dom.try_capture_node_identity(c).unwrap()),
                        outer_html: "<b></b>".to_string(),
                    }]
                );
            },
            other => panic!("unexpected record: {other:?}"),
        }
        match &records[2] {
            DomCaptureRecord::MutationBatch { mutations, .. } => {
                assert_eq!(
                    mutations,
                    &vec![
                        RecordedMutation::Removed {
                            node: dom.try_capture_node_identity(b).unwrap(),
                            former_parent: dom.try_capture_node_identity(root).unwrap(),
                            still_live: true,
                        },
                        RecordedMutation::Removed {
                            node: dom.try_capture_node_identity(c).unwrap(),
                            former_parent: dom.try_capture_node_identity(root).unwrap(),
                            still_live: false,
                        },
                    ]
                );
            },
            other => panic!("unexpected record: {other:?}"),
        }

        let _ = fs::remove_file(path);
    }
}
