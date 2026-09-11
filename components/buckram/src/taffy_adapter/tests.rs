// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use taffy::{
    geometry::Rect,
    prelude::{Dimension, Display, Position, Style, fr, length, line},
    style::{AlignItems, JustifyContent},
};

use super::*;

fn available(width: f32, height: f32) -> AlgorithmSize<AlgorithmAvailableSpace> {
    AlgorithmSize::new(
        AlgorithmAvailableSpace::Definite(width),
        AlgorithmAvailableSpace::Definite(height),
    )
}

fn zero_measure(
    _known: AlgorithmSize<Option<f32>>,
    _available: AlgorithmSize<AlgorithmAvailableSpace>,
    _node: AlgorithmNodeId,
    _context: Option<&mut ()>,
    _line_constraints: Option<&FloatLineConstraints>,
) -> AlgorithmSize<f32> {
    AlgorithmSize::new(0.0, 0.0)
}

#[test]
fn automatic_minimum_mode_stays_on_buckrams_content_sizing_route() {
    assert!(is_content_sizing_mode(SizingMode::ContentSize));
    assert!(is_content_sizing_mode(
        SizingMode::ContentSizeForAutomaticMinimum
    ));
    assert!(!is_content_sizing_mode(SizingMode::InherentSize));
}

#[test]
fn sequential_multicol_input_is_preserved_but_dispatch_stays_explicitly_unsupported() {
    let mut tree = AlgorithmTree::<Style, (), &str>::new();
    let child = tree.new_with_children(AlgorithmKind::Leaf, Style::default(), &[], "child");
    let root = tree.new_with_children(
        AlgorithmKind::Block,
        Style {
            display: Display::Block,
            size: taffy::Size {
                width: Dimension::length(264.0),
                height: Dimension::length(120.0),
            },
            ..Style::default()
        },
        &[child],
        "columns",
    );
    let input = crate::SequentialMulticolInput::new(3, Some(80.0), 12.0, crate::MulticolFill::Auto)
        .expect("the arbitrary input is valid");
    tree.set_sequential_multicol(root, input);
    assert_eq!(tree.sequential_multicol(root), Some(input));

    tree.compute_layout_with_measure(root, available(264.0, 120.0), zero_measure);
    let output = tree.fragmentation_output();
    assert_eq!(
        output.dispatch,
        FragmentationDispatch::SequentialMulticolUnsupported
    );
    assert_eq!(output.sequential_inputs, vec![(root, input)]);
}

#[test]
fn fragmentation_output_is_run_scoped_and_continuous_runs_are_distinguishable() {
    let mut tree = AlgorithmTree::<Style, (), &str>::new();
    let root = tree.new_with_children(
        AlgorithmKind::Block,
        Style {
            display: Display::Block,
            size: taffy::Size {
                width: Dimension::length(100.0),
                height: Dimension::length(40.0),
            },
            ..Style::default()
        },
        &[],
        "root",
    );
    let input = crate::SequentialMulticolInput::new(2, Some(40.0), 20.0, crate::MulticolFill::Auto)
        .expect("the input is valid");
    tree.set_sequential_multicol(root, input);
    tree.compute_layout_with_measure(root, available(100.0, 40.0), zero_measure);
    assert_eq!(
        tree.fragmentation_output().dispatch,
        FragmentationDispatch::SequentialMulticolUnsupported
    );
    tree.clear_sequential_multicol(root);
    tree.compute_layout_with_measure(root, available(100.0, 40.0), zero_measure);
    assert_eq!(
        tree.fragmentation_output().dispatch,
        FragmentationDispatch::Continuous
    );
    assert!(tree.fragmentation_output().sequential_inputs.is_empty());
}

#[test]
fn sequential_multicol_input_rejects_balancing_and_nondefinite_values() {
    assert!(
        crate::SequentialMulticolInput::new(2, Some(100.0), 20.0, crate::MulticolFill::Balance,)
            .is_none()
    );
    assert!(
        crate::SequentialMulticolInput::new(2, Some(f32::NAN), 20.0, crate::MulticolFill::Auto,)
            .is_none()
    );
}

#[test]
fn flex_dispatch_preserves_exact_placements_and_sources() {
    let mut tree = AlgorithmTree::<Style, (), &str>::new();
    let child_style = Style {
        size: taffy::Size {
            width: Dimension::length(50.0),
            height: Dimension::length(20.0),
        },
        ..Style::default()
    };
    let first = tree.new_with_children(AlgorithmKind::Leaf, child_style.clone(), &[], "first");
    let second = tree.new_with_children(AlgorithmKind::Leaf, child_style, &[], "second");
    let root = tree.new_with_children(
        AlgorithmKind::Flex,
        Style {
            display: Display::Flex,
            size: taffy::Size {
                width: Dimension::length(200.0),
                height: Dimension::length(40.0),
            },
            ..Style::default()
        },
        &[first, second],
        "root",
    );

    tree.compute_layout_with_measure(root, available(200.0, 40.0), zero_measure);

    assert_eq!(tree.source(second), &"second");
    assert_eq!(
        tree.layout(first),
        AlgorithmLayout {
            width: 50.0,
            height: 20.0,
            ..AlgorithmLayout::default()
        }
    );
    assert_eq!(
        tree.layout(second),
        AlgorithmLayout {
            x: 50.0,
            width: 50.0,
            height: 20.0,
            ..AlgorithmLayout::default()
        }
    );
}

#[test]
fn buckram_block_parent_keeps_absolute_child_at_its_static_position() {
    let mut tree = AlgorithmTree::<Style, (), u8>::new();
    let positioned = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        BlockStyle {
            position: crate::BlockPosition::Absolute,
            ..BlockStyle::default()
        },
        Style {
            position: Position::Absolute,
            inset: Rect {
                left: length(40.0_f32),
                top: length(12.0_f32),
                ..Rect::auto()
            },
            size: taffy::Size {
                width: Dimension::length(30.0),
                height: Dimension::length(20.0),
            },
            ..Style::default()
        },
        &[],
        1,
    );
    let root = tree.new_with_children(
        AlgorithmKind::Block,
        Style {
            display: Display::Block,
            size: taffy::Size {
                width: Dimension::length(200.0),
                height: Dimension::length(100.0),
            },
            ..Style::default()
        },
        &[positioned],
        0,
    );

    tree.compute_layout_with_measure(root, available(200.0, 100.0), zero_measure);

    assert_eq!(tree.block_algorithm(root), Some(BlockAlgorithm::Buckram));
    assert_eq!(tree.layout(positioned).x, 0.0);
    assert_eq!(tree.layout(positioned).y, 0.0);
    assert_eq!(tree.static_layout(positioned).x, 0.0);
    assert_eq!(tree.static_layout(positioned).y, 0.0);
}

#[test]
fn positioned_empty_leaf_has_zero_intrinsic_inline_contributions() {
    let mut tree = AlgorithmTree::<Style, (), u8>::new();
    let positioned = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        BlockStyle {
            position: crate::BlockPosition::Absolute,
            ..BlockStyle::default()
        },
        Style {
            position: Position::Absolute,
            ..Style::default()
        },
        &[],
        1,
    );

    assert_eq!(
        tree.positioned_intrinsic_inline_sizes(positioned, zero_measure),
        IntrinsicSizes::new(0.0, 0.0),
    );
}

#[test]
fn vertical_positioned_block_intrinsic_and_inline_size_follow_height() {
    use crate::{Direction, WritingMode};

    let vertical = FlowAxes::new(WritingMode::VerticalRl, Direction::Ltr);
    let mut tree = AlgorithmTree::<Style, (), u8>::new();
    let child = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            flow: vertical,
            containing_flow: vertical,
            size: crate::BlockDimensions::new(
                BlockSizeValue::Auto,
                BlockSizeValue::Length(FlowLength::px(20.0)),
            ),
            ..BlockStyle::default()
        },
        Style::default(),
        &[],
        2,
    );
    let positioned = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            flow: vertical,
            containing_flow: vertical,
            position: crate::BlockPosition::Absolute,
            ..BlockStyle::default()
        },
        Style::default(),
        &[child],
        1,
    );

    assert_eq!(
        tree.positioned_intrinsic_inline_sizes(positioned, zero_measure),
        IntrinsicSizes::new(20.0, 20.0),
    );

    tree.set_positioned_inline_size(positioned, 170.0);
    assert_eq!(
        tree.block_style(positioned).size.height,
        BlockSizeValue::Length(FlowLength::px(170.0)),
    );
}

#[test]
fn grid_static_layout_uses_content_box_before_insets() {
    let mut tree = AlgorithmTree::<Style, (), u8>::new();
    let positioned = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        BlockStyle {
            position: crate::BlockPosition::Absolute,
            ..BlockStyle::default()
        },
        Style {
            inset: Rect {
                left: length(18.0_f32),
                top: length(9.0_f32),
                ..Rect::auto()
            },
            size: taffy::Size {
                width: Dimension::length(30.0),
                height: Dimension::length(20.0),
            },
            grid_column: taffy::Line {
                start: line(2),
                end: line(3),
            },
            grid_row: taffy::Line {
                start: line(2),
                end: line(3),
            },
            ..Style::default()
        },
        &[],
        1,
    );
    let root = tree.new_with_children(
        AlgorithmKind::Grid,
        Style {
            display: Display::Grid,
            size: taffy::Size {
                width: Dimension::length(200.0),
                height: Dimension::length(100.0),
            },
            grid_template_columns: vec![length(80.0_f32), length(120.0_f32)],
            grid_template_rows: vec![length(40.0_f32), length(60.0_f32)],
            ..Style::default()
        },
        &[positioned],
        0,
    );
    tree.enable_flex_grid_static_position_provider(positioned);

    tree.compute_layout_with_measure(root, available(200.0, 100.0), zero_measure);

    assert_eq!(tree.layout(positioned).x, 98.0);
    assert_eq!(tree.layout(positioned).y, 49.0);
    assert_eq!(tree.static_layout(positioned).x, 0.0);
    assert_eq!(tree.static_layout(positioned).y, 0.0);
    assert_eq!(
        tree.grid_positioned_area(positioned),
        Some(PhysicalRect {
            x: 80.0,
            y: 40.0,
            width: 120.0,
            height: 60.0,
        }),
        "the grid area remains available to the positioned containing-block route"
    );
}

#[test]
fn grid_static_layout_uses_the_grid_area_when_the_grid_is_the_containing_block() {
    let mut tree = AlgorithmTree::<Style, (), u8>::new();
    let positioned = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        BlockStyle {
            position: crate::BlockPosition::Absolute,
            ..BlockStyle::default()
        },
        Style {
            size: taffy::Size {
                width: Dimension::length(30.0),
                height: Dimension::length(20.0),
            },
            grid_column: taffy::Line {
                start: line(2),
                end: line(3),
            },
            grid_row: taffy::Line {
                start: line(2),
                end: line(3),
            },
            ..Style::default()
        },
        &[],
        1,
    );
    let root = tree.new_with_children(
        AlgorithmKind::Grid,
        Style {
            display: Display::Grid,
            size: taffy::Size {
                width: Dimension::length(200.0),
                height: Dimension::length(100.0),
            },
            padding: Rect {
                left: length(10.0_f32),
                right: length(30.0_f32),
                top: length(5.0_f32),
                bottom: length(25.0_f32),
            },
            grid_template_columns: vec![length(80.0_f32), length(80.0_f32)],
            grid_template_rows: vec![length(40.0_f32), length(30.0_f32)],
            ..Style::default()
        },
        &[positioned],
        0,
    );
    tree.enable_flex_grid_static_position_provider(positioned);
    tree.use_grid_area_for_static_position(positioned);

    tree.compute_layout_with_measure(root, available(200.0, 100.0), zero_measure);

    // Column 2 starts at padding 10 + 80 and row 2 at padding 5 + 40: the
    // static location is the start of the placed area, not the content
    // origin (10, 5) the unselected provider would report.
    assert_eq!(
        (
            tree.static_layout(positioned).x,
            tree.static_layout(positioned).y
        ),
        (90.0, 45.0)
    );
    assert_eq!(
        tree.grid_positioned_area(positioned),
        Some(PhysicalRect {
            x: 90.0,
            y: 45.0,
            width: 80.0,
            height: 30.0,
        })
    );
}

