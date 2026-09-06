/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use std::hash::Hash;

#[cfg(feature = "accesskit")]
use accesskit::{
    Action, HasPopup, Live, Node as AccessNode, NodeId as AccessNodeId, Orientation, Rect, Role,
    Toggled, Tree, TreeId, TreeUpdate,
};
use document_session_api::{
    A11yCapability, DocumentA11yAction, DocumentA11yBounds, DocumentA11yHasPopup, DocumentA11yLive,
    DocumentA11yNode, DocumentA11yNodeId, DocumentA11yOrientation, DocumentA11yProjection,
    DocumentA11yRole, DocumentA11yState, DocumentA11ySupport, DocumentA11yToggled,
};
use genet_livery::LiveryLayout;
use layout_dom_api::{LayoutDom, LocalName, Namespace, NodeKind};

use crate::render::ScrollOffsets;

#[cfg(feature = "accesskit")]
fn access_id<D: LayoutDom>(dom: &D, node: D::NodeId) -> AccessNodeId {
    AccessNodeId(dom.opaque_id(node))
}

fn document_role<D: LayoutDom>(dom: &D, node: D::NodeId) -> DocumentA11yRole {
    if let Some(role) = dom.attribute(node, &Namespace::default(), &LocalName::from("role")) {
        match role.trim().to_ascii_lowercase().as_str() {
            "document" => return DocumentA11yRole::Document,
            "article" => return DocumentA11yRole::Article,
            "region" => return DocumentA11yRole::Region,
            "group" => return DocumentA11yRole::Group,
            "navigation" => return DocumentA11yRole::Navigation,
            "main" => return DocumentA11yRole::Main,
            "heading" => return DocumentA11yRole::Heading { level: 0 },
            "paragraph" => return DocumentA11yRole::Paragraph,
            "link" => return DocumentA11yRole::Link,
            "button" => return DocumentA11yRole::Button,
            "textbox" => return DocumentA11yRole::TextField,
            "checkbox" => return DocumentA11yRole::CheckBox,
            "radio" => return DocumentA11yRole::RadioButton,
            "radiogroup" => return DocumentA11yRole::RadioGroup,
            "switch" => return DocumentA11yRole::Switch,
            "combobox" => return DocumentA11yRole::ComboBox,
            "listbox" => return DocumentA11yRole::ListBox,
            "option" => return DocumentA11yRole::ListBoxOption,
            "list" => return DocumentA11yRole::List,
            "listitem" => return DocumentA11yRole::ListItem,
            "table" => return DocumentA11yRole::Table,
            "row" => return DocumentA11yRole::Row,
            "cell" | "gridcell" => return DocumentA11yRole::Cell,
            "image" | "img" => return DocumentA11yRole::Image,
            "form" => return DocumentA11yRole::Form,
            "dialog" => return DocumentA11yRole::Dialog,
            "alert" => return DocumentA11yRole::Alert,
            "menu" => return DocumentA11yRole::Menu,
            "menuitem" => return DocumentA11yRole::MenuItem,
            "menuitemcheckbox" => return DocumentA11yRole::MenuItemCheckBox,
            "menuitemradio" => return DocumentA11yRole::MenuItemRadio,
            "tablist" => return DocumentA11yRole::TabList,
            "tab" => return DocumentA11yRole::Tab,
            "tabpanel" => return DocumentA11yRole::TabPanel,
            "tree" => return DocumentA11yRole::Tree,
            "treeitem" => return DocumentA11yRole::TreeItem,
            "slider" => return DocumentA11yRole::Slider,
            "spinbutton" => return DocumentA11yRole::SpinButton,
            "status" => return DocumentA11yRole::Status,
            "log" => return DocumentA11yRole::Log,
            "note" => return DocumentA11yRole::Note,
            "separator" => return DocumentA11yRole::Splitter,
            "toolbar" => return DocumentA11yRole::Toolbar,
            "progressbar" => return DocumentA11yRole::ProgressIndicator,
            _ => {},
        }
    }
    match dom.kind(node) {
        NodeKind::Document => DocumentA11yRole::Window,
        NodeKind::Element => match dom.element_name(node).map(|name| name.local.as_ref()) {
            Some("html") => DocumentA11yRole::Document,
            Some("article") => DocumentA11yRole::Article,
            Some("nav") => DocumentA11yRole::Navigation,
            Some("main") => DocumentA11yRole::Main,
            Some("form") => DocumentA11yRole::Form,
            Some("dialog") => DocumentA11yRole::Dialog,
            Some("h1") => DocumentA11yRole::Heading { level: 1 },
            Some("h2") => DocumentA11yRole::Heading { level: 2 },
            Some("h3") => DocumentA11yRole::Heading { level: 3 },
            Some("h4") => DocumentA11yRole::Heading { level: 4 },
            Some("h5") => DocumentA11yRole::Heading { level: 5 },
            Some("h6") => DocumentA11yRole::Heading { level: 6 },
            Some("p") => DocumentA11yRole::Paragraph,
            Some("a")
                if dom
                    .attribute(node, &Namespace::default(), &LocalName::from("href"))
                    .is_some() =>
            {
                DocumentA11yRole::Link
            },
            Some("button") => DocumentA11yRole::Button,
            Some("input" | "textarea") => DocumentA11yRole::TextField,
            Some("label") => DocumentA11yRole::Label,
            Some("ul" | "ol") => DocumentA11yRole::List,
            Some("li") => DocumentA11yRole::ListItem,
            Some("table") => DocumentA11yRole::Table,
            Some("tr") => DocumentA11yRole::Row,
            Some("td" | "th") => DocumentA11yRole::Cell,
            Some("img") => DocumentA11yRole::Image,
            _ => DocumentA11yRole::Unknown,
        },
        _ => DocumentA11yRole::Unknown,
    }
}

