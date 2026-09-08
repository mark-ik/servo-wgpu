/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Env-gated DOM mutation capture for the scripted tier.

use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use genet_scripted_dom::{NodeId, ScriptedDom};
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
        node: u64,
        parent: u64,
        next_sibling: Option<u64>,
        outer_html: String,
    },
    Removed {
        node: u64,
        former_parent: u64,
        still_live: bool,
    },
    AttributeChanged {
        node: u64,
        name: CapturedQualName,
        old_value: Option<String>,
        new_value: Option<String>,
    },
    CharacterDataChanged {
        node: u64,
        new_data: String,
    },
    SubtreeReplaced {
        node: u64,
        new_inner_html: String,
    },
    /// An atomic in-tree move (`move_before`): the subtree survives, so no
    /// serialized HTML rides along — replay re-parents the live node.
    Moved {
        node: u64,
        from_parent: u64,
        to_parent: u64,
        next_sibling: Option<u64>,
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
    fn capture(dom: &ScriptedDom, mutation: &DomMutation<NodeId>) -> Self {
        match mutation {
            DomMutation::Inserted { node, parent } => Self::Inserted {
                node: dom.capture_node_id(*node),
                parent: dom.capture_node_id(*parent),
                next_sibling: dom.next_sibling(*node).map(|id| dom.capture_node_id(id)),
                outer_html: dom.outer_html(*node),
            },
            DomMutation::Removed {
                node,
                former_parent,
            } => Self::Removed {
                node: dom.capture_node_id(*node),
                former_parent: dom.capture_node_id(*former_parent),
                still_live: dom.is_live(*node),
            },
            DomMutation::AttributeChanged {
                node,
                name,
                old_value,
            } => Self::AttributeChanged {
                node: dom.capture_node_id(*node),
                name: name.into(),
                old_value: old_value.clone(),
                new_value: dom
                    .attribute(*node, &name.ns, &name.local)
                    .map(ToString::to_string),
            },
            DomMutation::CharacterDataChanged { node } => Self::CharacterDataChanged {
                node: dom.capture_node_id(*node),
                new_data: dom.text(*node).unwrap_or_default().to_string(),
            },
            DomMutation::SubtreeReplaced { node } => Self::SubtreeReplaced {
                node: dom.capture_node_id(*node),
                new_inner_html: dom.inner_html(*node),
            },
            DomMutation::Moved {
                node,
                from_parent,
                to_parent,
            } => Self::Moved {
                node: dom.capture_node_id(*node),
                from_parent: dom.capture_node_id(*from_parent),
                to_parent: dom.capture_node_id(*to_parent),
                next_sibling: dom.next_sibling(*node).map(|id| dom.capture_node_id(id)),
            },
        }
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

    fn open_at_path(
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
        let mut pending = Vec::new();
        dom.drain_mutations(&mut pending);
        if pending.is_empty() {
            return Ok(0);
        }
        let mutations = pending
            .iter()
            .map(|m| RecordedMutation::capture(dom, m))
            .collect();
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
fn read_capture_records(path: &Path) -> io::Result<Vec<DomCaptureRecord>> {
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
                        node: dom.capture_node_id(body),
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
                        node: dom.capture_node_id(b),
                        parent: dom.capture_node_id(root),
                        next_sibling: Some(dom.capture_node_id(c)),
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
                            node: dom.capture_node_id(b),
                            former_parent: dom.capture_node_id(root),
                            still_live: true,
                        },
                        RecordedMutation::Removed {
                            node: dom.capture_node_id(c),
                            former_parent: dom.capture_node_id(root),
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