#[test]
fn grid_static_layout_auto_lines_align_in_the_padding_box_when_selected() {
    let mut tree = AlgorithmTree::<Style, (), u8>::new();
    let positioned = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        BlockStyle {
            position: crate::BlockPosition::Absolute,
            ..BlockStyle::default()
        },
        Style {
            size: taffy::Size {
                width: Dimension::length(30.0),
                height: Dimension::length(20.0),
            },
            align_self: Some(AlignItems::CENTER),
            justify_self: Some(AlignItems::CENTER),
            ..Style::default()
        },
        &[],
        1,
    );
    let root = tree.new_with_children(
        AlgorithmKind::Grid,
        Style {
            display: Display::Grid,
            size: taffy::Size {
                width: Dimension::length(200.0),
                height: Dimension::length(100.0),
            },
            padding: Rect {
                left: length(10.0_f32),
                right: length(30.0_f32),
                top: length(5.0_f32),
                bottom: length(25.0_f32),
            },
            ..Style::default()
        },
        &[positioned],
        0,
    );
    tree.enable_flex_grid_static_position_provider(positioned);
    tree.use_grid_area_for_static_position(positioned);

    tree.compute_layout_with_measure(root, available(200.0, 100.0), zero_measure);

    // `auto` lines bound the area at the padding edges, so centering uses
    // the 200 x 100 padding box: (85, 40). Centering in the asymmetric
    // 160 x 70 content box would give (75, 30).
    assert_eq!(
        (
            tree.static_layout(positioned).x,
            tree.static_layout(positioned).y
        ),
        (85.0, 40.0)
    );
}

#[test]
fn grid_positioned_area_keeps_the_final_explicit_track_rectangle() {
    let mut tree = AlgorithmTree::<Style, (), u8>::new();
    let positioned = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        BlockStyle {
            position: crate::BlockPosition::Absolute,
            ..BlockStyle::default()
        },
        Style {
            position: Position::Absolute,
            grid_column: taffy::Line {
                start: line(2),
                end: line(3),
            },
            grid_row: taffy::Line {
                start: line(2),
                end: line(3),
            },
            ..Style::default()
        },
        &[],
        1,
    );
    let root = tree.new_with_children(
        AlgorithmKind::Grid,
        Style {
            display: Display::Grid,
            size: taffy::Size {
                width: Dimension::length(500.0),
                height: Dimension::length(250.0),
            },
            grid_template_columns: vec![length(200.0_f32), length(300.0_f32)],
            grid_template_rows: vec![length(150.0_f32), length(100.0_f32)],
            ..Style::default()
        },
        &[positioned],
        0,
    );
    tree.enable_flex_grid_static_position_provider(positioned);

    tree.compute_layout_with_measure(root, available(500.0, 250.0), zero_measure);

    assert_eq!(
        tree.grid_positioned_area(positioned),
        Some(PhysicalRect {
            x: 200.0,
            y: 150.0,
            width: 300.0,
            height: 100.0,
        })
    );
}

#[test]
fn grid_positioned_area_projects_track_axes_through_container_flow() {
    use crate::{Direction, WritingMode};

    for (flow, expected) in [
        (
            FlowAxes::new(WritingMode::VerticalRl, Direction::Ltr),
            PhysicalRect {
                x: 0.0,
                y: 20.0,
                width: 70.0,
                height: 60.0,
            },
        ),
        (
            FlowAxes::new(WritingMode::VerticalLr, Direction::Ltr),
            PhysicalRect {
                x: 30.0,
                y: 20.0,
                width: 70.0,
                height: 60.0,
            },
        ),
        (
            FlowAxes::new(WritingMode::VerticalRl, Direction::Rtl),
            PhysicalRect {
                x: 0.0,
                y: 0.0,
                width: 70.0,
                height: 60.0,
            },
        ),
        (
            FlowAxes::new(WritingMode::VerticalLr, Direction::Rtl),
            PhysicalRect {
                x: 30.0,
                y: 0.0,
                width: 70.0,
                height: 60.0,
            },
        ),
    ] {
        let mut tree = AlgorithmTree::<Style, (), u8>::new();
        let positioned = tree.new_with_children_and_block_style(
            AlgorithmKind::Leaf,
            BlockStyle {
                position: crate::BlockPosition::Absolute,
                ..BlockStyle::default()
            },
            Style {
                position: Position::Absolute,
                grid_column: taffy::Line {
                    start: line(2),
                    end: line(3),
                },
                grid_row: taffy::Line {
                    start: line(2),
                    end: line(3),
                },
                ..Style::default()
            },
            &[],
            1,
        );
        let root = tree.new_with_children_and_block_style(
            AlgorithmKind::Grid,
            BlockStyle {
                flow,
                containing_flow: flow,
                establishes_bfc: true,
                ..BlockStyle::default()
            },
            Style {
                display: Display::Grid,
                size: taffy::Size {
                    width: Dimension::length(100.0),
                    height: Dimension::length(80.0),
                },
                grid_template_columns: vec![length(20.0_f32), length(60.0_f32)],
                grid_template_rows: vec![length(30.0_f32), length(70.0_f32)],
                ..Style::default()
            },
            &[positioned],
            0,
        );
        tree.enable_flex_grid_static_position_provider(positioned);
        tree.compute_layout_with_measure(root, available(100.0, 80.0), zero_measure);

        assert_eq!(
            tree.grid_positioned_area(positioned),
            Some(expected),
            "{flow:?}: track coordinates project through the grid flow"
        );
    }
}

#[test]
fn flex_static_layout_keeps_alignment_when_insets_place_the_item() {
    let mut tree = AlgorithmTree::<Style, (), u8>::new();
    let positioned = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        BlockStyle {
            position: crate::BlockPosition::Absolute,
            ..BlockStyle::default()
        },
        Style {
            inset: Rect {
                left: length(18.0_f32),
                top: length(9.0_f32),
                ..Rect::auto()
            },
            size: taffy::Size {
                width: Dimension::length(30.0),
                height: Dimension::length(20.0),
            },
            ..Style::default()
        },
        &[],
        1,
    );
    let root = tree.new_with_children(
        AlgorithmKind::Flex,
        Style {
            display: Display::Flex,
            size: taffy::Size {
                width: Dimension::length(200.0),
                height: Dimension::length(100.0),
            },
            justify_content: Some(JustifyContent::CENTER),
            align_items: Some(AlignItems::END),
            ..Style::default()
        },
        &[positioned],
        0,
    );
    tree.enable_flex_grid_static_position_provider(positioned);

    tree.compute_layout_with_measure(root, available(200.0, 100.0), zero_measure);

    assert_eq!(
        (tree.layout(positioned).x, tree.layout(positioned).y),
        (18.0, 9.0)
    );
    assert_eq!(
        (
            tree.static_layout(positioned).x,
            tree.static_layout(positioned).y
        ),
        (85.0, 80.0)
    );
}

#[test]
fn grid_dispatch_preserves_exact_track_geometry() {
    let mut tree = AlgorithmTree::<Style, (), u8>::new();
    let children = (0..4)
        .map(|source| tree.new_with_children(AlgorithmKind::Leaf, Style::default(), &[], source))
        .collect::<Vec<_>>();
    let root = tree.new_with_children(
        AlgorithmKind::Grid,
        Style {
            display: Display::Grid,
            size: taffy::Size {
                width: Dimension::length(200.0),
                height: Dimension::length(100.0),
            },
            grid_template_columns: vec![length(100.0_f32), fr(1.0_f32)],
            grid_template_rows: vec![length(50.0_f32), length(50.0_f32)],
            ..Style::default()
        },
        &children,
        9,
    );

    tree.compute_layout_with_measure(root, available(200.0, 100.0), zero_measure);

    assert_eq!(
        (tree.layout(children[0]).x, tree.layout(children[0]).y),
        (0.0, 0.0)
    );
    assert_eq!(
        (tree.layout(children[1]).x, tree.layout(children[1]).y),
        (100.0, 0.0)
    );
    assert_eq!(
        (tree.layout(children[2]).x, tree.layout(children[2]).y),
        (0.0, 50.0)
    );
    assert_eq!(
        (tree.layout(children[3]).x, tree.layout(children[3]).y),
        (100.0, 50.0)
    );
}

#[test]
fn buckram_kind_selects_the_algorithm_independently_of_backend_display() {
    let mut tree = AlgorithmTree::<Style, (), u8>::new();
    let child_style = Style {
        size: taffy::Size {
            width: Dimension::length(50.0),
            height: Dimension::length(20.0),
        },
        ..Style::default()
    };
    let first = tree.new_with_children(AlgorithmKind::Leaf, child_style.clone(), &[], 1);
    let second = tree.new_with_children(AlgorithmKind::Leaf, child_style, &[], 2);
    let root = tree.new_with_children(
        AlgorithmKind::Flex,
        Style {
            // Deliberately contradictory. Dispatch must follow Buckram's
            // formatting role, not this private backend field.
            display: Display::Block,
            size: taffy::Size {
                width: Dimension::length(200.0),
                height: Dimension::length(40.0),
            },
            ..Style::default()
        },
        &[first, second],
        0,
    );

    tree.compute_layout_with_measure(root, available(200.0, 40.0), zero_measure);

    assert_eq!(tree.kind(root), AlgorithmKind::Flex);
    assert_eq!(tree.layout(first).x, 0.0);
    assert_eq!(tree.layout(second).x, 50.0);
    assert_eq!(tree.layout(second).y, 0.0);
}

#[test]
fn buckram_block_flow_uses_css_inputs_instead_of_backend_sizes() {
    let mut tree = AlgorithmTree::<Style, (), u8>::new();
    let backend_child_style = Style {
        size: taffy::Size {
            // Deliberately contradictory: Buckram's CSS input says 80px.
            width: Dimension::length(10.0),
            height: Dimension::length(20.0),
        },
        ..Style::default()
    };
    let block_child_style = BlockStyle {
        size: crate::BlockDimensions::new(
            BlockSizeValue::Length(crate::FlowLength::px(80.0)),
            BlockSizeValue::Length(crate::FlowLength::px(20.0)),
        ),
        ..BlockStyle::default()
    };
    let first = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        block_child_style,
        backend_child_style.clone(),
        &[],
        1,
    );
    let second = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        block_child_style,
        backend_child_style,
        &[],
        2,
    );
    let root = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle::default(),
        Style {
            display: Display::Block,
            size: taffy::Size {
                width: Dimension::length(200.0),
                height: Dimension::auto(),
            },
            ..Style::default()
        },
        &[first, second],
        0,
    );

    tree.compute_layout_with_measure(root, available(200.0, 100.0), zero_measure);

    assert_eq!(tree.layout(first).width, 80.0);
    assert_eq!(tree.layout(second).width, 80.0);
    assert_eq!(tree.layout(first).y, 0.0);
    assert_eq!(tree.layout(second).y, 20.0);
    assert_eq!(tree.layout(root).height, 40.0);
    assert_eq!(tree.block_algorithm(root), Some(BlockAlgorithm::Buckram));
}