fn direct_text<D: LayoutDom>(dom: &D, node: D::NodeId) -> String {
    dom.dom_children(node)
        .filter_map(|child| {
            (dom.kind(child) == NodeKind::Text)
                .then(|| dom.text(child))
                .flatten()
        })
        .collect()
}

/// Text contributed by a wrapping `<label>`, excluding the control it names.
/// This keeps a field's accessible name stable while its value changes.
fn label_text<D: LayoutDom>(dom: &D, node: D::NodeId) -> String {
    fn collect<D: LayoutDom>(dom: &D, node: D::NodeId, out: &mut String) {
        for child in dom.dom_children(node) {
            match dom.kind(child) {
                NodeKind::Text => out.push_str(dom.text(child).unwrap_or("")),
                NodeKind::Element => {
                    let tag = dom.element_name(child).map(|name| name.local.as_ref());
                    if matches!(tag, Some("button" | "input" | "select" | "textarea")) {
                        continue;
                    }
                    collect(dom, child, out);
                },
                _ => {},
            }
        }
    }
    let mut text = String::new();
    collect(dom, node, &mut text);
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn accessible_name<D: LayoutDom>(
    dom: &D,
    node: D::NodeId,
    label_context: Option<&str>,
) -> Option<String> {
    dom.attribute(node, &Namespace::default(), &LocalName::from("aria-label"))
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            let text = direct_text(dom, node);
            (!text.is_empty()).then_some(text)
        })
        .or_else(|| label_context.map(str::to_owned))
}

fn text_control_value<D: LayoutDom>(dom: &D, node: D::NodeId) -> Option<String> {
    let tag = dom.element_name(node).map(|name| name.local.as_ref())?;
    match tag {
        "textarea" => Some(descendant_text(dom, node)),
        "input" => {
            let input_type = dom
                .attribute(node, &Namespace::default(), &LocalName::from("type"))
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("text");
            matches!(
                input_type.to_ascii_lowercase().as_str(),
                "text" | "search" | "email" | "url" | "tel"
            )
            .then(|| {
                dom.attribute(node, &Namespace::default(), &LocalName::from("value"))
                    .unwrap_or("")
                    .to_owned()
            })
        },
        _ => None,
    }
}

fn descendant_text<D: LayoutDom>(dom: &D, node: D::NodeId) -> String {
    dom.dom_children(node)
        .map(|child| {
            if dom.kind(child) == NodeKind::Text {
                dom.text(child).unwrap_or("").to_owned()
            } else {
                descendant_text(dom, child)
            }
        })
        .collect()
}

/// One ARIA numeric attribute, when it parses. A malformed value is left unset
/// rather than projected as zero: a reader is better told nothing than told a
/// wrong position.
fn aria_number<D: LayoutDom>(dom: &D, node: D::NodeId, name: &str) -> Option<f64> {
    dom.attribute(node, &Namespace::default(), &LocalName::from(name))
        .and_then(|value| value.trim().parse::<f64>().ok())
}

/// One ARIA boolean attribute, when it is explicitly true. An absent or
/// malformed value is left unset rather than inferring an interaction state.
fn aria_true<D: LayoutDom>(dom: &D, node: D::NodeId, name: &str) -> bool {
    dom.attribute(node, &Namespace::default(), &LocalName::from(name))
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("true"))
}

/// An ARIA boolean, including an explicit false. Invalid and absent values
/// are left unset so the tree does not claim a state the DOM did not express.
fn aria_bool<D: LayoutDom>(dom: &D, node: D::NodeId, name: &str) -> Option<bool> {
    dom.attribute(node, &Namespace::default(), &LocalName::from(name))
        .and_then(|value| match value.trim().to_ascii_lowercase().as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        })
}

fn aria_toggled<D: LayoutDom>(dom: &D, node: D::NodeId, name: &str) -> Option<DocumentA11yToggled> {
    dom.attribute(node, &Namespace::default(), &LocalName::from(name))
        .and_then(|value| match value.trim().to_ascii_lowercase().as_str() {
            "true" => Some(DocumentA11yToggled::On),
            "false" => Some(DocumentA11yToggled::Off),
            "mixed" => Some(DocumentA11yToggled::Mixed),
            _ => None,
        })
}

fn aria_orientation<D: LayoutDom>(dom: &D, node: D::NodeId) -> Option<DocumentA11yOrientation> {
    dom.attribute(
        node,
        &Namespace::default(),
        &LocalName::from("aria-orientation"),
    )
    .and_then(|value| match value.trim().to_ascii_lowercase().as_str() {
        "horizontal" => Some(DocumentA11yOrientation::Horizontal),
        "vertical" => Some(DocumentA11yOrientation::Vertical),
        _ => None,
    })
}

fn aria_has_popup<D: LayoutDom>(dom: &D, node: D::NodeId) -> Option<DocumentA11yHasPopup> {
    dom.attribute(
        node,
        &Namespace::default(),
        &LocalName::from("aria-haspopup"),
    )
    .and_then(|value| match value.trim().to_ascii_lowercase().as_str() {
        "true" | "menu" => Some(DocumentA11yHasPopup::Menu),
        "listbox" => Some(DocumentA11yHasPopup::ListBox),
        "tree" => Some(DocumentA11yHasPopup::Tree),
        "grid" => Some(DocumentA11yHasPopup::Grid),
        "dialog" => Some(DocumentA11yHasPopup::Dialog),
        "false" | "none" | "" => None,
        _ => None,
    })
}

