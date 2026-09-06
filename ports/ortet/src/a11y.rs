// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Ortet's accessibility custody.
//!
//! `DocumentA11yNodeId` is local to one session. AccessKit requests carry only
//! a host node id, action, and data, so this module owns the publication record
//! that binds those raw requests to one session generation and one projection
//! revision. A fresh host id is allocated for every publication: a queued
//! request can therefore never be silently attributed to a newer revision or a
//! replacement session that reuses the same local document id.

use std::collections::HashMap;

use accesskit::{
    Action, ActionData, HasPopup, Live, Node as AccessNode, NodeId as AccessNodeId, Orientation,
    Role, Toggled, Tree, TreeId, TreeUpdate,
};
use document_session_api::session_engine::DocumentSession;
use document_session_api::{
    DocumentA11yAction, DocumentA11yActionData, DocumentA11yActionRequest, DocumentA11yHasPopup,
    DocumentA11yLive, DocumentA11yNode, DocumentA11yNodeId, DocumentA11yOrientation,
    DocumentA11yProjection, DocumentA11yRole, DocumentA11yState, DocumentA11yToggled,
};
use genet_winit_host::A11yActionRequest;
use netrender::Scene;

#[derive(Clone, Debug)]
struct PublishedNode {
    generation: u64,
    revision: u64,
    local: DocumentA11yNodeId,
    actions: Vec<DocumentA11yAction>,
}

/// The result of revalidating one raw platform request.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RoutedAction {
    Rejected,
    Dispatched,
    Click { x: f32, y: f32 },
}

/// Host-owned identity and custody for the tree currently advertised to the OS.
#[derive(Debug)]
pub(crate) struct Accessibility {
    generation: u64,
    next_host_id: u64,
    revision: Option<u64>,
    empty_published: bool,
    published: HashMap<AccessNodeId, PublishedNode>,
}

impl Default for Accessibility {
    fn default() -> Self {
        Self {
            generation: 1,
            next_host_id: 1,
            revision: None,
            empty_published: false,
            published: HashMap::new(),
        }
    }
}

impl Accessibility {
    pub(crate) fn replace_session(&mut self) {
        self.generation = self
            .generation
            .checked_add(1)
            .expect("Ortet a11y generation overflow");
        self.revision = None;
        self.empty_published = false;
        self.published.clear();
    }

    /// Lower a newly observed projection. `None` withdraws the former session's
    /// content with a content-free document tree; routing is cleared with it.
    pub(crate) fn publish(
        &mut self,
        projection: Option<DocumentA11yProjection>,
    ) -> Option<TreeUpdate> {
        let Some(projection) = projection else {
            if self.empty_published {
                return None;
            }
            return Some(self.publish_empty());
        };
        if self.revision == Some(projection.revision()) {
            return None;
        }

        self.published.clear();
        self.empty_published = false;
        let mut ids = HashMap::new();
        for node in projection.nodes() {
            ids.insert(node.id, self.allocate_host_id());
        }
        let root = *ids.get(&projection.root())?;
        let focus = projection
            .nodes()
            .iter()
            .find(|node| node.state.focused)
            .and_then(|node| ids.get(&node.id))
            .copied()
            .unwrap_or(root);
        let nodes = projection
            .nodes()
            .iter()
            .map(|node| {
                let host_id = ids[&node.id];
                self.published.insert(
                    host_id,
                    PublishedNode {
                        generation: self.generation,
                        revision: projection.revision(),
                        local: node.id,
                        actions: node.actions.clone(),
                    },
                );
                (host_id, lower_node(node, &ids))
            })
            .collect();
        self.revision = Some(projection.revision());
        Some(TreeUpdate {
            nodes,
            tree: Some(Tree::new(root)),
            tree_id: TreeId::ROOT,
            focus,
        })
    }

    fn publish_empty(&mut self) -> TreeUpdate {
        self.revision = None;
        self.empty_published = true;
        self.published.clear();
        let root = self.allocate_host_id();
        let node = AccessNode::new(Role::Document);
        TreeUpdate {
            nodes: vec![(root, node)],
            tree: Some(Tree::new(root)),
            tree_id: TreeId::ROOT,
            focus: root,
        }
    }