#[test]
fn buckram_block_flow_propagates_parent_child_and_empty_margin_chains() {
    fn block_style(height: f32, margin_top: f32, margin_bottom: f32) -> BlockStyle {
        BlockStyle {
            size: crate::BlockDimensions::new(
                BlockSizeValue::Auto,
                BlockSizeValue::Length(crate::FlowLength::px(height)),
            ),
            margin: PhysicalSides {
                top: crate::FlowLengthAuto::Value(crate::FlowLength::px(margin_top)),
                right: crate::FlowLengthAuto::ZERO,
                bottom: crate::FlowLengthAuto::Value(crate::FlowLength::px(margin_bottom)),
                left: crate::FlowLengthAuto::ZERO,
            },
            ..BlockStyle::default()
        }
    }

    let mut tree = AlgorithmTree::<Style, (), u8>::new();
    let child = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        block_style(20.0, 30.0, 40.0),
        Style {
            size: taffy::Size {
                width: Dimension::auto(),
                height: Dimension::length(20.0),
            },
            ..Style::default()
        },
        &[],
        2,
    );
    let parent = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            margin: PhysicalSides {
                top: crate::FlowLengthAuto::Value(crate::FlowLength::px(10.0)),
                right: crate::FlowLengthAuto::ZERO,
                bottom: crate::FlowLengthAuto::Value(crate::FlowLength::px(15.0)),
                left: crate::FlowLengthAuto::ZERO,
            },
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            ..Style::default()
        },
        &[child],
        1,
    );
    let after = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        block_style(10.0, 12.0, 0.0),
        Style {
            size: taffy::Size {
                width: Dimension::auto(),
                height: Dimension::length(10.0),
            },
            ..Style::default()
        },
        &[],
        3,
    );
    let root = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle::default(),
        Style {
            display: Display::Block,
            size: taffy::Size {
                width: Dimension::length(200.0),
                height: Dimension::auto(),
            },
            ..Style::default()
        },
        &[parent, after],
        0,
    );

    tree.compute_layout_with_measure(root, available(200.0, 200.0), zero_measure);

    assert_eq!(tree.layout(parent).y, 30.0);
    assert_eq!(tree.layout(child).y, 0.0);
    assert_eq!(tree.layout(after).y, 90.0);
    assert_eq!(tree.layout(root).height, 100.0);
    assert_eq!(tree.block_algorithm(parent), Some(BlockAlgorithm::Buckram));
    assert_eq!(tree.block_algorithm(root), Some(BlockAlgorithm::Buckram));
    assert_eq!(
        tree.block_margins(parent),
        Some(BlockMarginState {
            block_start: CollapsedMargin::from_margin(30.0),
            block_end: CollapsedMargin::from_margin(40.0),
            collapses_through: false,
        })
    );
}

#[test]
fn buckram_places_direct_floats_and_clearance_inside_an_independent_bfc() {
    fn float_style(side: FloatSide, width: f32, height: f32) -> BlockStyle {
        BlockStyle {
            size: crate::BlockDimensions::new(
                BlockSizeValue::Length(crate::FlowLength::px(width)),
                BlockSizeValue::Length(crate::FlowLength::px(height)),
            ),
            float: side,
            establishes_bfc: true,
            ..BlockStyle::default()
        }
    }
    fn backend_size(width: f32, height: f32) -> Style {
        Style {
            size: taffy::Size {
                width: Dimension::length(width),
                height: Dimension::length(height),
            },
            ..Style::default()
        }
    }

    let mut tree = AlgorithmTree::<Style, (), u8>::new();
    let left = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        float_style(FloatSide::Left, 80.0, 40.0),
        backend_size(80.0, 40.0),
        &[],
        1,
    );
    let right = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        float_style(FloatSide::Right, 60.0, 70.0),
        backend_size(60.0, 70.0),
        &[],
        2,
    );
    let clear = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        BlockStyle {
            size: crate::BlockDimensions::new(
                BlockSizeValue::Auto,
                BlockSizeValue::Length(crate::FlowLength::px(10.0)),
            ),
            clear: ClearSide::Both,
            ..BlockStyle::default()
        },
        backend_size(200.0, 10.0),
        &[],
        3,
    );
    let root = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            size: taffy::Size {
                width: Dimension::length(200.0),
                height: Dimension::auto(),
            },
            ..Style::default()
        },
        &[left, right, clear],
        0,
    );

    tree.compute_layout_with_measure(root, available(200.0, 200.0), zero_measure);

    assert_eq!((tree.layout(left).x, tree.layout(left).y), (0.0, 0.0));
    assert_eq!((tree.layout(right).x, tree.layout(right).y), (140.0, 0.0));
    assert_eq!((tree.layout(clear).x, tree.layout(clear).y), (0.0, 70.0));
    assert_eq!(tree.layout(root).height, 80.0);
    assert_eq!(tree.block_algorithm(root), Some(BlockAlgorithm::Buckram));
}

#[test]
fn buckram_carries_clearance_through_an_empty_collapsing_block() {
    let mut tree = AlgorithmTree::<Style, (), u8>::new();
    let float = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        BlockStyle {
            size: crate::BlockDimensions::new(
                BlockSizeValue::Length(crate::FlowLength::px(80.0)),
                BlockSizeValue::Length(crate::FlowLength::px(40.0)),
            ),
            float: FloatSide::Left,
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            size: taffy::Size {
                width: Dimension::length(80.0),
                height: Dimension::length(40.0),
            },
            ..Style::default()
        },
        &[],
        1,
    );
    let mut empty_style = BlockStyle {
        clear: ClearSide::Left,
        ..BlockStyle::default()
    };
    empty_style.margin.top = crate::FlowLengthAuto::Value(crate::FlowLength::px(10.0));
    empty_style.margin.bottom = crate::FlowLengthAuto::Value(crate::FlowLength::px(20.0));
    let empty = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        empty_style,
        Style {
            display: Display::Block,
            ..Style::default()
        },
        &[],
        2,
    );
    let mut following_style = BlockStyle {
        size: crate::BlockDimensions::new(
            BlockSizeValue::Auto,
            BlockSizeValue::Length(crate::FlowLength::px(10.0)),
        ),
        ..BlockStyle::default()
    };
    following_style.margin.top = crate::FlowLengthAuto::Value(crate::FlowLength::px(30.0));
    let following = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        following_style,
        Style {
            size: taffy::Size {
                width: Dimension::auto(),
                height: Dimension::length(10.0),
            },
            ..Style::default()
        },
        &[],
        3,
    );
    let root = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            size: taffy::Size {
                width: Dimension::length(200.0),
                height: Dimension::auto(),
            },
            ..Style::default()
        },
        &[float, empty, following],
        0,
    );

    tree.compute_layout_with_measure(root, available(200.0, 200.0), zero_measure);

    assert_eq!((tree.layout(float).x, tree.layout(float).y), (0.0, 0.0));
    assert_eq!(
        (tree.layout(empty).y, tree.layout(empty).height),
        (40.0, 0.0)
    );
    assert_eq!(
        (tree.layout(following).y, tree.layout(following).height),
        (70.0, 10.0)
    );
    assert_eq!(tree.layout(root).height, 80.0);
    assert_eq!(tree.block_algorithm(empty), Some(BlockAlgorithm::Buckram));
    assert_eq!(tree.block_algorithm(root), Some(BlockAlgorithm::Buckram));
}

#[test]
fn buckram_sizes_an_auto_float_from_inline_intrinsic_queries() {
    let mut tree = AlgorithmTree::<Style, Vec<AlgorithmAvailableSpace>, u8>::new();
    let lines = tree.new_leaf_with_context_and_block_style(
        BlockStyle::anonymous(FlowAxes::HORIZONTAL_LTR, FlowAxes::HORIZONTAL_LTR),
        Style::default(),
        Vec::new(),
        2,
    );
    let float = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            float: FloatSide::Left,
            establishes_bfc: true,
            shrink_to_fit: true,
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            ..Style::default()
        },
        &[lines],
        1,
    );
    tree.enable_intrinsic_shrink_to_fit(float);
    let root = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            size: taffy::Size {
                width: Dimension::length(100.0),
                height: Dimension::auto(),
            },
            ..Style::default()
        },
        &[float],
        0,
    );

    tree.compute_layout_with_measure(
        root,
        available(100.0, 200.0),
        |known, available, _, context, _| {
            if let Some(context) = context {
                context.push(available.width);
            }
            let measured_width = match available.width {
                AlgorithmAvailableSpace::Definite(width) => width,
                AlgorithmAvailableSpace::MinContent => 40.0,
                AlgorithmAvailableSpace::MaxContent => 120.0,
            };
            AlgorithmSize::new(
                known.width.unwrap_or(measured_width),
                known.height.unwrap_or(20.0),
            )
        },
    );

    assert_eq!(
        tree.layout(float),
        AlgorithmLayout {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 20.0,
        }
    );
    let queries = tree.context(lines).expect("inline queries");
    assert!(queries.contains(&AlgorithmAvailableSpace::MinContent));
    assert!(queries.contains(&AlgorithmAvailableSpace::MaxContent));
    assert!(queries.contains(&AlgorithmAvailableSpace::Definite(100.0)));
    assert_eq!(tree.layout(root).height, 20.0);
    assert_eq!(tree.block_algorithm(float), Some(BlockAlgorithm::Buckram));
    assert_eq!(tree.block_algorithm(root), Some(BlockAlgorithm::Buckram));
}

#[test]
fn buckram_queries_multi_child_and_block_content_intrinsics_for_shrink_to_fit() {
    fn layout_float(containing_width: f32, wrap_children: bool) -> (f32, usize) {
        let mut tree = AlgorithmTree::<Style, Vec<AlgorithmAvailableSpace>, u8>::new();
        let first = tree.new_leaf_with_context_and_block_style(
            BlockStyle::anonymous(FlowAxes::HORIZONTAL_LTR, FlowAxes::HORIZONTAL_LTR),
            Style::default(),
            Vec::new(),
            1,
        );
        let second = tree.new_leaf_with_context_and_block_style(
            BlockStyle::anonymous(FlowAxes::HORIZONTAL_LTR, FlowAxes::HORIZONTAL_LTR),
            Style::default(),
            Vec::new(),
            2,
        );
        let children = if wrap_children {
            let block = tree.new_with_children_and_block_style(
                AlgorithmKind::Block,
                BlockStyle::default(),
                Style::default(),
                &[first, second],
                3,
            );
            vec![block]
        } else {
            vec![first, second]
        };
        let float = tree.new_with_children_and_block_style(
            AlgorithmKind::Block,
            BlockStyle {
                float: FloatSide::Left,
                establishes_bfc: true,
                shrink_to_fit: true,
                ..BlockStyle::default()
            },
            Style::default(),
            &children,
            4,
        );
        tree.enable_intrinsic_shrink_to_fit(float);
        let root = tree.new_with_children_and_block_style(
            AlgorithmKind::Block,
            BlockStyle {
                establishes_bfc: true,
                ..BlockStyle::default()
            },
            Style {
                display: Display::Block,
                size: taffy::Size {
                    width: Dimension::length(containing_width),
                    height: Dimension::auto(),
                },
                ..Style::default()
            },
            &[float],
            0,
        );

        tree.compute_layout_with_measure(
            root,
            available(containing_width, 200.0),
            |known, available, _, context, _| {
                if let Some(context) = context {
                    context.push(available.width);
                }
                let width = match available.width {
                    AlgorithmAvailableSpace::Definite(width) => width,
                    AlgorithmAvailableSpace::MinContent => 40.0,
                    AlgorithmAvailableSpace::MaxContent => 120.0,
                };
                AlgorithmSize::new(known.width.unwrap_or(width), known.height.unwrap_or(10.0))
            },
        );

        assert!(
            tree.context(first)
                .expect("first inline context")
                .contains(&AlgorithmAvailableSpace::MinContent)
        );
        assert!(
            tree.context(second)
                .expect("second inline context")
                .contains(&AlgorithmAvailableSpace::MaxContent)
        );
        assert_eq!(tree.block_algorithm_counts().1, 0);
        (tree.layout(float).width, tree.block_algorithm_counts().0)
    }

    for wrap_children in [false, true] {
        assert_eq!(layout_float(30.0, wrap_children).0, 40.0);
        assert_eq!(layout_float(80.0, wrap_children).0, 80.0);
        let (width, buckram_blocks) = layout_float(200.0, wrap_children);
        assert_eq!(width, 120.0);
        assert!(buckram_blocks >= if wrap_children { 3 } else { 2 });
    }
}

#[test]
fn buckram_queries_atomic_inline_intrinsics_without_float_placement() {
    fn layout_atomic_inline(containing_width: f32) -> f32 {
        let mut tree = AlgorithmTree::<Style, Vec<AlgorithmAvailableSpace>, u8>::new();
        let lines = tree.new_leaf_with_context_and_block_style(
            BlockStyle::anonymous(FlowAxes::HORIZONTAL_LTR, FlowAxes::HORIZONTAL_LTR),
            Style::default(),
            Vec::new(),
            1,
        );
        let inline_block = tree.new_with_children_and_block_style(
            AlgorithmKind::Block,
            BlockStyle {
                establishes_bfc: true,
                shrink_to_fit: true,
                ..BlockStyle::default()
            },
            Style::default(),
            &[lines],
            2,
        );
        tree.enable_float_avoidance(inline_block);
        tree.enable_intrinsic_shrink_to_fit(inline_block);
        let root = tree.new_with_children_and_block_style(
            AlgorithmKind::Block,
            BlockStyle {
                establishes_bfc: true,
                ..BlockStyle::default()
            },
            Style {
                display: Display::Block,
                size: taffy::Size {
                    width: Dimension::length(containing_width),
                    height: Dimension::auto(),
                },
                ..Style::default()
            },
            &[inline_block],
            0,
        );

        tree.compute_layout_with_measure(
            root,
            available(containing_width, 200.0),
            |known, available, _, context, _| {
                if let Some(context) = context {
                    context.push(available.width);
                }
                let width = match available.width {
                    AlgorithmAvailableSpace::Definite(width) => width,
                    AlgorithmAvailableSpace::MinContent => 40.0,
                    AlgorithmAvailableSpace::MaxContent => 120.0,
                };
                AlgorithmSize::new(known.width.unwrap_or(width), known.height.unwrap_or(10.0))
            },
        );

        assert_eq!(tree.block_algorithm_counts().1, 0);
        tree.layout(inline_block).width
    }

    assert_eq!(layout_atomic_inline(30.0), 40.0);
    assert_eq!(layout_atomic_inline(80.0), 80.0);
    assert_eq!(layout_atomic_inline(200.0), 120.0);
}