fn is_disabled<D: LayoutDom>(dom: &D, node: D::NodeId) -> bool {
    aria_true(dom, node, "aria-disabled")
        || dom
            .attribute(node, &Namespace::default(), &LocalName::from("disabled"))
            .is_some()
}

fn aria_live<D: LayoutDom>(dom: &D, node: D::NodeId) -> Option<DocumentA11yLive> {
    dom.attribute(node, &Namespace::default(), &LocalName::from("aria-live"))
        .and_then(|value| match value.trim().to_ascii_lowercase().as_str() {
            "off" => Some(DocumentA11yLive::Off),
            "polite" => Some(DocumentA11yLive::Polite),
            "assertive" => Some(DocumentA11yLive::Assertive),
            _ => None,
        })
}

fn is_content_editable<D: LayoutDom>(dom: &D, node: D::NodeId) -> bool {
    dom.attribute(
        node,
        &Namespace::default(),
        &LocalName::from("contenteditable"),
    )
    .is_some_and(|value| {
        let value = value.trim();
        value.is_empty()
            || value.eq_ignore_ascii_case("true")
            || value.eq_ignore_ascii_case("plaintext-only")
    })
}

fn has_tabindex<D: LayoutDom>(dom: &D, node: D::NodeId) -> bool {
    dom.attribute(node, &Namespace::default(), &LocalName::from("tabindex"))
        .is_some_and(|value| value.trim().parse::<i32>().is_ok())
}

/// The accumulated scroll owned by element ancestors. A node's own scroll
/// offset moves its descendants, not its own retained border box.
fn ancestor_scroll<D>(
    dom: &D,
    node: D::NodeId,
    scroll_offsets: &ScrollOffsets<D::NodeId>,
) -> (f32, f32)
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    let mut total = (0.0, 0.0);
    let mut current = dom.parent(node);
    while let Some(parent) = current {
        if let Some((x, y)) = scroll_offsets.get(&parent) {
            total.0 += x;
            total.1 += y;
        }
        current = dom.parent(parent);
    }
    total
}

/// An active nested scrollport moves its descendant bounds. Pelt can route a
/// retained `ScrollIntoView` request back to Livery, but it does not yet own
/// the corresponding nested-pointer route. Keep descendants semantic and
/// focusable, advertise only the reveal action, and withhold Click.
fn has_active_scrolled_ancestor<D>(
    dom: &D,
    node: D::NodeId,
    scroll_offsets: &ScrollOffsets<D::NodeId>,
) -> bool
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    let mut current = dom.parent(node);
    while let Some(parent) = current {
        if scroll_offsets
            .get(&parent)
            .is_some_and(|&(x, y)| x != 0.0 || y != 0.0)
        {
            return true;
        }
        current = dom.parent(parent);
    }
    false
}

fn is_native_control<D: LayoutDom>(dom: &D, node: D::NodeId) -> bool {
    matches!(
        dom.element_name(node).map(|name| name.local.as_ref()),
        Some("button" | "input" | "select" | "textarea")
    ) || (dom.element_name(node).map(|name| name.local.as_ref()) == Some("a")
        && dom
            .attribute(node, &Namespace::default(), &LocalName::from("href"))
            .is_some())
}

/// Project a Livery/Buckram document into an AccessKit tree.
#[cfg(feature = "accesskit")]
pub fn accesskit_tree<D>(
    dom: &D,
    fragments: &LiveryLayout<D::NodeId>,
    focus: Option<D::NodeId>,
) -> TreeUpdate
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    accesskit_tree_with_optional_scroll(dom, fragments, focus, None)
}

/// Project a retained document after Livery has applied nested element scroll.
///
/// Bounds move with each scrolled ancestor. Enabled descendants of an active
/// nested scrollport advertise `ScrollIntoView`, while Click remains withheld
/// until Pelt owns the matching refreshed pointer-routing semantics.
#[cfg(feature = "accesskit")]
pub fn accesskit_tree_with_scroll<D>(
    dom: &D,
    fragments: &LiveryLayout<D::NodeId>,
    focus: Option<D::NodeId>,
    scroll_offsets: &ScrollOffsets<D::NodeId>,
) -> TreeUpdate
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    accesskit_tree_with_optional_scroll(dom, fragments, focus, Some(scroll_offsets))
}

#[cfg(feature = "accesskit")]
fn accesskit_tree_with_optional_scroll<D>(
    dom: &D,
    fragments: &LiveryLayout<D::NodeId>,
    focus: Option<D::NodeId>,
    scroll_offsets: Option<&ScrollOffsets<D::NodeId>>,
) -> TreeUpdate
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    let projection =
        document_a11y_projection_with_optional_scroll(dom, fragments, focus, 0, scroll_offsets);
    lower_accesskit_tree(dom, projection)
}