    pub(crate) fn route(
        &self,
        session: &mut dyn DocumentSession<Scene>,
        raw: &A11yActionRequest,
    ) -> RoutedAction {
        let Some(binding) = self.published.get(&raw.target_node) else {
            return RoutedAction::Rejected;
        };
        if binding.generation != self.generation {
            return RoutedAction::Rejected;
        }
        let Some(action) = document_action(raw.action, raw.data.as_ref()) else {
            return RoutedAction::Rejected;
        };
        if !binding.actions.contains(&action) {
            return RoutedAction::Rejected;
        }
        if action == DocumentA11yAction::Click {
            let Some(target) = session.accessibility_click_target(binding.local) else {
                return RoutedAction::Rejected;
            };
            return if target.revision == binding.revision {
                RoutedAction::Click {
                    x: target.point.x,
                    y: target.point.y,
                }
            } else {
                RoutedAction::Rejected
            };
        }
        let request = DocumentA11yActionRequest {
            revision: binding.revision,
            target: binding.local,
            action,
            data: document_action_data(raw.data.as_ref()),
        };
        if session.dispatch_accessibility_action(&request) {
            RoutedAction::Dispatched
        } else {
            RoutedAction::Rejected
        }
    }

    #[cfg(test)]
    fn host_id_for(&self, local: DocumentA11yNodeId) -> AccessNodeId {
        self.published
            .iter()
            .find_map(|(host, binding)| (binding.local == local).then_some(*host))
            .expect("local node is published")
    }

    fn allocate_host_id(&mut self) -> AccessNodeId {
        let id = self.next_host_id;
        self.next_host_id = self
            .next_host_id
            .checked_add(1)
            .expect("Ortet a11y host id overflow");
        AccessNodeId(id)
    }
}

fn document_action(action: Action, data: Option<&ActionData>) -> Option<DocumentA11yAction> {
    Some(match action {
        Action::Click => DocumentA11yAction::Click,
        Action::Focus => DocumentA11yAction::Focus,
        Action::SetValue if matches!(data, Some(ActionData::Value(_))) => {
            DocumentA11yAction::SetValue
        },
        Action::ScrollIntoView => DocumentA11yAction::ScrollIntoView,
        Action::Increment => DocumentA11yAction::Increment,
        Action::Decrement => DocumentA11yAction::Decrement,
        _ => return None,
    })
}

fn document_action_data(data: Option<&ActionData>) -> Option<DocumentA11yActionData> {
    match data {
        Some(ActionData::Value(value)) => Some(DocumentA11yActionData::Value(value.to_string())),
        _ => None,
    }
}

fn lower_node(
    node: &DocumentA11yNode,
    ids: &HashMap<DocumentA11yNodeId, AccessNodeId>,
) -> AccessNode {
    let mut access = AccessNode::new(role(node.role, node.state));
    if let DocumentA11yRole::Heading { level } = node.role
        && level > 0
    {
        access.set_level(level as usize);
    }
    if let Some(name) = &node.name {
        access.set_label(name.clone());
    }
    if let Some(value) = &node.value {
        access.set_value(value.clone());
    }
    if node.state.disabled {
        access.set_disabled();
    }
    if node.state.hidden {
        access.set_hidden();
    }
    if node.state.read_only {
        access.set_read_only();
    }
    if node.state.required {
        access.set_required();
    }
    // AccessKit 0.24 has no separate editable-state property. Text fields
    // retain their text role and read-only bit; multiline selects the distinct
    // multiline text role below. Do not invent a false editable value here.
    if let Some(selected) = node.state.selected {
        access.set_selected(selected);
    }
    if let Some(expanded) = node.state.expanded {
        access.set_expanded(expanded);
    }
    if let Some(toggled) = node.state.toggled {
        access.set_toggled(match toggled {
            DocumentA11yToggled::On => Toggled::True,
            DocumentA11yToggled::Off => Toggled::False,
            DocumentA11yToggled::Mixed => Toggled::Mixed,
        });
    } else if let Some(checked) = node.state.checked {
        access.set_toggled(if checked {
            Toggled::True
        } else {
            Toggled::False
        });
    }
    if let Some(live) = node.state.live {
        access.set_live(match live {
            DocumentA11yLive::Off => Live::Off,
            DocumentA11yLive::Polite => Live::Polite,
            DocumentA11yLive::Assertive => Live::Assertive,
        });
    }
    if let Some(orientation) = node.state.orientation {
        access.set_orientation(match orientation {
            DocumentA11yOrientation::Horizontal => Orientation::Horizontal,
            DocumentA11yOrientation::Vertical => Orientation::Vertical,
        });
    }
    if let Some(has_popup) = node.state.has_popup {
        access.set_has_popup(match has_popup {
            DocumentA11yHasPopup::Menu => HasPopup::Menu,
            DocumentA11yHasPopup::ListBox => HasPopup::Listbox,
            DocumentA11yHasPopup::Tree => HasPopup::Tree,
            DocumentA11yHasPopup::Grid => HasPopup::Grid,
            DocumentA11yHasPopup::Dialog => HasPopup::Dialog,
        });
    }
    if let Some(value) = node.numeric_value {
        access.set_numeric_value(value);
    }
    if let Some(value) = node.numeric_minimum {
        access.set_min_numeric_value(value);
    }
    if let Some(value) = node.numeric_maximum {
        access.set_max_numeric_value(value);
    }
    if let Some(bounds) = node.bounds {
        access.set_bounds(accesskit::Rect::new(
            bounds.x as f64,
            bounds.y as f64,
            (bounds.x + bounds.width) as f64,
            (bounds.y + bounds.height) as f64,
        ));
    }
    access.set_children(
        node.children
            .iter()
            .filter_map(|child| ids.get(child).copied())
            .collect::<Vec<_>>(),
    );
    for action in &node.actions {
        access.add_action(match action {
            DocumentA11yAction::Click => Action::Click,
            DocumentA11yAction::Focus => Action::Focus,
            DocumentA11yAction::SetValue => Action::SetValue,
            DocumentA11yAction::ScrollIntoView => Action::ScrollIntoView,
            DocumentA11yAction::Increment => Action::Increment,
            DocumentA11yAction::Decrement => Action::Decrement,
        });
    }
    access
}