#[test]
fn buckram_remeasures_opted_in_bfcs_inside_the_float_band() {
    let mut tree = AlgorithmTree::<Style, Vec<f32>, u8>::new();
    let float = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        BlockStyle {
            size: crate::BlockDimensions::new(
                BlockSizeValue::Length(crate::FlowLength::px(80.0)),
                BlockSizeValue::Length(crate::FlowLength::px(40.0)),
            ),
            float: FloatSide::Left,
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            size: taffy::Size {
                width: Dimension::length(80.0),
                height: Dimension::length(40.0),
            },
            ..Style::default()
        },
        &[],
        1,
    );
    let auto_bfc = tree.new_leaf_with_context_and_block_style(
        BlockStyle {
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style::default(),
        Vec::new(),
        2,
    );
    tree.enable_float_avoidance(auto_bfc);
    let definite_bfc = tree.new_leaf_with_context_and_block_style(
        BlockStyle {
            size: crate::BlockDimensions::new(
                BlockSizeValue::Length(crate::FlowLength::px(150.0)),
                BlockSizeValue::Auto,
            ),
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style::default(),
        Vec::new(),
        3,
    );
    tree.enable_float_avoidance(definite_bfc);
    let root = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            size: taffy::Size {
                width: Dimension::length(200.0),
                height: Dimension::auto(),
            },
            ..Style::default()
        },
        &[float, auto_bfc, definite_bfc],
        0,
    );

    tree.compute_layout_with_measure(
        root,
        available(200.0, 200.0),
        |known, available, _, context, _| {
            let width = known.width.unwrap_or(match available.width {
                AlgorithmAvailableSpace::Definite(width) => width,
                AlgorithmAvailableSpace::MinContent => 0.0,
                AlgorithmAvailableSpace::MaxContent => f32::INFINITY,
            });
            if let Some(context) = context {
                context.push(width);
            }
            AlgorithmSize::new(width, 20.0)
        },
    );

    assert_eq!(tree.block_algorithm(root), Some(BlockAlgorithm::Buckram));
    assert_eq!(tree.context(auto_bfc), Some(&vec![120.0]));
    assert_eq!(tree.context(definite_bfc), Some(&vec![150.0]));
    assert_eq!(
        tree.layout(auto_bfc),
        AlgorithmLayout {
            x: 80.0,
            y: 0.0,
            width: 120.0,
            height: 20.0,
        }
    );
    assert_eq!(
        tree.layout(definite_bfc),
        AlgorithmLayout {
            x: 0.0,
            y: 40.0,
            width: 150.0,
            height: 20.0,
        }
    );
    assert_eq!(tree.layout(root).height, 60.0);
}

#[test]
fn buckram_applies_bfc_inline_margins_before_avoiding_a_float() {
    let mut tree = AlgorithmTree::<Style, Vec<f32>, u8>::new();
    let float = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        BlockStyle {
            size: crate::BlockDimensions::new(
                BlockSizeValue::Length(crate::FlowLength::px(50.0)),
                BlockSizeValue::Length(crate::FlowLength::px(40.0)),
            ),
            float: FloatSide::Right,
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            size: taffy::Size {
                width: Dimension::length(50.0),
                height: Dimension::length(40.0),
            },
            ..Style::default()
        },
        &[],
        1,
    );
    let mut bfc_style = BlockStyle {
        establishes_bfc: true,
        ..BlockStyle::default()
    };
    bfc_style.margin.left = crate::FlowLengthAuto::Value(crate::FlowLength::px(51.0));
    let bfc =
        tree.new_leaf_with_context_and_block_style(bfc_style, Style::default(), Vec::new(), 2);
    tree.enable_float_avoidance(bfc);
    let root = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            size: taffy::Size {
                width: Dimension::length(100.0),
                height: Dimension::auto(),
            },
            ..Style::default()
        },
        &[float, bfc],
        0,
    );

    tree.compute_layout_with_measure(
        root,
        available(100.0, 200.0),
        |known, available, _, context, _| {
            let width = known.width.unwrap_or(match available.width {
                AlgorithmAvailableSpace::Definite(width) => width,
                AlgorithmAvailableSpace::MinContent => 0.0,
                AlgorithmAvailableSpace::MaxContent => f32::INFINITY,
            });
            if let Some(context) = context {
                context.push(width);
            }
            AlgorithmSize::new(width, 60.0)
        },
    );

    assert_eq!(
        tree.layout(bfc),
        AlgorithmLayout {
            x: 51.0,
            y: 40.0,
            width: 49.0,
            height: 60.0,
        }
    );
    assert_eq!(tree.context(bfc), Some(&vec![49.0]));
    assert_eq!(tree.layout(root).height, 100.0);
    assert_eq!(tree.block_algorithm(root), Some(BlockAlgorithm::Buckram));
}

#[test]
fn buckram_places_flex_and_grid_algorithms_around_a_float() {
    let mut tree = AlgorithmTree::<Style, (), u8>::new();
    let float = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        BlockStyle {
            size: crate::BlockDimensions::new(
                BlockSizeValue::Length(crate::FlowLength::px(40.0)),
                BlockSizeValue::Length(crate::FlowLength::px(40.0)),
            ),
            float: FloatSide::Left,
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            size: taffy::Size {
                width: Dimension::length(40.0),
                height: Dimension::length(40.0),
            },
            ..Style::default()
        },
        &[],
        1,
    );
    let flex_child = tree.new_with_children(
        AlgorithmKind::Leaf,
        Style {
            size: taffy::Size {
                width: Dimension::length(20.0),
                height: Dimension::length(10.0),
            },
            ..Style::default()
        },
        &[],
        2,
    );
    let flex = tree.new_with_children_and_block_style(
        AlgorithmKind::Flex,
        BlockStyle {
            size: crate::BlockDimensions::new(
                BlockSizeValue::Auto,
                BlockSizeValue::Length(crate::FlowLength::px(20.0)),
            ),
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            display: Display::Flex,
            size: taffy::Size {
                width: Dimension::auto(),
                height: Dimension::length(20.0),
            },
            ..Style::default()
        },
        &[flex_child],
        3,
    );
    tree.enable_float_avoidance(flex);
    let grid_child = tree.new_with_children(
        AlgorithmKind::Leaf,
        Style {
            size: taffy::Size {
                width: Dimension::length(20.0),
                height: Dimension::length(10.0),
            },
            ..Style::default()
        },
        &[],
        4,
    );
    let grid = tree.new_with_children_and_block_style(
        AlgorithmKind::Grid,
        BlockStyle {
            size: crate::BlockDimensions::new(
                BlockSizeValue::Length(crate::FlowLength::px(70.0)),
                BlockSizeValue::Length(crate::FlowLength::px(20.0)),
            ),
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            display: Display::Grid,
            size: taffy::Size {
                width: Dimension::length(70.0),
                height: Dimension::length(20.0),
            },
            grid_template_columns: vec![length(20.0_f32)],
            ..Style::default()
        },
        &[grid_child],
        5,
    );
    tree.enable_float_avoidance(grid);
    let root = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            size: taffy::Size {
                width: Dimension::length(100.0),
                height: Dimension::auto(),
            },
            ..Style::default()
        },
        &[float, flex, grid],
        0,
    );

    tree.compute_layout_with_measure(root, available(100.0, 200.0), zero_measure);

    assert_eq!(
        tree.layout(flex),
        AlgorithmLayout {
            x: 40.0,
            y: 0.0,
            width: 60.0,
            height: 20.0,
        }
    );
    assert_eq!(
        tree.layout(flex_child),
        AlgorithmLayout {
            width: 20.0,
            height: 10.0,
            ..AlgorithmLayout::default()
        }
    );
    assert_eq!(
        tree.layout(grid),
        AlgorithmLayout {
            x: 0.0,
            y: 40.0,
            width: 70.0,
            height: 20.0,
        }
    );
    assert_eq!(
        tree.layout(grid_child),
        AlgorithmLayout {
            width: 20.0,
            height: 10.0,
            ..AlgorithmLayout::default()
        }
    );
    assert_eq!(tree.layout(root).height, 60.0);
    assert_eq!(tree.block_algorithm(root), Some(BlockAlgorithm::Buckram));
    assert_eq!(tree.block_algorithm(flex), None);
    assert_eq!(tree.block_algorithm(grid), None);
}

#[test]
fn flex_without_an_active_float_does_not_widen_the_buckram_block_lane() {
    let mut tree = AlgorithmTree::<Style, (), u8>::new();
    let flex = tree.new_with_children_and_block_style(
        AlgorithmKind::Flex,
        BlockStyle {
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            display: Display::Flex,
            size: taffy::Size {
                width: Dimension::auto(),
                height: Dimension::length(20.0),
            },
            ..Style::default()
        },
        &[],
        1,
    );
    tree.enable_float_avoidance(flex);
    let root = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            size: taffy::Size {
                width: Dimension::length(100.0),
                height: Dimension::auto(),
            },
            ..Style::default()
        },
        &[flex],
        0,
    );

    tree.compute_layout_with_measure(root, available(100.0, 200.0), zero_measure);

    assert_eq!(tree.block_algorithm(root), Some(BlockAlgorithm::Taffy));
}

#[test]
fn buckram_delivers_float_constraints_to_a_direct_inline_leaf() {
    let mut tree = AlgorithmTree::<Style, Vec<crate::FloatAvailableSpace>, u8>::new();
    let float = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        BlockStyle {
            size: crate::BlockDimensions::new(
                BlockSizeValue::Length(crate::FlowLength::px(80.0)),
                BlockSizeValue::Length(crate::FlowLength::px(40.0)),
            ),
            float: FloatSide::Left,
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            size: taffy::Size {
                width: Dimension::length(80.0),
                height: Dimension::length(40.0),
            },
            ..Style::default()
        },
        &[],
        1,
    );
    let lines = tree.new_leaf_with_context_and_block_style(
        BlockStyle::anonymous(FlowAxes::HORIZONTAL_LTR, FlowAxes::HORIZONTAL_LTR),
        Style {
            display: Display::Block,
            ..Style::default()
        },
        Vec::new(),
        2,
    );
    tree.enable_float_line_constraints(lines);
    let root = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            size: taffy::Size {
                width: Dimension::length(200.0),
                height: Dimension::auto(),
            },
            ..Style::default()
        },
        &[float, lines],
        0,
    );

    tree.compute_layout_with_measure(
        root,
        available(200.0, 200.0),
        |known, _, _, context, constraints| {
            let Some(context) = context else {
                return AlgorithmSize::new(0.0, 0.0);
            };
            if let Some(constraints) = constraints {
                *context = [0.0, 20.0, 40.0]
                    .map(|line_top| constraints.available_space(line_top, 18.0))
                    .to_vec();
            }
            AlgorithmSize::new(known.width.unwrap_or(200.0), known.height.unwrap_or(60.0))
        },
    );

    assert_eq!(
        tree.context(lines).expect("line context"),
        &[
            crate::FloatAvailableSpace {
                inline_start: 80.0,
                inline_size: 120.0,
            },
            crate::FloatAvailableSpace {
                inline_start: 80.0,
                inline_size: 120.0,
            },
            crate::FloatAvailableSpace {
                inline_start: 0.0,
                inline_size: 200.0,
            },
        ]
    );
    assert_eq!(tree.layout(lines).height, 60.0);
    assert_eq!(tree.layout(root).height, 60.0);
    assert_eq!(tree.block_algorithm(root), Some(BlockAlgorithm::Buckram));
}