#[cfg(feature = "accesskit")]
fn accesskit_role(role: DocumentA11yRole) -> Role {
    match role {
        DocumentA11yRole::Window => Role::Window,
        DocumentA11yRole::Document => Role::Document,
        DocumentA11yRole::Article => Role::Article,
        DocumentA11yRole::Region => Role::Region,
        DocumentA11yRole::Group => Role::Group,
        DocumentA11yRole::Navigation => Role::Navigation,
        DocumentA11yRole::Main => Role::Main,
        DocumentA11yRole::Heading { .. } => Role::Heading,
        DocumentA11yRole::Paragraph => Role::Paragraph,
        // AccessKit represents ordinary text through its parent's label and
        DocumentA11yRole::StaticText => Role::TextRun,
        DocumentA11yRole::Link => Role::Link,
        DocumentA11yRole::Button => Role::Button,
        DocumentA11yRole::TextField => Role::TextInput,
        DocumentA11yRole::CheckBox => Role::CheckBox,
        DocumentA11yRole::RadioButton => Role::RadioButton,
        DocumentA11yRole::ComboBox => Role::ComboBox,
        DocumentA11yRole::List => Role::List,
        DocumentA11yRole::ListItem => Role::ListItem,
        DocumentA11yRole::Table => Role::Table,
        DocumentA11yRole::Row => Role::Row,
        DocumentA11yRole::Cell => Role::Cell,
        DocumentA11yRole::Image => Role::Image,
        DocumentA11yRole::Form => Role::Form,
        DocumentA11yRole::Dialog => Role::Dialog,
        DocumentA11yRole::Alert => Role::Alert,
        DocumentA11yRole::Menu => Role::Menu,
        DocumentA11yRole::MenuItem => Role::MenuItem,
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
        DocumentA11yRole::RadioGroup => Role::RadioGroup,
        DocumentA11yRole::Switch => Role::Switch,
        DocumentA11yRole::ListBox => Role::ListBox,
        DocumentA11yRole::ListBoxOption => Role::ListBoxOption,
        DocumentA11yRole::MenuItemCheckBox => Role::MenuItemCheckBox,
        DocumentA11yRole::MenuItemRadio => Role::MenuItemRadio,
        DocumentA11yRole::Unknown => Role::GenericContainer,
    }
}

#[cfg(feature = "accesskit")]
fn lower_accesskit_tree<D: LayoutDom>(dom: &D, projection: DocumentA11yProjection) -> TreeUpdate
where
    D::NodeId: Copy + Eq + Hash,
{
    let root = dom.document();
    let root_id = access_id(dom, root);
    let nodes = projection
        .nodes()
        .iter()
        .map(|node| {
            let mut access = AccessNode::new(accesskit_role(node.role));
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
                access.set_bounds(Rect::new(
                    bounds.x as f64,
                    bounds.y as f64,
                    (bounds.x + bounds.width) as f64,
                    (bounds.y + bounds.height) as f64,
                ));
            }
            access.set_children(
                node.children
                    .iter()
                    .map(|id| AccessNodeId(id.get()))
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
            (AccessNodeId(node.id.get()), access)
        })
        .collect();
    let focus = projection
        .nodes()
        .iter()
        .find(|node| node.state.focused)
        .map_or(root_id, |node| AccessNodeId(node.id.get()));
    TreeUpdate {
        nodes,
        tree: Some(Tree::new(root_id)),
        tree_id: TreeId::ROOT,
        focus,
    }
}

fn neutral_node_id<D: LayoutDom>(dom: &D, node: D::NodeId) -> DocumentA11yNodeId {
    DocumentA11yNodeId::new(dom.opaque_id(node))
}

fn neutral_heading_level<D: LayoutDom>(dom: &D, node: D::NodeId) -> u8 {
    aria_number(dom, node, "aria-level")
        .and_then(|level| u8::try_from(level as i64).ok())
        .filter(|level| *level > 0)
        .unwrap_or(0)
}

fn neutral_state<D: LayoutDom>(
    dom: &D,
    node: D::NodeId,
    focus: Option<D::NodeId>,
) -> DocumentA11yState {
    let read_only = aria_true(dom, node, "aria-readonly")
        || dom
            .attribute(node, &Namespace::default(), &LocalName::from("readonly"))
            .is_some();
    let disabled = is_disabled(dom, node);
    let editable = !disabled
        && !read_only
        && (is_content_editable(dom, node) || text_control_value(dom, node).is_some());
    let multiline = is_content_editable(dom, node)
        || dom
            .element_name(node)
            .is_some_and(|name| name.local.as_ref() == "textarea");
    let toggled =
        aria_toggled(dom, node, "aria-checked").or_else(|| aria_toggled(dom, node, "aria-pressed"));
    let live = aria_live(dom, node);
    let orientation = aria_orientation(dom, node);
    let has_popup = aria_has_popup(dom, node);
    DocumentA11yState {
        disabled,
        selected: aria_bool(dom, node, "aria-selected"),
        expanded: aria_bool(dom, node, "aria-expanded"),
        checked: aria_bool(dom, node, "aria-checked"),
        toggled,
        focused: focus.is_some_and(|focused| focused == node),
        editable,
        multiline,
        read_only,
        required: aria_true(dom, node, "aria-required")
            || dom
                .attribute(node, &Namespace::default(), &LocalName::from("required"))
                .is_some(),
        live,
        orientation,
        has_popup,
        ..DocumentA11yState::default()
    }
}