fn role(role: DocumentA11yRole, state: DocumentA11yState) -> Role {
    match role {
        DocumentA11yRole::Window => Role::Window,
        DocumentA11yRole::Document | DocumentA11yRole::Article => Role::Document,
        DocumentA11yRole::Region | DocumentA11yRole::Group => Role::Group,
        DocumentA11yRole::Navigation => Role::Navigation,
        DocumentA11yRole::Main => Role::Main,
        DocumentA11yRole::Heading { .. } => Role::Heading,
        DocumentA11yRole::Paragraph => Role::Paragraph,
        DocumentA11yRole::StaticText => Role::TextRun,
        DocumentA11yRole::Link => Role::Link,
        DocumentA11yRole::Button => Role::Button,
        DocumentA11yRole::TextField if state.multiline => Role::MultilineTextInput,
        DocumentA11yRole::TextField => Role::TextInput,
        DocumentA11yRole::CheckBox => Role::CheckBox,
        DocumentA11yRole::RadioButton => Role::RadioButton,
        DocumentA11yRole::RadioGroup => Role::RadioGroup,
        DocumentA11yRole::Switch => Role::Switch,
        DocumentA11yRole::ComboBox => Role::ComboBox,
        DocumentA11yRole::List => Role::List,
        DocumentA11yRole::ListItem => Role::ListItem,
        DocumentA11yRole::ListBox => Role::ListBox,
        DocumentA11yRole::ListBoxOption => Role::ListBoxOption,
        DocumentA11yRole::Table => Role::Table,
        DocumentA11yRole::Row => Role::Row,
        DocumentA11yRole::Cell => Role::Cell,
        DocumentA11yRole::Image => Role::Image,
        DocumentA11yRole::Form => Role::Form,
        DocumentA11yRole::Dialog => Role::Dialog,
        DocumentA11yRole::Alert => Role::Alert,
        DocumentA11yRole::Menu => Role::Menu,
        DocumentA11yRole::MenuItem => Role::MenuItem,
        DocumentA11yRole::MenuItemCheckBox => Role::MenuItemCheckBox,
        DocumentA11yRole::MenuItemRadio => Role::MenuItemRadio,
        DocumentA11yRole::TabList => Role::TabList,
        DocumentA11yRole::Tab => Role::Tab,
        DocumentA11yRole::TabPanel => Role::TabPanel,
        DocumentA11yRole::Tree => Role::Tree,
        DocumentA11yRole::TreeItem => Role::TreeItem,
        DocumentA11yRole::Slider => Role::Slider,
        DocumentA11yRole::SpinButton => Role::SpinButton,
        DocumentA11yRole::Splitter => Role::Splitter,
        DocumentA11yRole::Toolbar => Role::Toolbar,
        DocumentA11yRole::ProgressIndicator => Role::ProgressIndicator,
        DocumentA11yRole::Label => Role::Label,
        DocumentA11yRole::Status => Role::Status,
        DocumentA11yRole::Log => Role::Log,
        DocumentA11yRole::Note => Role::Note,
        DocumentA11yRole::Unknown => Role::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use document_session_api::session_engine::{SessionEngine, SessionSpawnRequest};
    use genet_documents::LiverySessionEngine;
    use genet_host_api::ResourceFetcher;

    #[derive(Clone)]
    struct NoFetch;
    impl ResourceFetcher for NoFetch {
        fn fetch(&self, _: &str) -> Option<Vec<u8>> {
            None
        }
    }

    fn session(body: &str) -> Box<dyn DocumentSession<Scene>> {
        let engine = LiverySessionEngine::new(NoFetch);
        let request = SessionSpawnRequest::new("https://example.test/")
            .with_body(body)
            .with_viewport(400, 240);
        let mut session = engine.spawn(&request).expect("Livery session");
        let _ = session.frame(400, 240);
        session
    }

    fn action(action: Action, target_node: AccessNodeId) -> A11yActionRequest {
        A11yActionRequest {
            action,
            target_node,
            data: None,
        }
    }

    #[test]
    fn publication_keeps_the_document_root_and_a_named_heading_descendant() {
        let session = session("<h1>Ortet</h1><a href=\"notes.html\">Field notes</a>");
        let projection = session
            .accessibility_projection()
            .expect("article projection");
        let mut custody = Accessibility::default();
        let tree = custody.publish(Some(projection)).expect("publish tree");
        assert!(
            tree.nodes
                .iter()
                .any(|(_, node)| node.role() == Role::Document),
            "the session-provided document root is retained"
        );
        assert!(
            tree.nodes
                .iter()
                .any(|(_, node)| { node.role() == Role::Heading && node.label() == Some("Ortet") }),
            "the fixture heading is a semantic descendant, not a fabricated root name"
        );
    }

    #[test]
    fn lowering_preserves_checkbox_and_range_state_without_inventing_absence() {
        use document_session_api::{A11yCapability, DocumentA11ySupport};

        let root = DocumentA11yNodeId::new(1);
        let checkbox = DocumentA11yNodeId::new(2);
        let slider = DocumentA11yNodeId::new(3);
        let projection = DocumentA11yProjection::new(
            1,
            DocumentA11ySupport::new(A11yCapability::Full, std::iter::empty::<String>())
                .expect("full support"),
            root,
            vec![
                DocumentA11yNode {
                    id: root,
                    parent: None,
                    children: vec![checkbox, slider],
                    role: DocumentA11yRole::Document,
                    name: None,
                    value: None,
                    numeric_value: None,
                    numeric_minimum: None,
                    numeric_maximum: None,
                    bounds: None,
                    state: DocumentA11yState::default(),
                    actions: Vec::new(),
                },
                DocumentA11yNode {
                    id: checkbox,
                    parent: Some(root),
                    children: Vec::new(),
                    role: DocumentA11yRole::CheckBox,
                    name: Some("Subscribe".into()),
                    value: None,
                    numeric_value: None,
                    numeric_minimum: None,
                    numeric_maximum: None,
                    bounds: None,
                    state: DocumentA11yState {
                        checked: Some(true),
                        selected: Some(false),
                        live: Some(DocumentA11yLive::Polite),
                        ..DocumentA11yState::default()
                    },
                    actions: Vec::new(),
                },
                DocumentA11yNode {
                    id: slider,
                    parent: Some(root),
                    children: Vec::new(),
                    role: DocumentA11yRole::Slider,
                    name: Some("Volume".into()),
                    value: None,
                    numeric_value: Some(4.0),
                    numeric_minimum: Some(0.0),
                    numeric_maximum: Some(10.0),
                    bounds: None,
                    state: DocumentA11yState {
                        orientation: Some(DocumentA11yOrientation::Horizontal),
                        has_popup: Some(DocumentA11yHasPopup::Dialog),
                        ..DocumentA11yState::default()
                    },
                    actions: Vec::new(),
                },
            ],
        );
        let mut custody = Accessibility::default();
        let tree = custody.publish(Some(projection)).expect("publish tree");
        let checkbox = &tree
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Subscribe"))
            .expect("checkbox")
            .1;
        assert_eq!(checkbox.toggled(), Some(Toggled::True));
        assert_eq!(checkbox.is_selected(), Some(false));
        assert_eq!(checkbox.live(), Some(Live::Polite));
        let root = &tree.nodes.iter().find(|(_, node)| node.role() == Role::Document).expect("root").1;
        assert_eq!(root.toggled(), None, "absent source state stays absent");
        assert_eq!(root.is_selected(), None, "absence is not false");
        let slider = &tree
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Volume"))
            .expect("slider")
            .1;
        assert_eq!(slider.numeric_value(), Some(4.0));
        assert_eq!(slider.min_numeric_value(), Some(0.0));
        assert_eq!(slider.max_numeric_value(), Some(10.0));
        assert_eq!(slider.orientation(), Some(Orientation::Horizontal));
        assert_eq!(slider.has_popup(), Some(HasPopup::Dialog));
    }

    #[test]
    fn queued_action_from_replaced_session_is_rejected_even_when_local_id_repeats() {
        let first = session("<input aria-label=\"First\" value=\"a\">");
        let first_projection = first.accessibility_projection().expect("first projection");
        let local = first_projection
            .nodes()
            .iter()
            .find(|node| node.actions.contains(&DocumentA11yAction::Focus))
            .expect("focusable node")
            .id;
        let mut custody = Accessibility::default();
        custody
            .publish(Some(first_projection.clone()))
            .expect("publish A");
        let queued = action(Action::Focus, custody.host_id_for(local));

        let mut second = session("<input aria-label=\"Second\" value=\"b\">");
        // Livery currently salts local IDs by session, but the host contract
        // explicitly permits a replacement engine to reuse one. Model that
        // permitted case by publishing A's local projection as B's mapping.
        let second_projection = first_projection.clone();
        assert_eq!(
            second_projection
                .nodes()
                .iter()
                .find(|node| node.actions.contains(&DocumentA11yAction::Focus))
                .expect("focusable B")
                .id,
            local,
            "fixture reuses the engine-local id"
        );
        custody.replace_session();
        custody.publish(Some(second_projection)).expect("publish B");
        assert_eq!(custody.route(&mut *second, &queued), RoutedAction::Rejected);
    }

    #[test]
    fn absent_projection_withdraws_the_old_tree_and_its_actions() {
        let mut session = session("<a href=\"notes.html\">Field notes</a>");
        let projection = session.accessibility_projection().expect("projection");
        let local = projection
            .nodes()
            .iter()
            .find(|node| node.actions.contains(&DocumentA11yAction::Focus))
            .expect("focusable link")
            .id;
        let mut custody = Accessibility::default();
        custody.publish(Some(projection)).expect("publish content");
        let stale = action(Action::Focus, custody.host_id_for(local));
        custody.replace_session();
        let withdrawn = custody.publish(None).expect("withdraw old tree");
        assert_eq!(withdrawn.nodes.len(), 1, "only the empty document remains");
        assert_eq!(withdrawn.nodes[0].1.role(), Role::Document);
        assert_eq!(custody.route(&mut *session, &stale), RoutedAction::Rejected);
        assert_eq!(custody.publish(None), None, "do not churn empty trees");
    }

    #[test]
    fn stale_revision_and_unadvertised_action_are_rejected_but_current_focus_dispatches() {
        let mut session = session("<a href=\"notes.html\">Field notes</a>");
        let first = session
            .accessibility_projection()
            .expect("first projection");
        let local = first
            .nodes()
            .iter()
            .find(|node| node.actions.contains(&DocumentA11yAction::Focus))
            .expect("focusable node")
            .id;
        let mut custody = Accessibility::default();
        custody.publish(Some(first)).expect("publish first");
        let stale = action(Action::Focus, custody.host_id_for(local));
        let wrong_action = action(Action::ScrollIntoView, custody.host_id_for(local));
        assert_eq!(
            custody.route(&mut *session, &wrong_action),
            RoutedAction::Rejected
        );

        assert_eq!(
            custody.route(&mut *session, &stale),
            RoutedAction::Dispatched
        );
        let current = session
            .accessibility_projection()
            .expect("focus changes projection");
        custody
            .publish(Some(current.clone()))
            .expect("publish current revision");
        assert_eq!(custody.route(&mut *session, &stale), RoutedAction::Rejected);
        let fresh = action(Action::Focus, custody.host_id_for(local));
        assert_eq!(
            custody.route(&mut *session, &fresh),
            RoutedAction::Dispatched
        );
    }
}