#[test]
fn buckram_exports_nested_negative_margin_floats_through_a_relative_wrapper() {
    let mut tree = AlgorithmTree::<Style, (), u8>::new();
    let nested_float = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        BlockStyle {
            size: crate::BlockDimensions::new(
                BlockSizeValue::Length(crate::FlowLength::px(80.0)),
                BlockSizeValue::Length(crate::FlowLength::px(40.0)),
            ),
            float: FloatSide::Left,
            establishes_bfc: true,
            margin: PhysicalSides {
                top: FlowLengthAuto::Value(FlowLength::px(-10.0)),
                right: FlowLengthAuto::ZERO,
                bottom: FlowLengthAuto::ZERO,
                left: FlowLengthAuto::ZERO,
            },
            ..BlockStyle::default()
        },
        Style {
            size: taffy::Size {
                width: Dimension::length(80.0),
                height: Dimension::length(40.0),
            },
            ..Style::default()
        },
        &[],
        1,
    );
    let inner = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle::default(),
        Style {
            display: Display::Block,
            ..Style::default()
        },
        &[nested_float],
        2,
    );
    tree.enable_nested_float_state(inner);
    let outer = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            position: crate::BlockPosition::Relative,
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            ..Style::default()
        },
        &[inner],
        3,
    );
    tree.enable_nested_float_state(outer);
    let clear = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        BlockStyle {
            size: crate::BlockDimensions::new(
                BlockSizeValue::Auto,
                BlockSizeValue::Length(crate::FlowLength::px(10.0)),
            ),
            clear: ClearSide::Left,
            ..BlockStyle::default()
        },
        Style {
            size: taffy::Size {
                width: Dimension::auto(),
                height: Dimension::length(10.0),
            },
            ..Style::default()
        },
        &[],
        4,
    );
    let root = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            size: taffy::Size {
                width: Dimension::length(200.0),
                height: Dimension::auto(),
            },
            ..Style::default()
        },
        &[outer, clear],
        0,
    );

    tree.compute_layout_with_measure(root, available(200.0, 200.0), zero_measure);

    assert_eq!(
        (tree.layout(nested_float).x, tree.layout(nested_float).y),
        (0.0, -10.0)
    );
    assert_eq!(tree.layout(inner).height, 0.0);
    assert_eq!(tree.layout(outer).height, 0.0);
    assert_eq!(tree.layout(clear).y, 30.0);
    assert_eq!(tree.layout(root).height, 40.0);
    assert_eq!(tree.block_algorithm(inner), Some(BlockAlgorithm::Buckram));
    assert_eq!(tree.block_algorithm(outer), Some(BlockAlgorithm::Buckram));
    assert_eq!(tree.block_algorithm(root), Some(BlockAlgorithm::Buckram));
}

#[test]
fn buckram_delivers_outer_float_constraints_through_an_ordinary_wrapper() {
    let mut tree = AlgorithmTree::<Style, Vec<crate::FloatAvailableSpace>, u8>::new();
    let float = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        BlockStyle {
            size: crate::BlockDimensions::new(
                BlockSizeValue::Length(crate::FlowLength::px(80.0)),
                BlockSizeValue::Length(crate::FlowLength::px(40.0)),
            ),
            float: FloatSide::Left,
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            size: taffy::Size {
                width: Dimension::length(80.0),
                height: Dimension::length(40.0),
            },
            ..Style::default()
        },
        &[],
        1,
    );
    let lines = tree.new_leaf_with_context_and_block_style(
        BlockStyle::anonymous(FlowAxes::HORIZONTAL_LTR, FlowAxes::HORIZONTAL_LTR),
        Style {
            display: Display::Block,
            ..Style::default()
        },
        Vec::new(),
        2,
    );
    tree.enable_float_line_constraints(lines);
    let wrapper = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle::default(),
        Style {
            display: Display::Block,
            ..Style::default()
        },
        &[lines],
        3,
    );
    tree.enable_nested_float_state(wrapper);
    let root = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            size: taffy::Size {
                width: Dimension::length(200.0),
                height: Dimension::auto(),
            },
            ..Style::default()
        },
        &[float, wrapper],
        0,
    );

    tree.compute_layout_with_measure(
        root,
        available(200.0, 200.0),
        |known, _, _, context, constraints| {
            let Some(context) = context else {
                return AlgorithmSize::new(0.0, 0.0);
            };
            if let Some(constraints) = constraints {
                *context = [0.0, 20.0, 40.0]
                    .map(|line_top| constraints.available_space(line_top, 18.0))
                    .to_vec();
            }
            AlgorithmSize::new(known.width.unwrap_or(200.0), known.height.unwrap_or(60.0))
        },
    );

    assert_eq!(
        tree.context(lines).expect("line context"),
        &[
            crate::FloatAvailableSpace {
                inline_start: 80.0,
                inline_size: 120.0,
            },
            crate::FloatAvailableSpace {
                inline_start: 80.0,
                inline_size: 120.0,
            },
            crate::FloatAvailableSpace {
                inline_start: 0.0,
                inline_size: 200.0,
            },
        ]
    );
    assert_eq!(tree.layout(wrapper).height, 60.0);
    assert_eq!(tree.layout(root).height, 60.0);
    assert_eq!(tree.block_algorithm(wrapper), Some(BlockAlgorithm::Buckram));
    assert_eq!(tree.block_algorithm(root), Some(BlockAlgorithm::Buckram));
}

#[test]
fn buckram_stops_nested_float_state_at_an_explicit_bfc() {
    let mut tree = AlgorithmTree::<Style, (), u8>::new();
    let nested_float = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        BlockStyle {
            size: crate::BlockDimensions::new(
                BlockSizeValue::Length(crate::FlowLength::px(80.0)),
                BlockSizeValue::Length(crate::FlowLength::px(40.0)),
            ),
            float: FloatSide::Left,
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            size: taffy::Size {
                width: Dimension::length(80.0),
                height: Dimension::length(40.0),
            },
            ..Style::default()
        },
        &[],
        1,
    );
    let boundary = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            size: crate::BlockDimensions::new(
                BlockSizeValue::Auto,
                BlockSizeValue::Length(crate::FlowLength::ZERO),
            ),
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            size: taffy::Size {
                width: Dimension::auto(),
                height: Dimension::length(0.0),
            },
            ..Style::default()
        },
        &[nested_float],
        2,
    );
    let clear = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        BlockStyle {
            size: crate::BlockDimensions::new(
                BlockSizeValue::Auto,
                BlockSizeValue::Length(crate::FlowLength::px(10.0)),
            ),
            clear: ClearSide::Left,
            ..BlockStyle::default()
        },
        Style {
            size: taffy::Size {
                width: Dimension::auto(),
                height: Dimension::length(10.0),
            },
            ..Style::default()
        },
        &[],
        3,
    );
    let root = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            size: taffy::Size {
                width: Dimension::length(200.0),
                height: Dimension::auto(),
            },
            ..Style::default()
        },
        &[boundary, clear],
        0,
    );

    tree.compute_layout_with_measure(root, available(200.0, 200.0), zero_measure);

    assert_eq!(tree.layout(boundary).height, 0.0);
    assert_eq!(tree.layout(clear).y, 0.0);
    assert_eq!(tree.layout(root).height, 10.0);
    assert_eq!(
        tree.block_algorithm(boundary),
        Some(BlockAlgorithm::Buckram)
    );
    assert_eq!(tree.block_algorithm(root), Some(BlockAlgorithm::Buckram));
}

#[test]
fn buckram_delivers_outer_clearance_through_an_ordinary_wrapper() {
    let mut tree = AlgorithmTree::<Style, (), u8>::new();
    let float = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        BlockStyle {
            size: crate::BlockDimensions::new(
                BlockSizeValue::Length(crate::FlowLength::px(80.0)),
                BlockSizeValue::Length(crate::FlowLength::px(40.0)),
            ),
            float: FloatSide::Left,
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            size: taffy::Size {
                width: Dimension::length(80.0),
                height: Dimension::length(40.0),
            },
            ..Style::default()
        },
        &[],
        1,
    );
    let clear = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        BlockStyle {
            size: crate::BlockDimensions::new(
                BlockSizeValue::Auto,
                BlockSizeValue::Length(crate::FlowLength::px(10.0)),
            ),
            clear: ClearSide::Left,
            ..BlockStyle::default()
        },
        Style {
            size: taffy::Size {
                width: Dimension::auto(),
                height: Dimension::length(10.0),
            },
            ..Style::default()
        },
        &[],
        2,
    );
    let wrapper = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle::default(),
        Style {
            display: Display::Block,
            ..Style::default()
        },
        &[clear],
        3,
    );
    tree.enable_nested_float_state(wrapper);
    let root = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            size: taffy::Size {
                width: Dimension::length(200.0),
                height: Dimension::auto(),
            },
            ..Style::default()
        },
        &[float, wrapper],
        0,
    );

    tree.compute_layout_with_measure(root, available(200.0, 200.0), zero_measure);

    assert_eq!(tree.layout(clear).y, 40.0);
    assert_eq!(tree.layout(wrapper).height, 50.0);
    assert_eq!(tree.layout(root).height, 50.0);
    assert_eq!(tree.block_algorithm(wrapper), Some(BlockAlgorithm::Buckram));
    assert_eq!(tree.block_algorithm(root), Some(BlockAlgorithm::Buckram));
}

#[test]
fn nested_clear_without_a_shared_float_role_remains_deferred() {
    let mut tree = AlgorithmTree::<Style, (), u8>::new();
    let clear = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        BlockStyle {
            size: crate::BlockDimensions::new(
                BlockSizeValue::Auto,
                BlockSizeValue::Length(crate::FlowLength::px(10.0)),
            ),
            clear: ClearSide::Both,
            ..BlockStyle::default()
        },
        Style {
            size: taffy::Size {
                width: Dimension::auto(),
                height: Dimension::length(10.0),
            },
            ..Style::default()
        },
        &[],
        1,
    );
    let wrapper = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle::default(),
        Style {
            display: Display::Block,
            ..Style::default()
        },
        &[clear],
        2,
    );
    let root = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            size: taffy::Size {
                width: Dimension::length(200.0),
                height: Dimension::auto(),
            },
            ..Style::default()
        },
        &[wrapper],
        0,
    );

    tree.compute_layout_with_measure(root, available(200.0, 200.0), zero_measure);

    assert_eq!(tree.block_algorithm(root), Some(BlockAlgorithm::Taffy));
}

#[test]
fn nested_float_state_nonconvergence_remains_deferred() {
    let mut tree = AlgorithmTree::<Style, (), u8>::new();
    let float = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        BlockStyle {
            size: crate::BlockDimensions::new(
                BlockSizeValue::Length(crate::FlowLength::px(80.0)),
                BlockSizeValue::Length(crate::FlowLength::px(40.0)),
            ),
            float: FloatSide::Left,
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            size: taffy::Size {
                width: Dimension::length(80.0),
                height: Dimension::length(40.0),
            },
            ..Style::default()
        },
        &[],
        1,
    );
    let oscillating_leaf = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        BlockStyle {
            margin: PhysicalSides {
                top: FlowLengthAuto::ZERO,
                right: FlowLengthAuto::ZERO,
                bottom: FlowLengthAuto::Value(FlowLength::px(20.0)),
                left: FlowLengthAuto::ZERO,
            },
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            ..Style::default()
        },
        &[],
        2,
    );
    let wrapper = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle::default(),
        Style {
            display: Display::Block,
            ..Style::default()
        },
        &[oscillating_leaf],
        3,
    );
    tree.enable_nested_float_state(wrapper);
    let root = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            size: taffy::Size {
                width: Dimension::length(200.0),
                height: Dimension::auto(),
            },
            ..Style::default()
        },
        &[float, wrapper],
        0,
    );
    let calls = std::cell::Cell::new(0_u32);

    tree.compute_layout_with_measure(root, available(200.0, 200.0), |known, _, _, _, _| {
        let call = calls.get();
        calls.set(call + 1);
        AlgorithmSize::new(
            known.width.unwrap_or(200.0),
            if call.is_multiple_of(2) { 0.0 } else { 10.0 },
        )
    });

    assert!(
        calls.get() >= 6,
        "fixture must exhaust the bounded retry loop"
    );
    assert_eq!(tree.block_algorithm(root), Some(BlockAlgorithm::Taffy));
    assert_eq!(
        tree.block_deferral(root),
        Some(BlockDeferral::NestedFloatState)
    );
}