fn neutral_actions<D: LayoutDom>(
    dom: &D,
    node: D::NodeId,
    role: DocumentA11yRole,
    state: DocumentA11yState,
    scroll_offsets: Option<&ScrollOffsets<D::NodeId>>,
) -> Vec<DocumentA11yAction> {
    let semantic_control = is_native_control(dom, node)
        || matches!(
            role,
            DocumentA11yRole::Button
                | DocumentA11yRole::CheckBox
                | DocumentA11yRole::RadioButton
                | DocumentA11yRole::Switch
                | DocumentA11yRole::ComboBox
                | DocumentA11yRole::Tab
                | DocumentA11yRole::MenuItem
                | DocumentA11yRole::MenuItemCheckBox
                | DocumentA11yRole::MenuItemRadio
                | DocumentA11yRole::Slider
                | DocumentA11yRole::SpinButton
                | DocumentA11yRole::TextField
                | DocumentA11yRole::Link
        );
    let focusable = semantic_control || has_tabindex(dom, node) || is_content_editable(dom, node);
    let blocked =
        scroll_offsets.is_some_and(|offsets| has_active_scrolled_ancestor(dom, node, offsets));
    let mut actions = Vec::new();
    if !state.disabled && blocked {
        actions.push(DocumentA11yAction::ScrollIntoView);
    }
    if !state.disabled && !blocked && (semantic_control || is_content_editable(dom, node)) {
        actions.push(DocumentA11yAction::Click);
    }
    if !state.disabled && focusable {
        actions.push(DocumentA11yAction::Focus);
    }
    if text_control_value(dom, node).is_some() && !state.disabled && !state.read_only && !blocked {
        actions.push(DocumentA11yAction::SetValue);
    }
    actions
}

fn projection_walk<D>(
    dom: &D,
    fragments: &LiveryLayout<D::NodeId>,
    scroll_offsets: Option<&ScrollOffsets<D::NodeId>>,
    node: D::NodeId,
    parent: Option<DocumentA11yNodeId>,
    label_context: Option<&str>,
    focus: Option<D::NodeId>,
    out: &mut Vec<DocumentA11yNode>,
) -> Vec<DocumentA11yNodeId>
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    if aria_true(dom, node, "aria-hidden") {
        return Vec::new();
    }
    let is_label = dom
        .element_name(node)
        .is_some_and(|name| name.local.as_ref() == "label");
    let own_label = is_label.then(|| label_text(dom, node));
    let child_label = own_label
        .as_deref()
        .filter(|label| !label.is_empty())
        .or(label_context);
    let projected = dom.kind(node) == NodeKind::Document || fragments.get(node).is_some();
    let id = projected.then(|| neutral_node_id(dom, node));
    let child_parent = id.or(parent);
    let children = dom
        .dom_children(node)
        .filter(|child| dom.kind(*child) == NodeKind::Element)
        .flat_map(|child| {
            projection_walk(
                dom,
                fragments,
                scroll_offsets,
                child,
                child_parent,
                child_label,
                focus,
                out,
            )
        })
        .collect::<Vec<_>>();
    let Some(id) = id else {
        return children;
    };

    let role = match document_role(dom, node) {
        DocumentA11yRole::Heading { level: 0 } => DocumentA11yRole::Heading {
            level: neutral_heading_level(dom, node),
        },
        role => role,
    };
    let state = neutral_state(dom, node, focus);
    let bounds = fragments.get(node).map(|fragment| {
        let (scroll_x, scroll_y) = scroll_offsets
            .map(|offsets| ancestor_scroll(dom, node, offsets))
            .unwrap_or_default();
        DocumentA11yBounds {
            x: fragment.x - scroll_x,
            y: fragment.y - scroll_y,
            width: fragment.width,
            height: fragment.height,
        }
    });
    let name = accessible_name(dom, node, label_context);
    let value = text_control_value(dom, node);
    let actions = neutral_actions(dom, node, role, state, scroll_offsets);
    let numeric_value = aria_number(dom, node, "aria-valuenow");
    let numeric_minimum = aria_number(dom, node, "aria-valuemin");
    let numeric_maximum = aria_number(dom, node, "aria-valuemax");
    out.push(DocumentA11yNode {
        id,
        parent,
        children: children.clone(),
        role,
        name,
        value,
        numeric_value,
        numeric_minimum,
        numeric_maximum,
        bounds,
        state,
        actions,
    });
    vec![id]
}

/// Project the retained Livery/Buckram document into the renderer-neutral
/// accessibility contract. `revision` scopes local identities and action
/// requests; compatibility AccessKit wrappers use zero.
pub fn document_a11y_projection<D>(
    dom: &D,
    fragments: &LiveryLayout<D::NodeId>,
    focus: Option<D::NodeId>,
    revision: u64,
) -> DocumentA11yProjection
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    document_a11y_projection_with_optional_scroll(dom, fragments, focus, revision, None)
}

/// Project a retained document after Livery has applied nested element scroll.
pub fn document_a11y_projection_with_scroll<D>(
    dom: &D,
    fragments: &LiveryLayout<D::NodeId>,
    focus: Option<D::NodeId>,
    revision: u64,
    scroll_offsets: &ScrollOffsets<D::NodeId>,
) -> DocumentA11yProjection
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    document_a11y_projection_with_optional_scroll(
        dom,
        fragments,
        focus,
        revision,
        Some(scroll_offsets),
    )
}

fn document_a11y_projection_with_optional_scroll<D>(
    dom: &D,
    fragments: &LiveryLayout<D::NodeId>,
    focus: Option<D::NodeId>,
    revision: u64,
    scroll_offsets: Option<&ScrollOffsets<D::NodeId>>,
) -> DocumentA11yProjection
where
    D: LayoutDom,
    D::NodeId: Copy + Eq + Hash,
{
    let root = dom.document();
    let root_id = neutral_node_id(dom, root);
    let mut nodes = Vec::new();
    projection_walk(
        dom,
        fragments,
        scroll_offsets,
        root,
        None,
        None,
        focus,
        &mut nodes,
    );
    let support = DocumentA11ySupport::new(
        A11yCapability::Partial,
        ["Laid-out element roles and states are exposed; custom leaves, standalone text nodes, and the complete accessible-name algorithm are not yet included."],
    )
    .expect("partial projections carry an explicit limitation");
    DocumentA11yProjection::new(revision, support, root_id, nodes)
}