#[test]
fn buckram_keeps_inline_context_float_state_deferred() {
    let mut tree = AlgorithmTree::<Style, (), u8>::new();
    let nested_float = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        BlockStyle {
            size: crate::BlockDimensions::new(
                BlockSizeValue::Length(crate::FlowLength::px(80.0)),
                BlockSizeValue::Length(crate::FlowLength::px(40.0)),
            ),
            float: FloatSide::Left,
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            size: taffy::Size {
                width: Dimension::length(80.0),
                height: Dimension::length(40.0),
            },
            ..Style::default()
        },
        &[],
        1,
    );
    tree.mark_inline_context_float(nested_float);
    let wrapper = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle::default(),
        Style {
            display: Display::Block,
            ..Style::default()
        },
        &[nested_float],
        2,
    );
    tree.enable_nested_float_state(wrapper);
    let root = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            size: taffy::Size {
                width: Dimension::length(200.0),
                height: Dimension::auto(),
            },
            ..Style::default()
        },
        &[wrapper],
        0,
    );

    tree.compute_layout_with_measure(root, available(200.0, 200.0), zero_measure);

    assert_eq!(tree.block_algorithm(root), Some(BlockAlgorithm::Taffy));
}

#[test]
fn adapter_propagates_declared_bfc_baselines_without_backend_child_walks() {
    let mut tree = AlgorithmTree::<Style, (), u8>::new();
    let absolute = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            position: crate::BlockPosition::Absolute,
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            position: Position::Absolute,
            size: taffy::Size {
                width: Dimension::length(80.0),
                height: Dimension::length(5.0),
            },
            ..Style::default()
        },
        &[],
        3,
    );
    let flex = tree.new_with_children_and_block_style(
        AlgorithmKind::Flex,
        BlockStyle {
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            display: Display::Flex,
            size: taffy::Size {
                width: Dimension::length(80.0),
                height: Dimension::length(20.0),
            },
            ..Style::default()
        },
        &[],
        1,
    );
    let grid = tree.new_with_children_and_block_style(
        AlgorithmKind::Grid,
        BlockStyle {
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            display: Display::Grid,
            size: taffy::Size {
                width: Dimension::length(80.0),
                height: Dimension::length(30.0),
            },
            ..Style::default()
        },
        &[],
        2,
    );
    let fixed = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            position: crate::BlockPosition::Fixed,
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            position: Position::Absolute,
            size: taffy::Size {
                width: Dimension::length(80.0),
                height: Dimension::length(5.0),
            },
            ..Style::default()
        },
        &[],
        4,
    );
    let root = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            size: taffy::Size {
                width: Dimension::length(80.0),
                height: Dimension::auto(),
            },
            ..Style::default()
        },
        &[absolute, flex, grid, fixed],
        0,
    );

    tree.compute_layout_with_measure(root, available(80.0, 200.0), zero_measure);
    tree.set_baselines(
        flex,
        Baselines::new(Some(7.0), Some(9.0)).expect("flex baselines"),
    );
    tree.set_baselines(
        grid,
        Baselines::new(Some(11.0), Some(13.0)).expect("grid baselines"),
    );
    tree.set_baselines(
        absolute,
        Baselines::new(Some(1.0), Some(2.0)).expect("absolute baselines"),
    );
    tree.set_baselines(
        fixed,
        Baselines::new(Some(40.0), Some(50.0)).expect("fixed baselines"),
    );
    tree.propagate_declared_baselines();

    assert_eq!(
        tree.baselines(flex),
        Baselines::new(Some(7.0), Some(9.0)).unwrap()
    );
    assert_eq!(
        tree.baselines(grid),
        Baselines::new(Some(11.0), Some(13.0)).unwrap()
    );
    assert_eq!(
        tree.baselines(root),
        Baselines::new(
            Some(tree.layout(flex).y + 7.0),
            Some(tree.layout(grid).y + 13.0),
        )
        .unwrap()
    );
}

/// K4d6b's seam: a formatting context Buckram owns writes its own and its
/// children's rectangles before the backend walk, and the walk must not
/// overwrite them. A `Table` node reports the size it was given and never
/// lays out a child.
#[test]
fn an_owned_table_context_keeps_the_geometry_it_was_given() {
    let mut tree: AlgorithmTree<Style, (), u8> = AlgorithmTree::new();
    let cells = (0..2)
        .map(|index| {
            tree.new_with_children_and_block_style(
                AlgorithmKind::Block,
                BlockStyle::default(),
                Style::default(),
                &[],
                index,
            )
        })
        .collect::<Vec<_>>();
    let table = tree.new_with_children_and_block_style(
        AlgorithmKind::Table,
        BlockStyle {
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style::default(),
        &cells,
        9,
    );
    let root = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            size: taffy::Size {
                width: Dimension::length(200.0),
                height: Dimension::length(200.0),
            },
            ..Style::default()
        },
        &[table],
        10,
    );

    // The table algorithm's decisions, written before the walk.
    let decided = [
        AlgorithmLayout {
            x: 0.0,
            y: 0.0,
            width: 60.0,
            height: 25.0,
        },
        AlgorithmLayout {
            x: 60.0,
            y: 0.0,
            width: 40.0,
            height: 25.0,
        },
    ];
    for (cell, layout) in cells.iter().zip(decided) {
        tree.set_layout(*cell, layout);
    }
    tree.set_layout(
        table,
        AlgorithmLayout {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 25.0,
        },
    );

    tree.compute_layout_with_measure(root, available(200.0, 200.0), zero_measure);

    // The table reported Buckram's size, and every cell rectangle
    // survived the walk untouched.
    let table_layout = tree.layout(table);
    assert_eq!((table_layout.width, table_layout.height), (100.0, 25.0));
    for (cell, expected) in cells.iter().zip(decided) {
        let layout = tree.layout(*cell);
        assert_eq!(
            (layout.x, layout.y, layout.width, layout.height),
            (expected.x, expected.y, expected.width, expected.height),
            "the backend overwrote a rectangle the table algorithm owns"
        );
    }
}

#[test]
fn horizontal_buckram_block_admits_vertical_flex_auto_block_size() {
    use crate::{Direction, WritingMode};

    let horizontal = FlowAxes::HORIZONTAL_LTR;
    let vertical = FlowAxes::new(WritingMode::VerticalRl, Direction::Ltr);
    for (css_direction, direction, expected) in [
        (
            "row",
            taffy::FlexDirection::Column,
            [(2.0, 2.0), (2.0, 47.0), (17.0, 2.0), (17.0, 47.0)],
        ),
        (
            "row-reverse",
            taffy::FlexDirection::ColumnReverse,
            [(2.0, 47.0), (2.0, 2.0), (17.0, 47.0), (17.0, 2.0)],
        ),
    ] {
        let mut tree = AlgorithmTree::<Style, (), u8>::new();
        let children = (1..=4)
            .map(|source| {
                tree.new_with_children_and_block_style(
                    AlgorithmKind::Leaf,
                    BlockStyle::anonymous(vertical, vertical),
                    Style {
                        size: taffy::Size {
                            width: Dimension::length(15.0),
                            height: Dimension::length(45.0),
                        },
                        flex_basis: Dimension::length(45.0).into(),
                        ..Style::default()
                    },
                    &[],
                    source,
                )
            })
            .collect::<Vec<_>>();
        let flex = tree.new_with_children_and_block_style(
            AlgorithmKind::Flex,
            BlockStyle {
                flow: vertical,
                containing_flow: horizontal,
                size: crate::BlockDimensions::new(
                    BlockSizeValue::Auto,
                    BlockSizeValue::Length(FlowLength::px(90.0)),
                ),
                border: PhysicalSides::splat(2.0),
                establishes_bfc: true,
                ..BlockStyle::default()
            },
            Style {
                display: Display::Flex,
                box_sizing: taffy::BoxSizing::ContentBox,
                size: taffy::Size {
                    width: Dimension::auto(),
                    height: Dimension::length(90.0),
                },
                border: Rect::length(2.0),
                flex_direction: direction,
                direction: taffy::Direction::Rtl,
                flex_wrap: taffy::FlexWrap::WrapReverse,
                align_items: Some(AlignItems::START),
                ..Style::default()
            },
            &children,
            5,
        );
        let root = tree.new_with_children_and_block_style(
            AlgorithmKind::Block,
            BlockStyle {
                flow: horizontal,
                containing_flow: horizontal,
                size: crate::BlockDimensions::new(
                    BlockSizeValue::Length(FlowLength::px(320.0)),
                    BlockSizeValue::Auto,
                ),
                establishes_bfc: true,
                ..BlockStyle::default()
            },
            Style {
                display: Display::Block,
                size: taffy::Size {
                    width: Dimension::length(320.0),
                    height: Dimension::auto(),
                },
                ..Style::default()
            },
            &[flex],
            0,
        );

        tree.compute_layout_with_measure(root, available(320.0, 240.0), zero_measure);

        assert_eq!(tree.block_algorithm(root), Some(BlockAlgorithm::Buckram));
        assert_eq!(tree.kind(flex), AlgorithmKind::Flex);
        assert_eq!(
            tree.layout(flex),
            AlgorithmLayout {
                width: 34.0,
                height: 94.0,
                ..AlgorithmLayout::default()
            },
            "{css_direction} flex border box"
        );
        for (child, (x, y)) in children.into_iter().zip(expected) {
            assert_eq!(
                tree.layout(child),
                AlgorithmLayout {
                    x,
                    y,
                    width: 15.0,
                    height: 45.0,
                },
                "{css_direction} child"
            );
        }
    }
}

#[test]
fn vertical_flex_auto_block_admission_rejects_physical_row_directions() {
    use crate::{Direction, WritingMode};

    let horizontal = FlowAxes::HORIZONTAL_LTR;
    let vertical = FlowAxes::new(WritingMode::VerticalRl, Direction::Ltr);
    let parent = BlockStyle {
        flow: horizontal,
        containing_flow: horizontal,
        establishes_bfc: true,
        ..BlockStyle::default()
    };
    let child = BlockStyle {
        flow: vertical,
        containing_flow: horizontal,
        size: crate::BlockDimensions::new(
            BlockSizeValue::Auto,
            BlockSizeValue::Length(FlowLength::px(90.0)),
        ),
        establishes_bfc: true,
        ..BlockStyle::default()
    };

    assert!(admits_orthogonal_auto_block_flex_child(
        parent,
        AlgorithmKind::Flex,
        child,
        taffy::FlexDirection::Column,
    ));
    assert!(!admits_orthogonal_auto_block_flex_child(
        parent,
        AlgorithmKind::Flex,
        child,
        taffy::FlexDirection::Row,
    ));
    assert!(!admits_orthogonal_auto_block_flex_child(
        parent,
        AlgorithmKind::Flex,
        BlockStyle {
            containing_flow: vertical,
            ..child
        },
        taffy::FlexDirection::Column,
    ));
}

#[test]
fn orthogonal_auto_inline_uses_intrinsic_child_block_contribution() {
    use crate::{Direction, WritingMode};

    let horizontal = FlowAxes::HORIZONTAL_LTR;
    let vertical = FlowAxes::new(WritingMode::VerticalRl, Direction::Ltr);
    let mut tree = AlgorithmTree::<Style, (), u8>::new();
    let text = tree.new_leaf_with_context_and_block_style(
        BlockStyle::anonymous(horizontal, horizontal),
        Style::default(),
        (),
        2,
    );
    let line = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle::anonymous(horizontal, vertical),
        Style {
            display: Display::Block,
            ..Style::default()
        },
        &[text],
        1,
    );
    let root = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            flow: vertical,
            containing_flow: horizontal,
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            ..Style::default()
        },
        &[line],
        0,
    );

    tree.compute_layout_with_measure(
        root,
        available(300.0, 200.0),
        |known, available, node, _context, _line_constraints| {
            assert_eq!(node, text);
            let width = known.width.unwrap_or_else(|| match available.width {
                AlgorithmAvailableSpace::Definite(width) => 120.0_f32.min(width),
                AlgorithmAvailableSpace::MinContent => 40.0,
                AlgorithmAvailableSpace::MaxContent => 120.0,
            });
            AlgorithmSize::new(width, known.height.unwrap_or(20.0))
        },
    );

    assert_eq!(tree.block_algorithm(root), Some(BlockAlgorithm::Buckram));
    assert_eq!(tree.block_algorithm(line), Some(BlockAlgorithm::Buckram));
    assert_eq!(
        tree.layout(root),
        AlgorithmLayout {
            width: 120.0,
            height: 20.0,
            ..AlgorithmLayout::default()
        }
    );
    assert_eq!(
        tree.layout(line),
        AlgorithmLayout {
            width: 120.0,
            height: 20.0,
            ..AlgorithmLayout::default()
        }
    );
}