#[cfg(all(test, feature = "accesskit"))]
mod tests {
    use accesskit::{Action, HasPopup, Live, Node as AccessNode, Orientation, Role, Toggled};
    use document_session_api::{DocumentA11yRole, DocumentA11yToggled};
    use genet_scripted_dom::ScriptedDom;
    use layout_dom_api::{LayoutDom, LayoutDomMut, NodeKind};

    use super::{accesskit_tree, accesskit_tree_with_scroll, document_a11y_projection};
    use crate::{ScrollOffsets, fragments_from_scripted_dom};

    const SHEET: &[&str] = &["div { display: block; }"];

    fn nodes_for(html: &str) -> Vec<AccessNode> {
        let mut dom = ScriptedDom::new();
        let root = dom.document();
        dom.set_inner_html(root, html);
        let fragments = fragments_from_scripted_dom(&dom, SHEET, 400, 300).expect("layout");
        accesskit_tree(&dom, &fragments, None)
            .nodes
            .into_iter()
            .map(|(_, node)| node)
            .collect()
    }

    fn with_role(html: &str, role: Role) -> AccessNode {
        nodes_for(html)
            .into_iter()
            .find(|node| node.role() == role)
            .unwrap_or_else(|| panic!("no node projected with role {role:?}"))
    }

    fn projection_for(html: &str) -> document_session_api::DocumentA11yProjection {
        let mut dom = ScriptedDom::new();
        let root = dom.document();
        dom.set_inner_html(root, html);
        let fragments = fragments_from_scripted_dom(&dom, SHEET, 400, 300).expect("layout");
        document_a11y_projection(&dom, &fragments, None, 7)
    }

    #[test]
    fn neutral_projection_preserves_wrapping_label_names() {
        let projection = projection_for(
            "<label style=\"display:block\">Board revision <input value=\"3\"></label>",
        );
        assert!(
            projection
                .nodes()
                .iter()
                .any(|node| node.role == DocumentA11yRole::Label),
            "the neutral projection preserves the native label role"
        );
        let field = projection
            .nodes()
            .iter()
            .find(|node| node.role == DocumentA11yRole::TextField)
            .expect("wrapped text field");
        assert_eq!(field.name.as_deref(), Some("Board revision"));
        assert_eq!(field.value.as_deref(), Some("3"));
        assert_eq!(projection.revision(), 7);
    }

    #[test]
    fn neutral_projection_keeps_semantic_roles_and_states() {
        let projection = projection_for(
            "<div role=\"log\" aria-live=\"polite\">Saved</div>\
             <div role=\"note\">Note</div>\
             <div role=\"list\"><div role=\"listitem\">Entry</div></div>\
             <div role=\"spinbutton\" aria-valuenow=\"2\" aria-checked=\"mixed\">Count</div>",
        );
        assert!(
            projection
                .nodes()
                .iter()
                .any(|node| node.role == DocumentA11yRole::Log)
        );
        assert!(
            projection
                .nodes()
                .iter()
                .any(|node| node.role == DocumentA11yRole::Note)
        );
        assert!(
            projection
                .nodes()
                .iter()
                .any(|node| node.role == DocumentA11yRole::List)
        );
        assert!(
            projection
                .nodes()
                .iter()
                .any(|node| node.role == DocumentA11yRole::ListItem)
        );
        let spin = projection
            .nodes()
            .iter()
            .find(|node| node.role == DocumentA11yRole::SpinButton)
            .expect("spinbutton");
        assert_eq!(spin.numeric_value, Some(2.0));
        assert_eq!(spin.state.toggled, Some(DocumentA11yToggled::Mixed));
    }

    #[test]
    fn a_refusal_projects_as_an_alert() {
        let nodes = nodes_for("<div role=\"alert\">Cannot flash this board</div>");
        assert!(
            nodes.iter().any(|node| node.role() == Role::Alert),
            "role=alert must reach the reader as an alert, not a generic container",
        );
    }

    #[test]
    fn a_progress_bar_carries_its_value() {
        let bar = with_role(
            "<div role=\"progressbar\" aria-valuenow=\"50\" aria-valuemin=\"0\" aria-valuemax=\"100\"></div>",
            Role::ProgressIndicator,
        );
        assert_eq!(bar.numeric_value(), Some(50.0));
        assert_eq!(bar.min_numeric_value(), Some(0.0));
        assert_eq!(bar.max_numeric_value(), Some(100.0));
    }

    #[test]
    fn a_malformed_value_is_left_unset() {
        let bar = with_role(
            "<div role=\"progressbar\" aria-valuenow=\"soon\"></div>",
            Role::ProgressIndicator,
        );
        assert_eq!(bar.numeric_value(), None);
    }

    #[test]
    fn a_read_only_document_keeps_its_document_semantics() {
        let document = with_role(
            "<div role=\"document\" aria-readonly=\"true\">Read-only notes</div>",
            Role::Document,
        );
        assert!(document.is_read_only());
    }

    #[test]
    fn landmarks_and_status_reach_the_reader() {
        let nodes = nodes_for(
            "<section role=\"region\" aria-label=\"Related notes\"></section>\
             <div role=\"status\">Synced</div>",
        );
        assert!(nodes.iter().any(|node| node.role() == Role::Region));
        assert!(nodes.iter().any(|node| node.role() == Role::Status));
    }