#[test]
fn orthogonal_auto_inline_rejects_a_nested_flow_boundary() {
    use crate::{Direction, WritingMode};

    let horizontal = FlowAxes::HORIZONTAL_LTR;
    let vertical = FlowAxes::new(WritingMode::VerticalRl, Direction::Ltr);
    let mut tree = AlgorithmTree::<Style, (), u8>::new();
    let text = tree.new_leaf_with_context_and_block_style(
        BlockStyle::anonymous(horizontal, horizontal),
        Style::default(),
        (),
        3,
    );
    let line = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle::anonymous(horizontal, vertical),
        Style {
            display: Display::Block,
            ..Style::default()
        },
        &[text],
        2,
    );
    let same_flow_wrapper = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle::anonymous(vertical, vertical),
        Style {
            display: Display::Block,
            ..Style::default()
        },
        &[line],
        1,
    );
    let root = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            flow: vertical,
            containing_flow: horizontal,
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            ..Style::default()
        },
        &[same_flow_wrapper],
        0,
    );

    tree.compute_layout_with_measure(
        root,
        available(300.0, 200.0),
        |known, available, node, _context, _line_constraints| {
            assert_eq!(node, text);
            let width = known.width.unwrap_or_else(|| match available.width {
                AlgorithmAvailableSpace::Definite(width) => 120.0_f32.min(width),
                AlgorithmAvailableSpace::MinContent => 40.0,
                AlgorithmAvailableSpace::MaxContent => 120.0,
            });
            AlgorithmSize::new(width, known.height.unwrap_or(20.0))
        },
    );

    assert_eq!(tree.block_algorithm(root), Some(BlockAlgorithm::Buckram));
    assert_eq!(tree.layout(root).height, 200.0);
}

#[test]
fn orthogonal_auto_block_sizes_finalize_before_physical_child_placement() {
    use crate::{Direction, WritingMode};

    for (writing_mode, first_x, second_x) in [
        (WritingMode::VerticalRl, 30.0, 0.0),
        (WritingMode::VerticalLr, 0.0, 20.0),
        (WritingMode::SidewaysRl, 30.0, 0.0),
        (WritingMode::SidewaysLr, 0.0, 20.0),
    ] {
        let flow = FlowAxes::new(writing_mode, Direction::Ltr);
        let mut tree = AlgorithmTree::<Style, (), u8>::new();
        let first = tree.new_with_children_and_block_style(
            AlgorithmKind::Leaf,
            BlockStyle::anonymous(flow, flow),
            Style::default(),
            &[],
            1,
        );
        let second = tree.new_with_children_and_block_style(
            AlgorithmKind::Leaf,
            BlockStyle::anonymous(flow, flow),
            Style::default(),
            &[],
            2,
        );
        let root = tree.new_with_children_and_block_style(
            AlgorithmKind::Block,
            BlockStyle {
                flow,
                containing_flow: flow,
                establishes_bfc: true,
                ..BlockStyle::default()
            },
            Style {
                display: Display::Block,
                ..Style::default()
            },
            &[first, second],
            0,
        );

        tree.compute_layout_with_measure(
            root,
            available(300.0, 200.0),
            |known, _available, node, _context, _line_constraints| {
                let block = if node == first { 20.0 } else { 30.0 };
                AlgorithmSize::new(block, known.height.unwrap_or(200.0))
            },
        );

        assert_eq!(tree.block_algorithm(root), Some(BlockAlgorithm::Buckram));
        assert_eq!(
            tree.layout(root),
            AlgorithmLayout {
                width: 50.0,
                height: 200.0,
                ..AlgorithmLayout::default()
            },
            "{writing_mode:?} root size"
        );
        assert_eq!(
            tree.layout(first),
            AlgorithmLayout {
                x: first_x,
                width: 20.0,
                height: 200.0,
                ..AlgorithmLayout::default()
            },
            "{writing_mode:?} first child"
        );
        assert_eq!(
            tree.layout(second),
            AlgorithmLayout {
                x: second_x,
                width: 30.0,
                height: 200.0,
                ..AlgorithmLayout::default()
            },
            "{writing_mode:?} second child"
        );
    }
}

#[test]
fn adapter_nests_horizontal_and_vertical_normal_flow_in_both_directions() {
    use crate::{Direction, WritingMode};

    fn layout_nested(parent_flow: FlowAxes, child_flow: FlowAxes) -> AlgorithmLayout {
        let mut tree = AlgorithmTree::<Style, (), u8>::new();
        let leaf = tree.new_with_children_and_block_style(
            AlgorithmKind::Leaf,
            BlockStyle::anonymous(child_flow, child_flow),
            Style::default(),
            &[],
            2,
        );
        let child = tree.new_with_children_and_block_style(
            AlgorithmKind::Block,
            BlockStyle {
                flow: child_flow,
                containing_flow: parent_flow,
                size: if child_flow.is_horizontal() {
                    crate::BlockDimensions::new(
                        BlockSizeValue::Length(FlowLength::px(80.0)),
                        BlockSizeValue::Auto,
                    )
                } else {
                    crate::BlockDimensions::new(
                        BlockSizeValue::Auto,
                        BlockSizeValue::Length(FlowLength::px(80.0)),
                    )
                },
                ..BlockStyle::default()
            },
            Style {
                display: Display::Block,
                ..Style::default()
            },
            &[leaf],
            1,
        );
        let root = tree.new_with_children_and_block_style(
            AlgorithmKind::Block,
            BlockStyle {
                flow: parent_flow,
                containing_flow: parent_flow,
                size: if parent_flow.is_horizontal() {
                    crate::BlockDimensions::new(
                        BlockSizeValue::Length(FlowLength::px(200.0)),
                        BlockSizeValue::Auto,
                    )
                } else {
                    crate::BlockDimensions::new(
                        BlockSizeValue::Auto,
                        BlockSizeValue::Length(FlowLength::px(200.0)),
                    )
                },
                establishes_bfc: true,
                ..BlockStyle::default()
            },
            Style {
                display: Display::Block,
                ..Style::default()
            },
            &[child],
            0,
        );

        tree.compute_layout_with_measure(
            root,
            available(200.0, 200.0),
            |known, _available, _node, _context, _line_constraints| {
                AlgorithmSize::new(known.width.unwrap_or(20.0), known.height.unwrap_or(20.0))
            },
        );

        assert_eq!(tree.block_algorithm(root), Some(BlockAlgorithm::Buckram));
        assert_eq!(tree.block_algorithm(child), Some(BlockAlgorithm::Buckram));
        tree.layout(child)
    }

    let horizontal_parent = FlowAxes::HORIZONTAL_LTR;
    let vertical_child = FlowAxes::new(WritingMode::VerticalRl, Direction::Ltr);
    assert_eq!(
        layout_nested(horizontal_parent, vertical_child),
        AlgorithmLayout {
            width: 20.0,
            height: 80.0,
            ..AlgorithmLayout::default()
        }
    );

    let vertical_parent = FlowAxes::new(WritingMode::VerticalLr, Direction::Ltr);
    let horizontal_child = FlowAxes::HORIZONTAL_LTR;
    assert_eq!(
        layout_nested(vertical_parent, horizontal_child),
        AlgorithmLayout {
            width: 80.0,
            height: 20.0,
            ..AlgorithmLayout::default()
        }
    );
}

#[test]
fn rtl_block_auto_inline_size_accounts_for_physical_margins() {
    use crate::{Direction, WritingMode};

    let rtl = FlowAxes::new(WritingMode::HorizontalTb, Direction::Rtl);
    let mut tree = AlgorithmTree::<Style, (), &str>::new();
    let body = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            flow: rtl,
            containing_flow: rtl,
            margin: PhysicalSides {
                top: crate::FlowLengthAuto::Value(crate::FlowLength::px(8.0)),
                right: crate::FlowLengthAuto::Value(crate::FlowLength::px(8.0)),
                bottom: crate::FlowLengthAuto::Value(crate::FlowLength::px(8.0)),
                left: crate::FlowLengthAuto::Value(crate::FlowLength::px(8.0)),
            },
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            ..Style::default()
        },
        &[],
        "body",
    );
    let html = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            flow: rtl,
            containing_flow: rtl,
            size: crate::BlockDimensions::new(
                BlockSizeValue::Length(FlowLength::px(800.0)),
                BlockSizeValue::Auto,
            ),
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            ..Style::default()
        },
        &[body],
        "html",
    );

    tree.compute_layout_with_measure(html, available(800.0, 600.0), zero_measure);

    assert_eq!(tree.block_algorithm(html), Some(BlockAlgorithm::Buckram));
    assert_eq!(tree.block_algorithm(body), Some(BlockAlgorithm::Buckram));
    let body_layout = tree.layout(body);
    assert_eq!(body_layout.x, 8.0);
    assert_eq!(body_layout.width, 784.0);
}

#[test]
fn orthogonal_percentage_width_uses_available_physical_fallback() {
    use crate::{Direction, WritingMode};

    let vertical = FlowAxes::new(WritingMode::VerticalLr, Direction::Ltr);
    let horizontal = FlowAxes::HORIZONTAL_LTR;
    let mut tree = AlgorithmTree::<Style, (), u8>::new();
    let text = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        BlockStyle::anonymous(horizontal, horizontal),
        Style::default(),
        &[],
        2,
    );
    let child = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            flow: horizontal,
            containing_flow: vertical,
            size: crate::BlockDimensions::new(
                BlockSizeValue::Length(FlowLength::percent(0.5)),
                BlockSizeValue::Auto,
            ),
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            ..Style::default()
        },
        &[text],
        1,
    );
    let root = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            flow: vertical,
            containing_flow: vertical,
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            ..Style::default()
        },
        &[child],
        0,
    );

    tree.compute_layout_with_measure(
        root,
        available(300.0, 200.0),
        |known, _available, _node, _context, _line_constraints| {
            AlgorithmSize::new(
                known.width.unwrap_or_default(),
                known.height.unwrap_or(40.0),
            )
        },
    );

    assert_eq!(tree.block_algorithm(root), Some(BlockAlgorithm::Buckram));
    assert_eq!(tree.block_algorithm(child), Some(BlockAlgorithm::Buckram));
    assert_eq!(
        tree.layout(child),
        AlgorithmLayout {
            width: 150.0,
            height: 40.0,
            ..AlgorithmLayout::default()
        }
    );
}

#[test]
fn orthogonal_float_continuation_stays_deferred_without_a_logical_transform() {
    use crate::{Direction, WritingMode};

    let horizontal = FlowAxes::HORIZONTAL_LTR;
    let vertical = FlowAxes::new(WritingMode::VerticalRl, Direction::Ltr);
    let mut tree = AlgorithmTree::<Style, (), u8>::new();
    let orthogonal_float = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        BlockStyle {
            flow: vertical,
            containing_flow: horizontal,
            float: FloatSide::Left,
            size: crate::BlockDimensions::new(
                BlockSizeValue::Length(FlowLength::px(40.0)),
                BlockSizeValue::Length(FlowLength::px(80.0)),
            ),
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            size: taffy::Size {
                width: Dimension::length(40.0),
                height: Dimension::length(80.0),
            },
            ..Style::default()
        },
        &[],
        1,
    );
    let root = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            size: taffy::Size {
                width: Dimension::length(200.0),
                height: Dimension::auto(),
            },
            ..Style::default()
        },
        &[orthogonal_float],
        0,
    );

    tree.compute_layout_with_measure(root, available(200.0, 200.0), zero_measure);

    assert_eq!(tree.block_algorithm(root), Some(BlockAlgorithm::Taffy));
    assert_eq!(
        tree.block_deferral(root),
        Some(BlockDeferral::NestedFloatState)
    );
}