    #[test]
    fn controls_advertise_click_and_focus_separately() {
        let button = with_role("<button>Open</button>", Role::Button);
        assert!(button.supports_action(Action::Click));
        assert!(button.supports_action(Action::Focus));

        let focusable = nodes_for("<div tabindex=\"0\">Focus only</div>")
            .into_iter()
            .find(|node| node.role() == Role::GenericContainer)
            .expect("focusable div");
        assert!(focusable.supports_action(Action::Focus));
        assert!(!focusable.supports_action(Action::Click));
    }

    #[test]
    fn native_links_project_link_semantics_only_when_navigable() {
        let nodes = nodes_for(
            "<a href=\"next.html\" style=\"display:block\">Navigate</a>\
             <a style=\"display:block\">Named anchor</a>",
        );
        let link = nodes
            .iter()
            .find(|node| node.label() == Some("Navigate"))
            .expect("native link");
        assert_eq!(link.role(), Role::Link);
        assert!(link.supports_action(Action::Click));
        assert!(link.supports_action(Action::Focus));

        let anchor = nodes
            .iter()
            .find(|node| node.label() == Some("Named anchor"))
            .expect("anchor without href");
        assert_eq!(anchor.role(), Role::GenericContainer);
        assert!(!anchor.supports_action(Action::Click));
        assert!(!anchor.supports_action(Action::Focus));
    }

    #[test]
    fn contenteditable_nodes_are_clickable_and_focusable() {
        let editor = with_role(
            "<div role=\"textbox\" contenteditable>Notes</div>",
            Role::TextInput,
        );
        assert!(editor.supports_action(Action::Click));
        assert!(editor.supports_action(Action::Focus));
    }

    #[test]
    fn writable_text_controls_project_values_and_set_value() {
        let nodes = nodes_for(
            "<input value=\"find me\"><input type=\"search\" value=\"search me\">
             <textarea>write me</textarea>",
        );
        let values: Vec<_> = nodes.iter().filter_map(AccessNode::value).collect();
        assert!(values.contains(&"find me"));
        assert!(values.contains(&"search me"));
        assert!(values.contains(&"write me"));
        assert_eq!(values.len(), 3);
        assert_eq!(
            nodes
                .iter()
                .filter(|node| node.supports_action(Action::SetValue))
                .count(),
            3
        );
    }

    #[test]
    fn readonly_disabled_and_non_text_inputs_do_not_advertise_set_value() {
        let nodes = nodes_for(
            "<input readonly value=\"read only\"><input disabled value=\"disabled\">
             <input type=\"number\" value=\"42\"><input type=\"checkbox\" value=\"yes\">
             <textarea aria-readonly=\"true\">locked</textarea>",
        );
        assert!(
            nodes
                .iter()
                .all(|node| !node.supports_action(Action::SetValue))
        );
        assert_eq!(
            nodes.iter().filter(|node| node.value().is_some()).count(),
            3
        );
        assert_eq!(nodes.iter().filter(|node| node.is_read_only()).count(), 2);
    }

    #[test]
    fn nested_scroll_withholds_text_control_set_value() {
        let mut dom = ScriptedDom::new();
        let root = dom.document();
        dom.set_inner_html(root, "<div><input value=\"nested\"></div>");
        let container = dom
            .dom_children(root)
            .find(|node| dom.kind(*node) == NodeKind::Element)
            .expect("scroll container");
        let input = dom
            .dom_children(container)
            .find(|node| dom.kind(*node) == NodeKind::Element)
            .expect("input");
        let fragments = fragments_from_scripted_dom(&dom, SHEET, 400, 300).expect("layout");
        let mut offsets = ScrollOffsets::new();
        offsets.insert(container, (0.0, 24.0));
        let tree = accesskit_tree_with_scroll(&dom, &fragments, Some(input), &offsets);
        let node = tree
            .nodes
            .iter()
            .find(|(id, _)| *id == super::access_id(&dom, input))
            .map(|(_, node)| node)
            .expect("scrolled input");
        assert_eq!(node.value(), Some("nested"));
        assert!(!node.supports_action(Action::SetValue));
        assert!(node.supports_action(Action::ScrollIntoView));
    }

    #[test]
    fn disabled_nested_controls_do_not_advertise_scroll_into_view() {
        let mut dom = ScriptedDom::new();
        let root = dom.document();
        dom.set_inner_html(
            root,
            "<div><button disabled>Unavailable</button><div role=\"link\" aria-disabled=\"true\" tabindex=\"0\">Also unavailable</div></div>",
        );
        let container = dom
            .dom_children(root)
            .find(|node| dom.kind(*node) == NodeKind::Element)
            .expect("scroll container");
        let fragments = fragments_from_scripted_dom(&dom, SHEET, 400, 300).expect("layout");
        let mut offsets = ScrollOffsets::new();
        offsets.insert(container, (0.0, 24.0));
        let tree = accesskit_tree_with_scroll(&dom, &fragments, None, &offsets);

        for label in ["Unavailable", "Also unavailable"] {
            let node = tree
                .nodes
                .iter()
                .find(|(_, node)| node.label() == Some(label))
                .map(|(_, node)| node)
                .expect("disabled nested control");
            assert!(node.is_disabled());
            assert!(!node.supports_action(Action::Click));
            assert!(!node.supports_action(Action::Focus));
            assert!(!node.supports_action(Action::ScrollIntoView));
            assert!(!node.supports_action(Action::SetValue));
        }
    }