#[test]
fn taffy_block_fallback_retains_the_css_facing_deferral() {
    let mut tree = AlgorithmTree::<Style, (), u8>::new();
    let child = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle::default(),
        Style {
            display: Display::Block,
            size: taffy::Size {
                width: Dimension::auto(),
                height: Dimension::length(20.0),
            },
            ..Style::default()
        },
        &[],
        1,
    );
    let root = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            replaced: true,
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            size: taffy::Size {
                width: Dimension::length(200.0),
                height: Dimension::auto(),
            },
            ..Style::default()
        },
        &[child],
        0,
    );

    tree.compute_layout_with_measure(root, available(200.0, 200.0), zero_measure);

    assert_eq!(tree.block_algorithm(root), Some(BlockAlgorithm::Taffy));
    assert_eq!(tree.block_deferral(root), Some(BlockDeferral::Replaced));
    assert_eq!(tree.block_algorithm(child), Some(BlockAlgorithm::Taffy));
    assert_eq!(
        tree.block_deferral(child),
        Some(BlockDeferral::BackendSizingMode)
    );
}

fn leaf_with_height(
    tree: &mut AlgorithmTree<Style, (), u8>,
    height: f32,
    source: u8,
) -> AlgorithmNodeId {
    tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        BlockStyle {
            size: crate::BlockDimensions::new(
                BlockSizeValue::Auto,
                BlockSizeValue::Length(crate::FlowLength::px(height)),
            ),
            ..BlockStyle::default()
        },
        Style {
            size: taffy::Size {
                width: Dimension::auto(),
                height: Dimension::length(height),
            },
            ..Style::default()
        },
        &[],
        source,
    )
}

fn bfc_root_with_children(
    tree: &mut AlgorithmTree<Style, (), u8>,
    width: f32,
    children: &[AlgorithmNodeId],
) -> AlgorithmNodeId {
    tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            size: taffy::Size {
                width: Dimension::length(width),
                height: Dimension::auto(),
            },
            ..Style::default()
        },
        children,
        0,
    )
}

/// Lane 8 (2026-08-21): a table wrapper box is an opaque block child of
/// the owned block formatter. Its `auto` inline size is the grid's
/// border-edge inline size (CSS Tables 3 section 2.2.1), so `margin: auto`
/// centers it by that width, and neither it nor its parent falls back to
/// Taffy because the grid establishes its own formatting context.
#[test]
fn buckram_admits_a_table_wrapper_as_an_opaque_block_sized_by_its_grid() {
    let mut tree: AlgorithmTree<Style, (), u8> = AlgorithmTree::new();
    let cells = (0..2)
        .map(|index| {
            tree.new_with_children_and_block_style(
                AlgorithmKind::Block,
                BlockStyle::default(),
                Style::default(),
                &[],
                index,
            )
        })
        .collect::<Vec<_>>();
    let grid = tree.new_with_children_and_block_style(
        AlgorithmKind::Table,
        BlockStyle {
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style::default(),
        &cells,
        9,
    );
    // The wrapper carries the table's margins (CSS 2.1 section 17.4).
    let wrapper = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            establishes_bfc: true,
            margin: PhysicalSides {
                top: FlowLengthAuto::ZERO,
                right: FlowLengthAuto::Auto,
                bottom: FlowLengthAuto::ZERO,
                left: FlowLengthAuto::Auto,
            },
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            margin: Rect {
                left: taffy::LengthPercentageAuto::auto(),
                right: taffy::LengthPercentageAuto::auto(),
                top: taffy::LengthPercentageAuto::length(0.0),
                bottom: taffy::LengthPercentageAuto::length(0.0),
            },
            ..Style::default()
        },
        &[grid],
        10,
    );
    let after = leaf_with_height(&mut tree, 10.0, 11);
    let root = bfc_root_with_children(&mut tree, 200.0, &[wrapper, after]);
    let decided = [
        AlgorithmLayout {
            x: 0.0,
            y: 0.0,
            width: 60.0,
            height: 25.0,
        },
        AlgorithmLayout {
            x: 60.0,
            y: 0.0,
            width: 40.0,
            height: 25.0,
        },
    ];
    for (cell, layout) in cells.iter().zip(decided) {
        tree.set_layout(*cell, layout);
    }
    tree.set_layout(
        grid,
        AlgorithmLayout {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 25.0,
        },
    );
    tree.set_table_wrapper_inline_size(wrapper, 100.0);

    tree.compute_layout_with_measure(root, available(200.0, 200.0), zero_measure);

    assert_eq!(tree.block_algorithm(root), Some(BlockAlgorithm::Buckram));
    assert_eq!(tree.block_algorithm(wrapper), Some(BlockAlgorithm::Buckram));
    assert_eq!(
        tree.layout(wrapper),
        AlgorithmLayout {
            x: 50.0,
            y: 0.0,
            width: 100.0,
            height: 25.0,
        },
        "the wrapper is as wide as its grid and centered by its auto margins"
    );
    assert_eq!(
        tree.layout(grid),
        AlgorithmLayout {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 25.0,
        }
    );
    for (cell, expected) in cells.iter().zip(decided) {
        assert_eq!(tree.layout(*cell), expected);
    }
    assert_eq!(tree.layout(after).y, 25.0);
    assert_eq!(tree.layout(root).height, 35.0);
}

/// A flow-root whose own subtree defers (a replaced child) is laid out by
/// Taffy on its own, and its parent places it with margins modeled from
/// its style: 30px below the previous sibling's margin, not deferred.
#[test]
fn a_deferred_flow_root_child_does_not_defer_its_buckram_parent() {
    let mut tree = AlgorithmTree::<Style, (), u8>::new();
    let replaced = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        BlockStyle {
            replaced: true,
            size: crate::BlockDimensions::new(
                BlockSizeValue::Length(crate::FlowLength::px(50.0)),
                BlockSizeValue::Length(crate::FlowLength::px(30.0)),
            ),
            ..BlockStyle::default()
        },
        Style {
            size: taffy::Size {
                width: Dimension::length(50.0),
                height: Dimension::length(30.0),
            },
            ..Style::default()
        },
        &[],
        1,
    );
    let flow_root = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            establishes_bfc: true,
            margin: PhysicalSides {
                top: FlowLengthAuto::Value(crate::FlowLength::px(10.0)),
                right: FlowLengthAuto::ZERO,
                bottom: FlowLengthAuto::Value(crate::FlowLength::px(20.0)),
                left: FlowLengthAuto::ZERO,
            },
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            margin: Rect {
                left: taffy::LengthPercentageAuto::length(0.0),
                right: taffy::LengthPercentageAuto::length(0.0),
                top: taffy::LengthPercentageAuto::length(10.0),
                bottom: taffy::LengthPercentageAuto::length(20.0),
            },
            ..Style::default()
        },
        &[replaced],
        2,
    );
    let before = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        BlockStyle {
            size: crate::BlockDimensions::new(
                BlockSizeValue::Auto,
                BlockSizeValue::Length(crate::FlowLength::px(40.0)),
            ),
            margin: PhysicalSides {
                top: FlowLengthAuto::ZERO,
                right: FlowLengthAuto::ZERO,
                bottom: FlowLengthAuto::Value(crate::FlowLength::px(30.0)),
                left: FlowLengthAuto::ZERO,
            },
            ..BlockStyle::default()
        },
        Style {
            size: taffy::Size {
                width: Dimension::auto(),
                height: Dimension::length(40.0),
            },
            margin: Rect {
                left: taffy::LengthPercentageAuto::length(0.0),
                right: taffy::LengthPercentageAuto::length(0.0),
                top: taffy::LengthPercentageAuto::length(0.0),
                bottom: taffy::LengthPercentageAuto::length(30.0),
            },
            ..Style::default()
        },
        &[],
        3,
    );
    let after = leaf_with_height(&mut tree, 10.0, 4);
    let root = bfc_root_with_children(&mut tree, 200.0, &[before, flow_root, after]);

    tree.compute_layout_with_measure(root, available(200.0, 200.0), zero_measure);

    assert_eq!(tree.block_algorithm(root), Some(BlockAlgorithm::Buckram));
    assert_eq!(tree.block_algorithm(flow_root), Some(BlockAlgorithm::Taffy));
    assert_eq!(
        tree.block_deferral(flow_root),
        Some(BlockDeferral::Replaced)
    );
    assert_eq!(
        tree.layout(flow_root),
        AlgorithmLayout {
            x: 0.0,
            y: 70.0,
            width: 200.0,
            height: 30.0,
        },
        "30px bottom margin and 10px top margin collapse to 30px below a 40px sibling"
    );
    assert_eq!(tree.layout(after).y, 120.0);
    assert_eq!(tree.layout(root).height, 130.0);
}

/// CSS 2.1 section 9.5 still applies: beside an active float, a BFC child
/// without the avoidance opt-in defers with the float deferral, not with a
/// formatting-context deferral, and the parent falls back as before.
#[test]
fn a_table_beside_an_active_float_still_defers_to_float_avoidance() {
    let mut tree = AlgorithmTree::<Style, (), u8>::new();
    let float = tree.new_with_children_and_block_style(
        AlgorithmKind::Leaf,
        BlockStyle {
            size: crate::BlockDimensions::new(
                BlockSizeValue::Length(crate::FlowLength::px(80.0)),
                BlockSizeValue::Length(crate::FlowLength::px(40.0)),
            ),
            float: FloatSide::Left,
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style {
            size: taffy::Size {
                width: Dimension::length(80.0),
                height: Dimension::length(40.0),
            },
            ..Style::default()
        },
        &[],
        1,
    );
    let grid = tree.new_with_children_and_block_style(
        AlgorithmKind::Table,
        BlockStyle {
            establishes_bfc: true,
            ..BlockStyle::default()
        },
        Style::default(),
        &[],
        2,
    );
    tree.set_layout(
        grid,
        AlgorithmLayout {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 25.0,
        },
    );
    let root = bfc_root_with_children(&mut tree, 200.0, &[float, grid]);

    tree.compute_layout_with_measure(root, available(200.0, 200.0), zero_measure);

    assert_eq!(tree.block_algorithm(root), Some(BlockAlgorithm::Taffy));
    assert_eq!(
        tree.block_deferral(root),
        Some(BlockDeferral::FloatFormattingContextAvoidance)
    );
}

/// A relatively positioned scroll container establishes a BFC but is not
/// a float-avoidance candidate in Livery; without an active float it is an
/// ordinary opaque block child.
#[test]
fn a_relatively_positioned_scroll_container_is_an_opaque_block_child() {
    let mut tree = AlgorithmTree::<Style, (), u8>::new();
    let inner = leaf_with_height(&mut tree, 10.0, 1);
    let scroll = tree.new_with_children_and_block_style(
        AlgorithmKind::Block,
        BlockStyle {
            establishes_bfc: true,
            position: crate::BlockPosition::Relative,
            margin: PhysicalSides {
                top: FlowLengthAuto::Value(crate::FlowLength::px(10.0)),
                right: FlowLengthAuto::ZERO,
                bottom: FlowLengthAuto::ZERO,
                left: FlowLengthAuto::ZERO,
            },
            ..BlockStyle::default()
        },
        Style {
            display: Display::Block,
            margin: Rect {
                left: taffy::LengthPercentageAuto::length(0.0),
                right: taffy::LengthPercentageAuto::length(0.0),
                top: taffy::LengthPercentageAuto::length(10.0),
                bottom: taffy::LengthPercentageAuto::length(0.0),
            },
            ..Style::default()
        },
        &[inner],
        2,
    );
    let root = bfc_root_with_children(&mut tree, 200.0, &[scroll]);

    tree.compute_layout_with_measure(root, available(200.0, 200.0), zero_measure);

    assert_eq!(tree.block_algorithm(root), Some(BlockAlgorithm::Buckram));
    assert_eq!(tree.block_algorithm(scroll), Some(BlockAlgorithm::Buckram));
    assert_eq!(
        tree.layout(scroll),
        AlgorithmLayout {
            x: 0.0,
            y: 10.0,
            width: 200.0,
            height: 10.0,
        }
    );
    assert_eq!(tree.layout(root).height, 20.0);
}