    #[test]
    fn hidden_controls_do_not_enter_the_tree() {
        let nodes = nodes_for(
            "<button style=\"display: none\">Paint hidden</button>\
             <button aria-hidden=\"true\">ARIA hidden</button>\
             <button>Visible</button>",
        );
        let labels: Vec<_> = nodes.iter().filter_map(AccessNode::label).collect();
        assert!(!labels.contains(&"Paint hidden"));
        assert!(!labels.contains(&"ARIA hidden"));
        assert!(labels.contains(&"Visible"));
    }

    #[test]
    fn live_regions_and_disabled_controls_keep_their_state() {
        let nodes = nodes_for(
            "<div role=\"status\" aria-live=\"polite\">Saved</div>\
             <button disabled>Unavailable</button>\
             <div role=\"button\" aria-disabled=\"true\">Also unavailable</div>",
        );
        let status = nodes
            .iter()
            .find(|node| node.role() == Role::Status)
            .expect("status");
        assert_eq!(status.live(), Some(Live::Polite));
        for disabled in nodes.iter().filter(|node| node.is_disabled()) {
            assert!(!disabled.supports_action(Action::Click));
            assert!(!disabled.supports_action(Action::Focus));
        }
        assert_eq!(nodes.iter().filter(|node| node.is_disabled()).count(), 2);
    }

    #[test]
    fn aria_widget_roles_and_states_reach_accesskit() {
        let nodes = nodes_for(
            "<div role=\"menu\" aria-label=\"Actions\">\
                <div role=\"menuitemradio\" aria-label=\"Compact\" aria-checked=\"true\" aria-selected=\"true\">Compact</div>\
                <div role=\"menuitemcheckbox\" aria-label=\"Details\" aria-checked=\"mixed\">Details</div>\
            </div>\
            <div role=\"separator\" aria-orientation=\"vertical\"></div>\
            <button aria-expanded=\"false\" aria-haspopup=\"dialog\">Details</button>",
        );

        let menu = nodes
            .iter()
            .find(|node| node.role() == Role::Menu)
            .expect("menu role");
        assert_eq!(menu.label(), Some("Actions"));

        let radio = nodes
            .iter()
            .find(|node| node.role() == Role::MenuItemRadio)
            .expect("menuitemradio role");
        assert_eq!(radio.toggled(), Some(Toggled::True));
        assert_eq!(radio.is_selected(), Some(true));
        assert!(radio.supports_action(Action::Click));

        let mixed = nodes
            .iter()
            .find(|node| node.label() == Some("Details") && node.role() == Role::MenuItemCheckBox)
            .expect("menuitemcheckbox role");
        assert_eq!(mixed.toggled(), Some(Toggled::Mixed));

        let separator = nodes
            .iter()
            .find(|node| node.role() == Role::Splitter)
            .expect("separator role");
        assert_eq!(separator.orientation(), Some(Orientation::Vertical));

        let trigger = nodes
            .iter()
            .find(|node| node.role() == Role::Button && node.label() == Some("Details"))
            .expect("popup trigger");
        assert_eq!(trigger.is_expanded(), Some(false));
        assert_eq!(trigger.has_popup(), Some(HasPopup::Dialog));
    }

    #[test]
    fn aria_pressed_and_bounds_are_projected_without_inference() {
        let nodes = nodes_for(
            "<button style=\"display:block;width:80px;height:20px\" aria-pressed=\"mixed\">Filter</button>\
             <div role=\"button\" aria-expanded=\"maybe\" aria-haspopup=\"unknown\">Invalid</div>",
        );
        let filter = nodes
            .iter()
            .find(|node| node.label() == Some("Filter"))
            .expect("filter button");
        assert_eq!(filter.toggled(), Some(Toggled::Mixed));
        let bounds = filter.bounds().expect("laid out button bounds");
        assert_eq!(bounds.x1 - bounds.x0, 80.0);
        assert_eq!(bounds.y1 - bounds.y0, 20.0);

        let invalid = nodes
            .iter()
            .find(|node| node.label() == Some("Invalid"))
            .expect("invalid state button");
        assert_eq!(invalid.is_expanded(), None);
        assert_eq!(invalid.has_popup(), None);
    }

    #[test]
    fn nested_scroll_offsets_bounds_and_withholds_descendant_click() {
        let mut dom = ScriptedDom::new();
        let root = dom.document();
        dom.set_inner_html(
            root,
            "<div><div role=\"link\" tabindex=\"0\" style=\"display:block;width:80px;height:20px\">Scrolled action</div></div>",
        );
        let container = dom
            .dom_children(root)
            .find(|node| dom.kind(*node) == NodeKind::Element)
            .expect("scroll container");
        let fragments = fragments_from_scripted_dom(&dom, SHEET, 400, 300).expect("layout");
        let before = accesskit_tree(&dom, &fragments, None);
        let before_link = before
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Scrolled action"))
            .map(|(_, node)| node)
            .expect("unscrolled link");
        let before_bounds = before_link.bounds().expect("unscrolled bounds");

        let mut offsets = ScrollOffsets::new();
        offsets.insert(container, (0.0, 24.0));
        let after = accesskit_tree_with_scroll(&dom, &fragments, None, &offsets);
        let after_link = after
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Scrolled action"))
            .map(|(_, node)| node)
            .expect("scrolled link");
        let after_bounds = after_link.bounds().expect("scrolled bounds");

        assert_eq!(after_bounds.x0, before_bounds.x0);
        assert_eq!(after_bounds.y0, before_bounds.y0 - 24.0);
        assert!(after_link.supports_action(Action::Focus));
        assert!(
            !after_link.supports_action(Action::Click),
            "an active nested scrollport cannot advertise a stale Click target"
        );
        assert!(after_link.supports_action(Action::ScrollIntoView));
    }
}
